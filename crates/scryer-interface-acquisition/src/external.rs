use async_graphql::{Enum, InputObject, SimpleObject};
use scryer_application::{
    ExternalReleaseInput, ExternalReleaseOutcome, ExternalReleaseProtocol, ExternalReleaseStatus,
    IndexerResponseAttributes,
};

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
/// Download protocol advertised by an external release source.
pub enum ExternalReleaseProtocolValue {
    /// An NZB release downloaded through a Usenet client.
    Usenet,
    /// A torrent file or magnet downloaded through a BitTorrent client.
    Torrent,
}

#[derive(InputObject)]
/// Externally discovered release evaluated against the existing authorized catalog.
pub struct SubmitExternalReleaseInput {
    /// Original release name used for title, episode, and quality matching.
    pub name: String,
    /// HTTP or HTTPS download URL, or a valid torrent magnet URI.
    pub download_url: String,
    /// Protocol used to route the release to a download client.
    pub protocol: ExternalReleaseProtocolValue,
    /// Source name retained for attribution without creating an indexer configuration.
    pub source: String,
    /// Advertised nonnegative release size in bytes, when available.
    pub size_bytes: Option<i64>,
    /// Original publication time used by normal acquisition delay rules.
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Optional TVDB title identity supplied by the source.
    pub tvdb_id: Option<String>,
    /// Optional TMDB title identity supplied by the source.
    pub tmdb_id: Option<String>,
    /// Optional IMDb title identity, including its tt prefix.
    pub imdb_id: Option<String>,
    /// Release flags: freeleech, halfleech, double_upload, internal, scene, freeleech_75, freeleech_25, nuked, subtitles, golden, or approved.
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
/// Result of evaluating an externally announced release.
pub enum ExternalReleaseStatusValue {
    /// A new download was submitted through normal acquisition.
    Queued,
    /// The release was persisted for later automatic acquisition evaluation.
    Held,
    /// Matching or acquisition policy refused the release.
    Rejected,
    /// The release already has an existing download; no new download was submitted.
    Duplicate,
}

#[derive(SimpleObject)]
/// Acquisition decision and durable identities associated with an external release.
pub struct ExternalReleasePayload {
    /// Whether the release queued, was held, was rejected, or already existed.
    pub status: ExternalReleaseStatusValue,
    /// Decision explanations, including delay or rejection reasons.
    pub reasons: Vec<String>,
    /// Matched authorized catalog title, when one was resolved.
    pub title_id: Option<async_graphql::ID>,
    /// New or existing canonical download identity, when applicable.
    pub download_id: Option<async_graphql::ID>,
    /// Durable pending-release identity when acquisition is delayed.
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
