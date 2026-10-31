use super::{AppError, AppUseCase, AuthenticatedIndexerClient};

/// A caps response was prepared for an authenticated client.
pub const CAPS_READY: &str = "INDEXER_CLIENTS_CAPS_READY";
/// A search page was prepared without an unavailable or pending source.
pub const SEARCH_READY: &str = "INDEXER_CLIENTS_SEARCH_READY";
/// A search page was prepared with pending or unavailable sources.
pub const SEARCH_PARTIAL: &str = "INDEXER_CLIENTS_SEARCH_PARTIAL";
/// An authorized artifact was fetched and revalidated for delivery.
pub const ARTIFACT_READY: &str = "INDEXER_CLIENTS_ARTIFACT_READY";
/// An authenticated request was refused by its current authorization or input.
pub const REFUSED: &str = "INDEXER_CLIENTS_REQUEST_REFUSED";
/// An authenticated request could not currently be served.
pub const UNAVAILABLE: &str = "INDEXER_CLIENTS_REQUEST_UNAVAILABLE";
/// An authenticated request failed before preparing a response.
pub const FAILED: &str = "INDEXER_CLIENTS_REQUEST_FAILED";

pub const ALL: &[&str] = &[
    CAPS_READY,
    SEARCH_READY,
    SEARCH_PARTIAL,
    ARTIFACT_READY,
    REFUSED,
    UNAVAILABLE,
    FAILED,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexerClientUseOutcome {
    CapsReady,
    SearchReady,
    SearchPartial,
    ArtifactReady,
    Refused,
    Unavailable,
    Failed,
}

impl IndexerClientUseOutcome {
    pub fn code(self) -> &'static str {
        match self {
            Self::CapsReady => CAPS_READY,
            Self::SearchReady => SEARCH_READY,
            Self::SearchPartial => SEARCH_PARTIAL,
            Self::ArtifactReady => ARTIFACT_READY,
            Self::Refused => REFUSED,
            Self::Unavailable => UNAVAILABLE,
            Self::Failed => FAILED,
        }
    }

    pub fn from_error(error: &AppError) -> Self {
        match error {
            AppError::Unauthorized(_)
            | AppError::UnauthorizedReason { .. }
            | AppError::Validation(_)
            | AppError::ValidationRefused { .. } => Self::Refused,
            AppError::TemporaryUnavailable { .. } | AppError::NewznabQuotaExceeded { .. } => {
                Self::Unavailable
            }
            _ => Self::Failed,
        }
    }
}

impl AppUseCase {
    /// Status describes application response preparation, not transport delivery.
    /// Await the existing writer directly: reporting never spawns an unbounded
    /// task queue and cannot turn an otherwise valid response into a failure.
    pub async fn record_indexer_client_use(
        &self,
        authenticated: &AuthenticatedIndexerClient,
        outcome: IndexerClientUseOutcome,
    ) {
        let result = async {
            self.indexer_client_repository()?
                .record_use(
                    &authenticated.client.id,
                    authenticated.client.revision,
                    self.runtime.environment.now(),
                    outcome.code(),
                )
                .await
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(client_id = %authenticated.client.id, %error, "could not record indexer client use");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn usage_codes_are_distinct_and_area_prefixed() {
        assert_eq!(ALL.iter().copied().collect::<HashSet<_>>().len(), ALL.len());
        assert!(ALL.iter().all(|code| {
            code.starts_with("INDEXER_CLIENTS_")
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
        }));
    }

    #[test]
    fn outcome_classification_uses_typed_errors_only() {
        assert_eq!(
            IndexerClientUseOutcome::from_error(&AppError::Repository(REFUSED.into())),
            IndexerClientUseOutcome::Failed,
        );
        assert_eq!(
            IndexerClientUseOutcome::from_error(&AppError::temporary_unavailable(
                "any wording",
                None
            )),
            IndexerClientUseOutcome::Unavailable,
        );
        assert_eq!(
            IndexerClientUseOutcome::from_error(&AppError::unauthorized_reason(
                super::super::reasons::PROTOCOL_FORBIDDEN,
                "any wording"
            )),
            IndexerClientUseOutcome::Refused,
        );
    }
}
