//! Unified API error type — every handler returns `Result<T, AppError>` and
//! errors are rendered as `{"error": "<message>"}` with the right status code.

use actix_web::{HttpResponse, ResponseError};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    Internal(String),
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn db(error: sqlx::Error) -> Self {
        match &error {
            sqlx::Error::Database(db_error) if db_error.is_unique_violation() => {
                Self::Conflict("Duplicate value violates a unique constraint".to_string())
            }
            sqlx::Error::RowNotFound => Self::NotFound("Record not found".to_string()),
            _ => {
                log::error!("Database error: {error}");
                Self::Internal("Internal database error".to_string())
            }
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        Self::db(error)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(message) => write!(formatter, "{message}"),
            Self::Unauthorized(message) => write!(formatter, "{message}"),
            Self::Forbidden(message) => write!(formatter, "{message}"),
            Self::NotFound(message) => write!(formatter, "{message}"),
            Self::Conflict(message) => write!(formatter, "{message}"),
            Self::Internal(message) => write!(formatter, "{message}"),
        }
    }
}

impl ResponseError for AppError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        match self {
            Self::BadRequest(_) => actix_web::http::StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => actix_web::http::StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => actix_web::http::StatusCode::FORBIDDEN,
            Self::NotFound(_) => actix_web::http::StatusCode::NOT_FOUND,
            Self::Conflict(_) => actix_web::http::StatusCode::CONFLICT,
            Self::Internal(_) => actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code())
            .content_type("application/json")
            .body(json!({ "error": self.to_string() }).to_string())
    }
}
