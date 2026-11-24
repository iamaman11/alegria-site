//! One PostgreSQL SQLSTATE-to-domain-error classifier for all SQLx owners.

use primitives::errors::DomainError;

pub fn classify_sqlx(error: sqlx::Error) -> DomainError {
    match error {
        sqlx::Error::RowNotFound => DomainError::ValidationFailure {
            message: "expected row was not found".to_string(),
        },
        sqlx::Error::Database(db) => {
            let code = db.code().map(|code| code.into_owned()).unwrap_or_default();
            let message = format!(
                "PostgreSQL SQLSTATE={code} constraint={}: {}",
                db.constraint().unwrap_or("-"),
                db.message()
            );
            match code.as_str() {
                "23505" | "23P01" => DomainError::ConflictViolation { message },
                "23503" => DomainError::ForeignKeyViolation { message },
                "23502" | "23514" | "22001" | "22P02" => {
                    DomainError::ValidationFailure { message }
                }
                _ if code.starts_with("42") => DomainError::ContractViolation { message },
                _ => DomainError::InfraUnavailable { message },
            }
        }
        other => DomainError::InfraUnavailable {
            message: other.to_string(),
        },
    }
}
