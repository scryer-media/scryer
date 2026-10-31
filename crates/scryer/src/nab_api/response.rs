use super::request::Format;
use axum::{
    body::Body,
    http::{StatusCode, header},
    response::Response,
};
use scryer_application::{
    AppError, RateLimitCooldownAction,
    error_reasons::{indexer_clients as clients, raw_search},
};
use scryer_infrastructure_acquisition::indexers::nab_caps::{write_json_error, write_xml_error};

/// Compatibility failures use typed conditions, never provider error sentences.
/// Only fixed protocol descriptions leave this boundary: upstream errors can
/// contain credentials, request URLs, and private connection details.
pub(super) fn error_response(error: &AppError, format: Format) -> Response {
    let (status, code, description) = match error.reason() {
        Some(clients::PROTOCOL_FORBIDDEN) => (
            StatusCode::FORBIDDEN,
            102,
            "Insufficient privileges/not authorized",
        ),
        Some(clients::SEARCH_UNSUPPORTED) => {
            (StatusCode::BAD_REQUEST, 203, "Function not available")
        }
        Some(clients::REQUEST_INVALID | clients::CATEGORY_INVALID | raw_search::OFFSET_INVALID) => {
            (StatusCode::BAD_REQUEST, 201, "Incorrect parameter")
        }
        Some(raw_search::ALL_SOURCES_LIMITED) => {
            (StatusCode::TOO_MANY_REQUESTS, 500, "Request limit reached")
        }
        _ => match error {
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. } => {
                (StatusCode::UNAUTHORIZED, 100, "Incorrect user credentials")
            }
            AppError::NewznabQuotaExceeded { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, 500, "Request limit reached")
            }
            AppError::TemporaryUnavailable {
                rate_limit_cooldown,
                ..
            } if !matches!(rate_limit_cooldown, RateLimitCooldownAction::None) => {
                (StatusCode::TOO_MANY_REQUESTS, 500, "Request limit reached")
            }
            _ => (StatusCode::SERVICE_UNAVAILABLE, 900, "Unknown error"),
        },
    };
    let mut body = Vec::new();
    match format {
        Format::Xml => write_xml_error(&mut body, code, description),
        Format::Json => write_json_error(&mut body, code, description),
    }
    .expect("fixed Nab error metadata serializes into memory");
    let mut response = Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header(
            header::CONTENT_TYPE,
            match format {
                Format::Xml => "application/xml; charset=utf-8",
                Format::Json => "application/json; charset=utf-8",
            },
        );
    if status == StatusCode::TOO_MANY_REQUESTS
        && let AppError::TemporaryUnavailable {
            retry_after: Some(delay),
            ..
        } = error
    {
        let seconds = delay
            .as_secs()
            .saturating_add(u64::from(delay.subsec_nanos() != 0))
            .max(1);
        response = response.header(header::RETRY_AFTER, seconds.to_string());
    }
    response
        .body(Body::from(body))
        .expect("fixed Nab error headers are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rate_blocked_scope_uses_consumer_code_and_observed_retry_delay() {
        let error = AppError::temporary_unavailable_reason(
            raw_search::ALL_SOURCES_LIMITED,
            "private upstream detail",
            Some(std::time::Duration::from_millis(1501)),
        );
        let response = error_response(&error, Format::Xml);
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "2");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.contains("code=\"500\"") && body.contains("Request limit reached"));
        assert!(!body.contains("private"));
    }

    #[tokio::test]
    async fn credential_and_source_errors_do_not_expose_internal_sentences() {
        for (error, status, code) in [
            (
                AppError::unauthorized_reason(clients::KEY_INVALID, "apikey=secret"),
                StatusCode::UNAUTHORIZED,
                "100",
            ),
            (
                AppError::Repository("https://upstream.example/?apikey=secret".into()),
                StatusCode::SERVICE_UNAVAILABLE,
                "900",
            ),
            (
                AppError::validation_refused(clients::SEARCH_UNSUPPORTED, "private source config"),
                StatusCode::BAD_REQUEST,
                "203",
            ),
        ] {
            let response = error_response(&error, Format::Json);
            assert_eq!(response.status(), status);
            assert!(response.headers().get(header::RETRY_AFTER).is_none());
            let bytes = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let body = std::str::from_utf8(&bytes).unwrap();
            assert!(body.contains(code));
            assert!(
                !body.contains("secret")
                    && !body.contains("private")
                    && !body.contains("upstream.example")
            );
        }
    }
}
