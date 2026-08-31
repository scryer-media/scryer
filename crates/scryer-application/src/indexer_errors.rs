use crate::{
    CapturedIndexerHttpResponse, IndexerErrorClassification, IndexerErrorDetail, IndexerErrorPage,
    NewIndexerError,
};
use async_trait::async_trait;

pub const UNKNOWN_INDEXER_ERROR_MESSAGE: &str = "Unknown indexer error";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassifiedIndexerError {
    pub classification: IndexerErrorClassification,
    pub provider_error_code: Option<u16>,
    pub message: &'static str,
}

/// Classifies an accepted indexer HTTP response. Newznab error documents take
/// precedence over HTTP status because those documents may use a 2xx status.
pub fn classify_indexer_http_response(
    response: &CapturedIndexerHttpResponse,
) -> Option<ClassifiedIndexerError> {
    if let Some(code) = newznab_error_code(&response.body) {
        return Some(classify_newznab_code(code));
    }

    match response.status {
        400 => Some(http_error(
            IndexerErrorClassification::HttpBadRequest,
            "Indexer request was invalid",
        )),
        401 => Some(http_error(
            IndexerErrorClassification::HttpUnauthorized,
            "Indexer authentication failed",
        )),
        403 => Some(http_error(
            IndexerErrorClassification::HttpForbidden,
            "Indexer access was forbidden",
        )),
        404 => Some(http_error(
            IndexerErrorClassification::HttpNotFound,
            "Indexer endpoint was not found",
        )),
        408 => Some(http_error(
            IndexerErrorClassification::HttpRequestTimeout,
            "Indexer request timed out",
        )),
        429 => Some(http_error(
            IndexerErrorClassification::HttpRateLimited,
            "Indexer rate limit reached",
        )),
        500..=599 => Some(http_error(
            IndexerErrorClassification::HttpServerError,
            "Indexer server error",
        )),
        200..=299 => None,
        _ => Some(unknown_error()),
    }
}

/// Preserves the existing connection-test recognition for plugin error text.
pub fn classify_newznab_error_message(message: &str) -> Option<ClassifiedIndexerError> {
    let lower = message.to_ascii_lowercase();
    let marker = lower.find("newznab")?;
    let after_marker = &message[marker..];
    let lower_after_marker = &lower[marker..];
    let error_marker = lower_after_marker.find("error")?;
    let after_error = &after_marker[error_marker + "error".len()..];
    let code = after_error
        .split(|character: char| !character.is_ascii_digit())
        .find(|part| !part.is_empty())?
        .parse::<u16>()
        .ok()?;
    let classified = classify_newznab_code(code);
    (!matches!(
        classified.classification,
        IndexerErrorClassification::Unknown
    ))
    .then_some(classified)
}

pub fn unknown_indexer_error() -> ClassifiedIndexerError {
    unknown_error()
}

pub fn indexer_response_content_type(response: &CapturedIndexerHttpResponse) -> Option<String> {
    response
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("content-type"))
        .and_then(|header| std::str::from_utf8(&header.value).ok())
        .map(ToOwned::to_owned)
}

pub fn redact_indexer_response_headers(response: &mut CapturedIndexerHttpResponse) {
    for header in &mut response.headers {
        if is_sensitive_response_header_name(&header.name) {
            header.value.clear();
        }
    }
}

pub fn is_sensitive_response_header_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
    ) || normalized.contains("api-key")
        || normalized.contains("apikey")
        || normalized.contains("api_key")
        || normalized.contains("token")
        || normalized.contains("secret")
}

#[async_trait]
pub trait IndexerErrorRepository: Send + Sync {
    async fn record(&self, error: NewIndexerError) -> crate::AppResult<()>;
    async fn list(
        &self,
        indexer_id: Option<&str>,
        first: usize,
        after: Option<&str>,
    ) -> crate::AppResult<IndexerErrorPage>;
    async fn get_detail(&self, id: &str) -> crate::AppResult<Option<IndexerErrorDetail>>;
    async fn delete_older_than(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> crate::AppResult<u32>;
}

/// The id of the throwaway `IndexerConfig` a connection test probes under.
///
/// A connection test runs before the indexer exists (or without touching the
/// stored one), so there is no `indexers` row to key error history on and
/// `indexer_errors.indexer_id` is a foreign key onto that table.
pub const CONNECTION_TEST_INDEXER_ID: &str = "test-connection";

/// Whether error history may be persisted for this indexer id.
///
/// Capture paths ask before recording: writing history for the connection-test
/// id can only ever fail the foreign key, and a storage failure raised behind a
/// failed probe buries the probe's own error — which is the one thing the
/// operator asked the connection test for.
pub fn indexer_error_history_is_persistable(indexer_id: &str) -> bool {
    indexer_id.trim() != CONNECTION_TEST_INDEXER_ID
}

pub trait IndexerErrorRecorder: Send + Sync {
    fn record(&self, error: NewIndexerError) -> crate::AppResult<()>;
}

#[derive(Default)]
pub struct NullIndexerErrorRecorder;

impl IndexerErrorRecorder for NullIndexerErrorRecorder {
    fn record(&self, _error: NewIndexerError) -> crate::AppResult<()> {
        Ok(())
    }
}

fn newznab_error_code(body: &[u8]) -> Option<u16> {
    use quick_xml::{Reader, events::Event};

    let mut reader = Reader::from_reader(body);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if element.name().as_ref().eq_ignore_ascii_case("error") =>
            {
                for attribute in element.attributes().flatten() {
                    if attribute.key.as_ref().eq_ignore_ascii_case("code") {
                        return attribute.value.parse().ok();
                    }
                }
                return None;
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => buffer.clear(),
        }
    }
}

fn classify_newznab_code(code: u16) -> ClassifiedIndexerError {
    let (classification, message) = match code {
        100 => (
            IndexerErrorClassification::NewznabInvalidApiKey,
            "Invalid API Key",
        ),
        101 => (
            IndexerErrorClassification::NewznabAccountSuspended,
            "Account suspended",
        ),
        102 => (
            IndexerErrorClassification::NewznabInsufficientPrivileges,
            "Insufficient privileges",
        ),
        103 => (
            IndexerErrorClassification::NewznabRegistrationDenied,
            "Registration denied",
        ),
        104 => (
            IndexerErrorClassification::NewznabRegistrationsClosed,
            "Registrations are closed",
        ),
        105 => (
            IndexerErrorClassification::NewznabInvalidRegistration,
            "Invalid registration",
        ),
        106 => (
            IndexerErrorClassification::NewznabInvalidRegistrationEmail,
            "Invalid registration email address",
        ),
        107 => (
            IndexerErrorClassification::NewznabRegistrationFailed,
            "Registration failed",
        ),
        200 => (
            IndexerErrorClassification::NewznabMissingParameter,
            "Missing parameter",
        ),
        201 => (
            IndexerErrorClassification::NewznabIncorrectParameter,
            "Incorrect parameter",
        ),
        202 => (
            IndexerErrorClassification::NewznabNoSuchFunction,
            "No such function",
        ),
        203 => (
            IndexerErrorClassification::NewznabFunctionNotAvailable,
            "Function not available",
        ),
        300 => (
            IndexerErrorClassification::NewznabNoSuchItem,
            "No such item",
        ),
        500 => (
            IndexerErrorClassification::NewznabRequestLimitReached,
            "Request limit reached",
        ),
        501 => (
            IndexerErrorClassification::NewznabDownloadLimitReached,
            "Download limit reached",
        ),
        900 => (
            IndexerErrorClassification::NewznabUnknownError,
            "Unknown Newznab error",
        ),
        910 => (
            IndexerErrorClassification::NewznabApiDisabled,
            "Newznab API disabled",
        ),
        _ => return unknown_error(),
    };
    ClassifiedIndexerError {
        classification,
        provider_error_code: Some(code),
        message,
    }
}

fn http_error(
    classification: IndexerErrorClassification,
    message: &'static str,
) -> ClassifiedIndexerError {
    ClassifiedIndexerError {
        classification,
        provider_error_code: None,
        message,
    }
}

fn unknown_error() -> ClassifiedIndexerError {
    ClassifiedIndexerError {
        classification: IndexerErrorClassification::Unknown,
        provider_error_code: None,
        message: UNKNOWN_INDEXER_ERROR_MESSAGE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CapturedIndexerHttpHeader;

    fn response(status: u16, body: impl AsRef<[u8]>) -> CapturedIndexerHttpResponse {
        CapturedIndexerHttpResponse {
            status,
            headers: Vec::new(),
            body: body.as_ref().to_vec(),
        }
    }

    #[test]
    fn known_newznab_errors_take_precedence_over_http_status() {
        let classifications = [
            (100, IndexerErrorClassification::NewznabInvalidApiKey),
            (101, IndexerErrorClassification::NewznabAccountSuspended),
            (
                102,
                IndexerErrorClassification::NewznabInsufficientPrivileges,
            ),
            (103, IndexerErrorClassification::NewznabRegistrationDenied),
            (104, IndexerErrorClassification::NewznabRegistrationsClosed),
            (105, IndexerErrorClassification::NewznabInvalidRegistration),
            (
                106,
                IndexerErrorClassification::NewznabInvalidRegistrationEmail,
            ),
            (107, IndexerErrorClassification::NewznabRegistrationFailed),
            (200, IndexerErrorClassification::NewznabMissingParameter),
            (201, IndexerErrorClassification::NewznabIncorrectParameter),
            (202, IndexerErrorClassification::NewznabNoSuchFunction),
            (203, IndexerErrorClassification::NewznabFunctionNotAvailable),
            (300, IndexerErrorClassification::NewznabNoSuchItem),
            (500, IndexerErrorClassification::NewznabRequestLimitReached),
            (501, IndexerErrorClassification::NewznabDownloadLimitReached),
            (900, IndexerErrorClassification::NewznabUnknownError),
            (910, IndexerErrorClassification::NewznabApiDisabled),
        ];
        for (code, expected) in classifications {
            let classified = classify_indexer_http_response(&response(
                200,
                format!(r#"<?xml version="1.0"?><error code="{code}"/>"#),
            ))
            .expect("Newznab error document should be terminal");
            assert_eq!(classified.classification, expected);
            assert_eq!(classified.provider_error_code, Some(code));
        }
    }

    #[test]
    fn http_errors_and_unknown_fallback_are_classified() {
        assert_eq!(
            classify_indexer_http_response(&response(429, "busy"))
                .expect("429 is an error")
                .classification,
            IndexerErrorClassification::HttpRateLimited
        );
        assert_eq!(
            classify_indexer_http_response(&response(503, "busy"))
                .expect("503 is an error")
                .classification,
            IndexerErrorClassification::HttpServerError
        );
        assert_eq!(
            classify_indexer_http_response(&response(418, "teapot"))
                .expect("non-2xx is an error")
                .classification,
            IndexerErrorClassification::Unknown
        );
        assert_eq!(
            classify_indexer_http_response(&response(200, r#"<error code="777"/>"#))
                .expect("unknown Newznab code is still terminal")
                .classification,
            IndexerErrorClassification::Unknown
        );
    }

    #[test]
    fn sensitive_headers_are_redacted_without_changing_header_shape() {
        let mut response = CapturedIndexerHttpResponse {
            status: 401,
            headers: vec![
                CapturedIndexerHttpHeader {
                    name: "Set-Cookie".to_string(),
                    value: b"session=secret".to_vec(),
                },
                CapturedIndexerHttpHeader {
                    name: "X-RateLimit-Limit".to_string(),
                    value: b"10".to_vec(),
                },
            ],
            body: vec![0, 255],
        };
        redact_indexer_response_headers(&mut response);
        assert!(response.headers[0].value.is_empty());
        assert_eq!(response.headers[1].value, b"10");
        assert_eq!(response.body, vec![0, 255]);
    }
}
