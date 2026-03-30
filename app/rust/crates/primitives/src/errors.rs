use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    ContractViolation,
    ValidationFailure,
    ForeignKeyViolation,
    ConflictViolation,
    TransportTimeout,
    RemoteRateLimit,
    Remote5xx,
    InfraUnavailable,
    UnexpectedBug,
}

impl ErrorClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContractViolation => "contract_violation",
            Self::ValidationFailure => "validation_failure",
            Self::ForeignKeyViolation => "foreign_key_violation",
            Self::ConflictViolation => "conflict_violation",
            Self::TransportTimeout => "transport_timeout",
            Self::RemoteRateLimit => "remote_rate_limit",
            Self::Remote5xx => "remote_5xx",
            Self::InfraUnavailable => "infra_unavailable",
            Self::UnexpectedBug => "unexpected_bug",
        }
    }
}

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("contract violation: {message}")]
    ContractViolation { message: String },
    #[error("validation failure: {message}")]
    ValidationFailure { message: String },
    #[error("foreign key violation: {message}")]
    ForeignKeyViolation { message: String },
    #[error("conflict violation: {message}")]
    ConflictViolation { message: String },
    #[error("transport timeout: {message}")]
    TransportTimeout { message: String },
    #[error("remote rate limit: {message}")]
    RemoteRateLimit { message: String },
    #[error("remote 5xx: {message}")]
    Remote5xx { message: String },
    #[error("infra unavailable: {message}")]
    InfraUnavailable { message: String },
    #[error("unexpected bug: {message}")]
    UnexpectedBug { message: String },
}

impl DomainError {
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::ContractViolation { .. } => ErrorClass::ContractViolation,
            Self::ValidationFailure { .. } => ErrorClass::ValidationFailure,
            Self::ForeignKeyViolation { .. } => ErrorClass::ForeignKeyViolation,
            Self::ConflictViolation { .. } => ErrorClass::ConflictViolation,
            Self::TransportTimeout { .. } => ErrorClass::TransportTimeout,
            Self::RemoteRateLimit { .. } => ErrorClass::RemoteRateLimit,
            Self::Remote5xx { .. } => ErrorClass::Remote5xx,
            Self::InfraUnavailable { .. } => ErrorClass::InfraUnavailable,
            Self::UnexpectedBug { .. } => ErrorClass::UnexpectedBug,
        }
    }

    pub fn class_str(&self) -> &'static str {
        self.class().as_str()
    }
}
