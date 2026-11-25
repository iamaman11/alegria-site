//! One bounded JSON serializer shared by the pure site runner and the
//! provider-neutral Temporal DAG state machine. No temporary oversized
//! allocation is needed before rejecting an envelope.

use std::io::{self, Write};

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JsonBudgetError {
    #[error("DAG {resource} exceeds resource budget {limit}")]
    ResourceBudgetExceeded {
        resource: &'static str,
        limit: usize,
    },
    #[error("DAG envelope cannot be serialized: {0}")]
    EnvelopeSerialization(String),
}

struct CountingJsonWriter {
    limit: usize,
    bytes: usize,
    exceeded: bool,
}

impl Write for CountingJsonWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.limit.saturating_sub(self.bytes) {
            self.exceeded = true;
            return Err(io::Error::other("JSON budget exceeded"));
        }
        self.bytes += buffer.len();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn enforce_json_budget<T: Serialize>(
    value: &T,
    limit: usize,
    resource: &'static str,
) -> Result<(), JsonBudgetError> {
    let mut writer = CountingJsonWriter {
        limit,
        bytes: 0,
        exceeded: false,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(()),
        Err(_) if writer.exceeded => {
            Err(JsonBudgetError::ResourceBudgetExceeded { resource, limit })
        }
        Err(error) => Err(JsonBudgetError::EnvelopeSerialization(error.to_string())),
    }
}

pub const DAG_MAX_NODES: usize = 64;
pub const DAG_MAX_INPUT_BYTES: usize = 512 * 1024;
pub const DAG_MAX_RESULT_BYTES: usize = 512 * 1024;
