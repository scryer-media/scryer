//! The versioned metadata snapshot a media request carries (FR-030).
//!
//! A request is decided against the metadata that was true *when it was submitted*, and the
//! decision has to stay explainable long after SMG has moved on. So enrichment captures every
//! fact the rule surface can read into one document, and that document is persisted verbatim on
//! the request row (`media_requests.metadata_snapshot_json`).
//!
//! The one thing this module refuses to do is lose the difference between "SMG says there is no
//! content rating" and "we could not ask SMG". The first is an answer; the second is
//! [`MediaRequestMetadataSnapshot::partial`] with the unavailable groups named in `missing`, so a
//! fact derived from them reads as *unknown* (⇒ manual review) rather than quietly absent.

use chrono::{DateTime, Utc};
use scryer_domain::CanonicalMediaTag;
use serde::{Deserialize, Serialize};

use crate::types::{ContentRating, MdblistSummary, TitleAward, TitleRatingSummary};
use crate::{MovieMetadata, SeriesMetadata};

/// Bump when a stored snapshot's *meaning* changes, not when a field is added — every field is
/// `serde(default)`, so an older document still reads correctly into a newer struct.
pub const MEDIA_REQUEST_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// `missing` group names. A group is listed when enrichment could not establish it at all.
pub const SNAPSHOT_GROUP_ALL: &str = "all";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaRequestMetadataSnapshot {
    /// `0` only ever appears on a `Default::default()` value or an empty stored document; a
    /// snapshot that was really captured always carries
    /// [`MEDIA_REQUEST_SNAPSHOT_SCHEMA_VERSION`]. [`MediaRequestMetadataSnapshot::parse`] uses
    /// that to tell "no snapshot" from "a snapshot with nothing in it".
    pub schema_version: u32,
    pub captured_at: Option<DateTime<Utc>>,
    /// Where the facts came from — `smg_titles`, `smg_movie`, `smg_series` — or, when `partial`
    /// is set and nothing could be captured, why they are absent (`enrichment_failed`,
    /// `unparseable`, …). There is no third place to record that, and dropping it would leave an
    /// unexplained empty snapshot, which is exactly what FR-030 forbids.
    pub source: Option<String>,
    /// True when enrichment did not fully succeed; `missing` names the groups that are
    /// unavailable (`content_ratings`, `genres`, `mdblist`, `ratings`, `awards`, `all`).
    pub partial: bool,
    pub missing: Vec<String>,
    pub genres: Vec<String>,
    pub canonical_tags: Vec<CanonicalMediaTag>,
    pub content_ratings: Vec<ContentRating>,
    pub mdblist: Option<MdblistSummary>,
    pub ratings: TitleRatingSummary,
    pub tmdb_vote_average: Option<f64>,
    pub tmdb_vote_count: Option<i64>,
    pub popularity: Option<f64>,
    pub runtime_minutes: Option<i32>,
    pub original_language: Option<String>,
    pub country: Option<String>,
    pub network: Option<String>,
    pub studio: Option<String>,
    pub content_status: Option<String>,
    /// Movies: `tmdb_release_date`.
    pub release_date: Option<String>,
    /// Series.
    pub first_aired: Option<String>,
    pub awards: Vec<TitleAward>,
    /// True when any canonical tag is flagged adult.
    pub is_adult: bool,
}

/// The literal a `to_json` failure falls back to. It is a valid, parseable, explicitly partial
/// snapshot, so a serialization problem degrades to "metadata unavailable ⇒ manual review"
/// instead of writing a broken document into a NOT NULL column.
const UNSERIALIZABLE_SNAPSHOT_JSON: &str =
    r#"{"schema_version":1,"partial":true,"missing":["all"],"source":"unserializable"}"#;

fn non_empty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn positive_runtime(minutes: i32) -> Option<i32> {
    (minutes > 0).then_some(minutes)
}

fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}

impl MediaRequestMetadataSnapshot {
    pub fn from_movie(movie: &MovieMetadata, captured_at: DateTime<Utc>) -> Self {
        Self {
            schema_version: MEDIA_REQUEST_SNAPSHOT_SCHEMA_VERSION,
            captured_at: Some(captured_at),
            source: Some("smg_movie".to_string()),
            partial: false,
            missing: Vec::new(),
            genres: movie.genres.clone(),
            canonical_tags: movie.canonical_tags.clone(),
            content_ratings: movie.content_ratings.clone(),
            mdblist: movie.mdblist.clone(),
            ratings: movie.ratings.clone(),
            tmdb_vote_average: finite(movie.tmdb_vote_average),
            tmdb_vote_count: movie.tmdb_vote_count,
            popularity: finite(movie.popularity),
            runtime_minutes: positive_runtime(movie.runtime_minutes),
            original_language: movie.original_language.clone().and_then(non_empty),
            // A movie has no broadcast country or network in SMG's shape; leaving these `None`
            // keeps "not applicable to this facet" distinct from "empty string".
            country: None,
            network: None,
            studio: non_empty(movie.studio.clone()),
            content_status: non_empty(movie.content_status.clone()),
            release_date: movie.tmdb_release_date.clone().and_then(non_empty),
            first_aired: None,
            awards: movie.awards.clone(),
            is_adult: movie.canonical_tags.iter().any(|tag| tag.is_adult),
        }
    }

    pub fn from_series(series: &SeriesMetadata, captured_at: DateTime<Utc>) -> Self {
        Self {
            schema_version: MEDIA_REQUEST_SNAPSHOT_SCHEMA_VERSION,
            captured_at: Some(captured_at),
            source: Some("smg_series".to_string()),
            partial: false,
            missing: Vec::new(),
            genres: series.genres.clone(),
            canonical_tags: series.canonical_tags.clone(),
            content_ratings: series.content_ratings.clone(),
            mdblist: series.mdblist.clone(),
            ratings: series.ratings.clone(),
            // SMG publishes TMDB vote aggregates and popularity on movies only.
            tmdb_vote_average: None,
            tmdb_vote_count: None,
            popularity: None,
            runtime_minutes: positive_runtime(series.runtime_minutes),
            original_language: series.original_language.clone().and_then(non_empty),
            country: non_empty(series.country.clone()),
            network: non_empty(series.network.clone()),
            studio: None,
            content_status: non_empty(series.content_status.clone()),
            release_date: None,
            first_aired: non_empty(series.first_aired.clone()),
            awards: series.awards.clone(),
            is_adult: series.canonical_tags.iter().any(|tag| tag.is_adult),
        }
    }

    /// Nothing could be captured. `reason` is recorded on `source` so the trace says *why* the
    /// snapshot is empty rather than leaving an approver guessing.
    pub fn unavailable(reason: &str) -> Self {
        Self {
            schema_version: MEDIA_REQUEST_SNAPSHOT_SCHEMA_VERSION,
            source: Some(reason.to_string()),
            partial: true,
            missing: vec![SNAPSHOT_GROUP_ALL.to_string()],
            ..Self::default()
        }
    }

    /// Whether `group` (or everything) is unavailable in this snapshot.
    pub fn is_missing(&self, group: &str) -> bool {
        self.missing
            .iter()
            .any(|entry| entry == group || entry == SNAPSHOT_GROUP_ALL)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|error| {
            tracing::warn!(
                error = %error,
                "media request metadata snapshot could not be serialized; storing an explicitly \
                 partial snapshot instead"
            );
            UNSERIALIZABLE_SNAPSHOT_JSON.to_string()
        })
    }

    /// Read a stored document. Never fails: an absent, empty, or unreadable snapshot is a real
    /// state of the world for rows written before this feature (or by a build whose document this
    /// one cannot understand), and it must read back as *unavailable*, never as "no facts", which
    /// a rule would happily treat as an answer.
    pub fn parse(json: &str) -> Self {
        let trimmed = json.trim();
        if trimmed.is_empty() {
            return Self::unavailable("absent");
        }
        match serde_json::from_str::<Self>(trimmed) {
            Ok(snapshot) if snapshot.schema_version > 0 => snapshot,
            Ok(_) => Self::unavailable("absent"),
            Err(_) => Self::unavailable("unparseable"),
        }
    }
}

/// `MediaRequest` is a domain type and the snapshot is an application concern, so the accessor
/// lives here rather than widening the domain with a serde-parsing method.
pub trait MediaRequestMetadataSnapshotExt {
    /// Parse the stored snapshot. Parsing happens on each call; callers that need it more than
    /// once in a hot path should bind it.
    fn metadata_snapshot(&self) -> MediaRequestMetadataSnapshot;
}

impl MediaRequestMetadataSnapshotExt for scryer_domain::MediaRequest {
    fn metadata_snapshot(&self) -> MediaRequestMetadataSnapshot {
        MediaRequestMetadataSnapshot::parse(&self.metadata_snapshot_json)
    }
}
