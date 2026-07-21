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
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
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
}
