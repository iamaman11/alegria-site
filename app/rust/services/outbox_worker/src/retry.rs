pub fn next_retry_delay_sec(retry_count: i32) -> i64 {
    match retry_count {
        0 => 5,
        1 => 30,
        2 => 120,
        _ => 600,
    }
}
