use async_graphql::{Enum, InputObject, SimpleObject};
use scryer_application::{
    ExternalReleaseInput, ExternalReleaseOutcome, ExternalReleaseProtocol, ExternalReleaseStatus,
    IndexerResponseAttributes,
};

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ExternalReleaseProtocolValue {
    Usenet,
    Torrent,
}

#[derive(InputObject)]
pub struct SubmitExternalReleaseInput {
    pub name: String,
    pub download_url: String,
    pub protocol: ExternalReleaseProtocolValue,
    pub source: String,
    pub size_bytes: Option<i64>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub tvdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    #[graphql(default)]
    pub flags: Vec<String>,
}

impl SubmitExternalReleaseInput {
    pub fn into_application(self) -> async_graphql::Result<ExternalReleaseInput> {
        Ok(ExternalReleaseInput {
            name: self.name,
            download_url: self.download_url,
            protocol: match self.protocol {
                ExternalReleaseProtocolValue::Usenet => ExternalReleaseProtocol::Usenet,
                ExternalReleaseProtocolValue::Torrent => ExternalReleaseProtocol::Torrent,
            },
            source: self.source,
            size_bytes: self.size_bytes,
            published_at: self.published_at,
            external_ids: IndexerResponseAttributes {
                tvdb_id: self.tvdb_id,
                tmdb_id: self.tmdb_id,
                imdb_id: self.imdb_id,
                categories: vec![],
            },
            flags: self.flags,
        })
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ExternalReleaseStatusValue {
    Queued,
    Held,
    Rejected,
    Duplicate,
}

#[derive(SimpleObject)]
pub struct ExternalReleasePayload {
    pub status: ExternalReleaseStatusValue,
    pub reasons: Vec<String>,
    pub title_id: Option<async_graphql::ID>,
    pub download_id: Option<async_graphql::ID>,
    pub pending_id: Option<async_graphql::ID>,
}

impl From<ExternalReleaseOutcome> for ExternalReleasePayload {
    fn from(value: ExternalReleaseOutcome) -> Self {
        Self {
            status: match value.status {
                ExternalReleaseStatus::Queued => ExternalReleaseStatusValue::Queued,
                ExternalReleaseStatus::Held => ExternalReleaseStatusValue::Held,
                ExternalReleaseStatus::Rejected => ExternalReleaseStatusValue::Rejected,
                ExternalReleaseStatus::Duplicate => ExternalReleaseStatusValue::Duplicate,
            },
            reasons: value.reasons,
            title_id: value.title_id.map(Into::into),
            download_id: value.download_id.map(Into::into),
            pending_id: value.pending_id.map(Into::into),
        }
    }
}
