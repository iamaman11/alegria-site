use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use serde_json::{json, Value};

pub fn read_runtime_window(name: &str) -> Option<Vec<u8>> {
    let mut target = PathBuf::from("/dev/shm");
    target.push(format!("alegria_{name}.fbs"));
    fs::read(target).ok()
}

pub fn read_json_window(name: &str) -> Option<Value> {
    let raw = read_runtime_window(name)?;
    if raw.is_empty() {
        return None;
    }
    serde_json::from_slice::<Value>(&raw).ok()
}

pub fn read_sync_status() -> Value {
    match read_json_window("sync_status") {
        Some(v) if v.is_object() => {
            let mut out = serde_json::Map::new();
            out.insert("available".to_string(), json!(true));
            if let Some(obj) = v.as_object() {
                for (k, val) in obj {
                    out.insert(k.clone(), val.clone());
                }
            }
            Value::Object(out)
        }
        _ => json!({
            "available": false,
            "pending_events": null,
            "failed_events": null,
            "max_lag_ms": null
        }),
    }
}

pub fn read_worker_heartbeat(stale_after_s: u64) -> Value {
    let payload = match read_json_window("worker_heartbeat") {
        Some(v) => v,
        None => return json!({"available": false, "is_stale": true}),
    };

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_millis() as i64;

    let ts_epoch_ms = payload.get("ts_epoch_ms").and_then(Value::as_i64);
    let is_stale = match ts_epoch_ms {
        Some(ts) => now_ms.saturating_sub(ts) > (stale_after_s as i64 * 1000),
        None => true,
    };

    let mut out = serde_json::Map::new();
    out.insert("available".to_string(), json!(true));
    out.insert("is_stale".to_string(), json!(is_stale));

    if let Some(obj) = payload.as_object() {
        for (k, val) in obj {
            out.insert(k.clone(), val.clone());
        }
    }

    Value::Object(out)
}
