//! PDF text extraction — Rust replacement for pipeline/crawl/pdf_parser.py (archived R9).
//!
//! Uses `lopdf` (pure Rust) for PDF parsing.
//!
//! Table extraction is heuristic (text-layout based), not spatial like pdfplumber.
//! For complex merged-cell tables, prefer analytics_svc.

use anyhow::Result;
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfTable {
    pub page: u32,
    pub data: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfExtractResult {
    pub content_md: String,
    pub content_hash: String,
    pub tables: Vec<PdfTable>,
    pub page_count: usize,
}

fn is_separator_cell(cell: &str) -> bool {
    let t = cell.trim();
    !t.is_empty() && t.chars().all(|c| c == '-' || c == ':' || c == '=')
}

fn parse_table_row(line: &str, multi_space_re: &Regex) -> Option<Vec<String>> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }

    if t.contains('|') {
        let cells: Vec<String> = t
            .split('|')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if cells.len() >= 2 && !cells.iter().all(|c| is_separator_cell(c)) {
            return Some(cells);
        }
        return None;
    }

    if t.contains('\t') {
        let cells: Vec<String> = t
            .split('\t')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if cells.len() >= 2 {
            return Some(cells);
        }
    }

    let cells: Vec<String> = multi_space_re
        .split(t)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if cells.len() >= 2 {
        return Some(cells);
    }

    None
}

fn infer_tables_from_pages(page_entries: &[(u32, String)]) -> Vec<PdfTable> {
    let multi_space_re = Regex::new(r"\s{2,}").expect("regex literal must compile");
    let mut tables: Vec<PdfTable> = Vec::new();

    for (page, text) in page_entries {
        let mut current_rows: Vec<Vec<String>> = Vec::new();
        let mut expected_cols: Option<usize> = None;

        let mut flush = |rows: &mut Vec<Vec<String>>, cols: &mut Option<usize>| {
            if rows.len() >= 2 {
                tables.push(PdfTable {
                    page: *page,
                    data: std::mem::take(rows),
                });
            } else {
                rows.clear();
            }
            *cols = None;
        };

        for line in text.lines() {
            match parse_table_row(line, &multi_space_re) {
                Some(row) => {
                    let cols = row.len();
                    match expected_cols {
                        None => {
                            expected_cols = Some(cols);
                            current_rows.push(row);
                        }
                        Some(c) if c == cols => {
                            current_rows.push(row);
                        }
                        Some(_) => {
                            flush(&mut current_rows, &mut expected_cols);
                            expected_cols = Some(cols);
                            current_rows.push(row);
                        }
                    }
                }
                None => {
                    flush(&mut current_rows, &mut expected_cols);
                }
            }
        }
        flush(&mut current_rows, &mut expected_cols);
    }

    tables
}

/// Extracts text content and metadata from a PDF file.
///
/// Errors are returned via `Result`, no JSON error envelope in Rust core.
pub fn extract_pdf(file_path: &str) -> Result<PdfExtractResult> {
    let doc = lopdf::Document::load(file_path)?;
    let pages = doc.get_pages();
    let page_count = pages.len();

    let mut page_entries: Vec<(u32, String)> = Vec::with_capacity(page_count);
    let mut page_numbers: Vec<u32> = pages.keys().copied().collect();
    page_numbers.sort_unstable();

    for page_num in page_numbers {
        match doc.extract_text(&[page_num]) {
            Ok(text) => {
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    page_entries.push((page_num, trimmed));
                }
            }
            Err(_) => {} // skip unreadable pages (scanned images, encrypted)
        }
    }

    let page_texts: Vec<String> = page_entries.iter().map(|(_, t)| t.clone()).collect();
    let content_md = page_texts.join("\n\n");
    let content_hash = crate::hash::content_hash_v1(&content_md);
    let tables = infer_tables_from_pages(&page_entries);

    Ok(PdfExtractResult {
        content_md,
        content_hash,
        tables,
        page_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_returns_error_result() {
        let out = extract_pdf("/tmp/nonexistent_test_file.pdf");
        assert!(out.is_err(), "missing file must return Result::Err");
    }

    #[test]
    fn infers_tables_from_pipe_and_multispace_rows() {
        let pages = vec![
            (
                1u32,
                "Header A | Header B\nValue 1 | Value 2\n\nNot a table line".to_string(),
            ),
            (
                2u32,
                "Country  Fee\nPoland   60 EUR\nSpain    90 EUR".to_string(),
            ),
        ];

        let tables = infer_tables_from_pages(&pages);
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].page, 1);
        assert_eq!(tables[0].data.len(), 2);
        assert_eq!(tables[1].page, 2);
        assert_eq!(tables[1].data.len(), 3);
    }
}
