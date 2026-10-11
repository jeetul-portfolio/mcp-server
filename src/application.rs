use crate::domain::Op;
use serde_json::Value;

/// Safe, client-facing failure categories. Detail stays in server logs.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Invalid(String),
    #[error("record not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("portfolio backend is unavailable")]
    Unavailable,
    #[error("internal error")]
    Internal,
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Invalid(_) => "validation_failed",
            AppError::NotFound => "not_found",
            AppError::Conflict(_) => "conflict",
            AppError::Unavailable => "unavailable",
            AppError::Internal => "internal",
        }
    }
}

/// The single port the MCP layer depends on; core's REST API is one implementation,
/// tests inject fakes.
#[async_trait::async_trait]
pub trait PortfolioApi: Send + Sync {
    async fn execute(&self, op: Op) -> Result<Value, AppError>;
}
