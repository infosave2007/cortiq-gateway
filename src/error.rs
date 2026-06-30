//! Unified gateway error type + rendering into an OpenAI-compatible envelope
//! `{"error": {message, type, code, param}}` that SDK clients parse natively.
//! See docs/PROTOCOLS.md §8.5.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("authentication failed: {0}")]
    Unauthorized(String),
    #[error("rate limit exceeded")]
    RateLimited { retry_after_secs: u64 },
    #[error("upstream unavailable: {0}")]
    UpstreamUnavailable(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl GatewayError {
    fn parts(&self) -> (StatusCode, &'static str, &'static str) {
        match self {
            Self::InvalidRequest(_) => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "invalid_request",
            ),
            Self::Unauthorized(_) => (
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "unauthorized",
            ),
            Self::RateLimited { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limit_error",
                "rate_limited",
            ),
            Self::UpstreamUnavailable(_) => (
                StatusCode::BAD_GATEWAY,
                "upstream_unavailable",
                "no_healthy_model",
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal",
            ),
        }
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> axum::response::Response {
        let (status, typ, code) = self.parts();
        let body = Json(json!({
            "error": {
                "message": self.to_string(),
                "type": typ,
                "code": code,
                "param": serde_json::Value::Null,
            }
        }));
        let mut resp = (status, body).into_response();
        if let Self::RateLimited { retry_after_secs } = self {
            resp.headers_mut()
                .insert("retry-after", retry_after_secs.to_string().parse().unwrap());
        }
        resp
    }
}

pub type Result<T> = std::result::Result<T, GatewayError>;
