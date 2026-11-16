use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use composer_core::ProjectSpec;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

#[derive(Debug, Serialize)]
struct ValidationResponse {
    valid: bool,
    spec: ProjectSpec,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: &'static str,
    detail: String,
}

#[derive(Debug)]
enum ApiError {
    InvalidJson(String),
    InvalidSpecification(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error, detail) = match self {
            Self::InvalidJson(detail) => (StatusCode::BAD_REQUEST, "invalid_json", detail),
            Self::InvalidSpecification(detail) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_specification",
                detail,
            ),
        };

        (status, Json(ErrorResponse { error, detail })).into_response()
    }
}

/// Build the HTTP router. Kept separate from the listener for in-process tests.
pub fn router() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/specifications/validate", post(validate_specification))
}

/// Bind and serve the API on the supplied socket address.
pub async fn serve(bind: SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, router()).await
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "software-composer-api",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn validate_specification(
    payload: Result<Json<ProjectSpec>, JsonRejection>,
) -> Result<Json<ValidationResponse>, ApiError> {
    let Json(spec) = payload.map_err(|error| ApiError::InvalidJson(error.body_text()))?;

    spec.validate()
        .map_err(|error| ApiError::InvalidSpecification(error.to_string()))?;

    Ok(Json(ValidationResponse { valid: true, spec }))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::router;

    #[tokio::test]
    async fn health_endpoint_reports_service_status() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn validation_endpoint_accepts_a_valid_specification() {
        let body = r#"{
            "name": "Task Platform",
            "description": "A task platform assembled from reusable capabilities.",
            "capabilities": [
                {
                    "id": "authentication",
                    "description": "User sign-in and session management.",
                    "priority": "critical"
                }
            ],
            "constraints": {
                "preferred_languages": ["rust", "typescript"]
            }
        }"#;

        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/specifications/validate")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request should be valid"),
            )
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::OK);
        let response_body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&response_body).unwrap();
        assert_eq!(json["valid"], true);
        assert_eq!(json["spec"]["name"], "Task Platform");
    }

    #[tokio::test]
    async fn validation_endpoint_rejects_duplicate_capabilities() {
        let body = r#"{
            "name": "Task Platform",
            "description": "A task platform.",
            "capabilities": [
                {"id": "auth", "description": "First."},
                {"id": "auth", "description": "Duplicate."}
            ]
        }"#;

        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/specifications/validate")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request should be valid"),
            )
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn validation_endpoint_rejects_malformed_json() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/specifications/validate")
                    .header("content-type", "application/json")
                    .body(Body::from("{"))
                    .expect("request should be valid"),
            )
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
