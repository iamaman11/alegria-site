//! Pure crawl eligibility and retry policy for acquired source documents.
//!
//! Network access, queue state, and retry scheduling stay in infrastructure.
//! A failure's retryability and its delay have one owner here, with no SQL,
//! HTTP client, or workflow payload dependencies.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotsRules {
    pub allowed: bool,
    pub matched_rule: Option<String>,
    pub allow_rules: Vec<String>,
    pub disallow_rules: Vec<String>,
}

/// Preserve the crawler's existing longest-prefix and allow-on-tie policy.
pub fn evaluate_robots_rules(body: &str, user_agent: &str, path: &str) -> RobotsRules {
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

    RobotsRules {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryDelaySeconds(i32);

impl RetryDelaySeconds {
    pub fn as_i32(self) -> i32 {
        self.0
    }
}

const MAX_CRAWL_ATTEMPTS: i32 = 4;

/// Transport-level HTTP failures are retryable only for the established codes.
pub fn retry_after_http_response(
    status_code: i32,
    attempt_count: i32,
) -> Option<RetryDelaySeconds> {
    if !(status_code == 408
        || status_code == 409
        || status_code == 425
        || status_code == 429
        || status_code >= 500)
    {
        return None;
    }
    retry_after_recoverable_failure(attempt_count)
}

/// Fetch errors and failed downstream projection share the same attempt budget.
pub fn retry_after_recoverable_failure(attempt_count: i32) -> Option<RetryDelaySeconds> {
    if !(attempt_count > 0 && attempt_count < MAX_CRAWL_ATTEMPTS) {
        return None;
    }
    let delay = match attempt_count {
        1 => 60,
        2 => 300,
        3 => 1800,
        _ => 7200,
    };
    Some(RetryDelaySeconds(delay))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_allow_tie_and_longest_disallow_keep_existing_precedence() {
        let body = concat!(
            "User-agent: *\nDisallow: /private/\nAllow: /private/public/\n",
            "Disallow: /equal/\nAllow: /equal/\n"
        );
        let blocked = evaluate_robots_rules(body, "AlegriaBot", "/private/secret");
        assert!(!blocked.allowed);
        assert_eq!(blocked.matched_rule.as_deref(), Some("/private/"));
        let allowed = evaluate_robots_rules(body, "AlegriaBot", "/private/public/doc");
        assert!(allowed.allowed);
        assert_eq!(allowed.matched_rule.as_deref(), Some("/private/public/"));
        let tied = evaluate_robots_rules(body, "AlegriaBot", "/equal/file");
        assert!(tied.allowed);
        assert_eq!(tied.matched_rule.as_deref(), Some("/equal/"));
    }

    #[test]
    fn robots_user_agent_blocks_and_comments_preserve_original_behavior() {
        let body = concat!(
            "User-agent: OtherBot\nDisallow: /other/\n\n",
            "User-agent: AlegriaBot\nDisallow: /my/ # comment\n"
        );
        assert!(!evaluate_robots_rules(body, "alegriabot", "/my/page").allowed);
        assert!(evaluate_robots_rules(body, "alegriabot", "/other/page").allowed);
        let none = evaluate_robots_rules("User-agent: BadBot\nDisallow: /\n", "AlegriaBot", "/");
        assert!(none.allowed);
        assert!(none.matched_rule.is_none());
    }

    #[test]
    fn retry_budget_and_http_classes_are_one_typed_policy() {
        for (attempt, delay) in [
            (0, None),
            (1, Some(60)),
            (2, Some(300)),
            (3, Some(1800)),
            (4, None),
            (5, None),
        ] {
            assert_eq!(
                retry_after_recoverable_failure(attempt).map(RetryDelaySeconds::as_i32),
                delay
            );
        }
        for code in [408, 409, 425, 429, 500, 503] {
            assert_eq!(
                retry_after_http_response(code, 2).map(RetryDelaySeconds::as_i32),
                Some(300)
            );
        }
        for code in [200, 301, 400, 401, 404, 499] {
            assert!(retry_after_http_response(code, 2).is_none());
        }
        assert!(retry_after_http_response(503, 4).is_none());
    }
}
