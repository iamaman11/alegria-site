use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerRouterInput {
    pub section_id: String,
    pub heading_text: String,
    pub raw_text: String,
    pub source_tier: String,
    pub block_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerScores {
    pub procedural: f32,
    pub operational: f32,
    pub editorial: f32,
    pub seo: f32,
    pub commercial: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecondaryLayer {
    pub layer: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerRouterOutput {
    pub layer_scores: LayerScores,
    pub primary_layer: String,
    pub secondary_layers: Vec<SecondaryLayer>,
    pub confidence: f32,
    pub needs_hitl: bool,
    pub hitl_reason: Option<String>,
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

pub fn execute(input: &LayerRouterInput) -> LayerRouterOutput {
    let text = format!(
        "{}\n{}\n{}\n{}",
        input.heading_text, input.raw_text, input.source_tier, input.block_type
    )
    .to_lowercase();

    let mut p = 0.15f32;
    let mut o = 0.10f32;
    let mut e = 0.10f32;
    let mut s = 0.05f32;
    let mut c = 0.05f32;

    for k in ["документ", "паспорт", "сбор", "fee", "eur", "must", "требует"] {
        if text.contains(k) {
            p += 0.12;
        }
    }
    for k in ["schedule", "график", "выходн", "holiday", "notice", "время работы"] {
        if text.contains(k) {
            o += 0.14;
        }
    }
    for k in ["faq", "как", "почему", "совет", "ошибка", "pain"] {
        if text.contains(k) {
            e += 0.11;
        }
    }
    for k in ["ключев", "seo", "serp", "ранжир", "интент"] {
        if text.contains(k) {
            s += 0.14;
        }
    }
    for k in ["услуга", "стоимость услуги", "под ключ", "консультац", "заказать"] {
        if text.contains(k) {
            c += 0.14;
        }
    }

    let scores = LayerScores {
        procedural: clamp01(p),
        operational: clamp01(o),
        editorial: clamp01(e),
        seo: clamp01(s),
        commercial: clamp01(c),
    };

    let mut ranked = vec![
        ("procedural".to_string(), scores.procedural),
        ("operational".to_string(), scores.operational),
        ("editorial".to_string(), scores.editorial),
        ("seo".to_string(), scores.seo),
        ("commercial".to_string(), scores.commercial),
    ];
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let primary_layer = ranked[0].0.clone();
    let confidence = ranked[0].1;
    let mut secondary_layers = Vec::new();
    for (layer, score) in ranked.iter().skip(1) {
        if *score >= 0.50 {
            secondary_layers.push(SecondaryLayer {
                layer: layer.clone(),
                confidence: *score,
            });
        }
    }

    let needs_hitl = confidence < 0.60;
    let hitl_reason = if needs_hitl {
        Some("low_layer_confidence".to_string())
    } else {
        None
    };

    LayerRouterOutput {
        layer_scores: scores,
        primary_layer,
        secondary_layers,
        confidence,
        needs_hitl,
        hitl_reason,
    }
}
