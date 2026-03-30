use tracing::{error, info};

pub fn telemetry_info(message: &str) {
    info!(target: "alegria.telemetry", "{message}");
}

pub fn telemetry_error(message: &str) {
    error!(target: "alegria.telemetry", "{message}");
}
