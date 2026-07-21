//! API error types and their HTTP representation.

use crate::dto::ErrorResponse;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("upstream error: {0}")]
    Upstream(String),

    #[error("bad request: {0}")]
    BadRequest(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Upstream(_) => StatusCode::BAD_GATEWAY,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
        };
        (
            status,
            Json(ErrorResponse {
                error: self.to_string(),
            }),
        )
            .into_response()
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_found_maps_to_404() {
        let response = ApiError::NotFound("order abc123".to_string()).into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_not_found_body_carries_message() {
        let err = ApiError::NotFound("order abc123".to_string());
        assert_eq!(err.to_string(), "not found: order abc123");
    }

    #[test]
    fn test_upstream_maps_to_502() {
        let response = ApiError::Upstream("RPC timed out".to_string()).into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn test_bad_request_maps_to_400() {
        let response = ApiError::BadRequest("invalid max_capital_usd".to_string()).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
