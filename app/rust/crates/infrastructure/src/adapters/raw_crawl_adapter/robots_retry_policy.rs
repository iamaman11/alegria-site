struct RobotsRuleSet {
    allowed: bool,
    matched_rule: Option<String>,
    allow_rules: Vec<String>,
    disallow_rules: Vec<String>,
}

fn parse_robots_rules(body: &str, user_agent: &str, path: &str) -> RobotsRuleSet {
    let mut current_block_matches = false;
    let mut allow_rules = Vec::new();
    let mut disallow_rules = Vec::new();
    let target_agent = user_agent.to_ascii_lowercase();

    for raw_line in body.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            current_block_matches = false;
            continue;
        }
        if let Some(value) = line.strip_prefix("User-agent:") {
            let agent = value.trim().to_ascii_lowercase();
            current_block_matches |= agent == "*" || agent == target_agent;
            continue;
        }
        if !current_block_matches {
            continue;
        }
        if let Some(value) = line.strip_prefix("Allow:") {
            let rule = value.trim().to_string();
            if !rule.is_empty() {
                allow_rules.push(rule);
            }
        } else if let Some(value) = line.strip_prefix("Disallow:") {
            let rule = value.trim().to_string();
            if !rule.is_empty() {
                disallow_rules.push(rule);
            }
        }
    }

    let best_allow = best_matching_rule(&allow_rules, path);
    let best_disallow = best_matching_rule(&disallow_rules, path);
    let allowed = match (&best_allow, &best_disallow) {
        (Some(allow), Some(disallow)) => allow.len() >= disallow.len(),
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => true,
    };
    let matched_rule = if allowed {
        best_allow.or(best_disallow)
    } else {
        best_disallow
    };

    RobotsRuleSet {
        allowed,
        matched_rule,
        allow_rules,
        disallow_rules,
    }
}

fn best_matching_rule(rules: &[String], path: &str) -> Option<String> {
    rules
        .iter()
        .filter(|rule| path.starts_with(rule.as_str()) || rule.as_str() == "/")
        .max_by_key(|rule| rule.len())
        .cloned()
}

pub async fn mark_crawl_done(
    pool: &PgPool,
    url_norm: &str,
    http_status: i32,
    notes: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE serp.crawl_queue
         SET status = 'done',
             http_status = $2,
             next_attempt_at = now(),
             locked_until = NULL,
             last_error = NULL,
             notes = $3
         WHERE url_norm = $1",
    )
    .bind(url_norm)
    .bind(http_status)
    .bind(notes)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn mark_crawl_failed(
    pool: &PgPool,
    url_norm: &str,
    http_status: Option<i32>,
    error: &str,
) -> std::result::Result<(), primitives::errors::DomainError> {
    sqlx::query(
        "UPDATE serp.crawl_queue
         SET status = 'failed',
             http_status = $2,
             next_attempt_at = now(),
             locked_until = NULL,
             last_error = left($3, 2000),
             notes = left($3, 2000)
         WHERE url_norm = $1",
    )
    .bind(url_norm)
    .bind(http_status)
    .bind(error)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub async fn mark_crawl_retry(
    pool: &PgPool,
    url_norm: &str,
    http_status: Option<i32>,
    error: &str,
    attempt_count: i32,
) -> std::result::Result<(), primitives::errors::DomainError> {
    let delay_sec = next_crawl_retry_delay_sec(attempt_count);
    sqlx::query(
        "UPDATE serp.crawl_queue
         SET status = 'pending',
             http_status = $2,
             next_attempt_at = now() + make_interval(secs => $3::int),
             locked_until = NULL,
             last_error = left($4, 2000),
             notes = concat_ws('; ', nullif(notes, ''), left($4, 1800), concat('retry_in_sec=', $3::text))
         WHERE url_norm = $1",
    )
    .bind(url_norm)
    .bind(http_status)
    .bind(delay_sec)
    .bind(error)
    .execute(pool)
    .await
    .map_err(classify_sqlx)?;
    Ok(())
}

pub fn should_retry_http_status(status_code: i32) -> bool {
    status_code == 408
        || status_code == 409
        || status_code == 425
        || status_code == 429
        || status_code >= 500
}

pub fn should_retry_crawl_attempt(attempt_count: i32) -> bool {
    attempt_count > 0 && attempt_count < MAX_CRAWL_ATTEMPTS
}

pub fn next_crawl_retry_delay_sec(attempt_count: i32) -> i32 {
    match attempt_count {
        0 | 1 => 60,
        2 => 300,
        3 => 1800,
        _ => 7200,
    }
}

