//! Fact snapshots for maintenance rule evaluation (RFC 137 section 8).
//!
//! Every fact the matcher can read is built here, from ports Scryer already
//! has. A fact Scryer cannot resolve becomes [`Observation::unknown`] with a
//! stable code rather than a plausible-looking default: an unknown fact holds
//! the rule (`unknown` beats `match` in the decoder), which is the fail-closed
//! behaviour the RFC demands. Guessing `false`, `0`, or "the newest file" would
//! silently authorize a destructive action on evidence Scryer never had.

use chrono::{DateTime, Utc};
use scryer_domain::{MediaFacet, Title};
use scryer_rules::maintenance::{
    MAINTENANCE_INPUT_SCHEMA_VERSION, MaintenanceFactsDoc, MaintenanceFileDoc, MaintenanceInput,
    MaintenanceLibraryDoc, MaintenanceSubjectDoc, MaintenanceSubjectKind, Observation,
};

use crate::types::TitleMediaFile;

/// Structured tag prefix carrying a title's quality profile. Kept in sync with
/// [`crate::ports::TitleRepository::count_by_quality_profile_id`], which
/// defines the trim-after-strip resolver semantics reused here.
pub(crate) const QUALITY_PROFILE_TAG_PREFIX: &str = "scryer:quality-profile:";

/// Stable reason codes for facts this wave cannot observe.
///
/// These are part of the rule-authoring contract: a matcher that tests
/// `input.facts.<fact>.reason` compares against exactly these strings.
pub mod unknown_reason {
    /// Scryer does not record this signal yet. Applies to `last_upgraded_at`
    /// (no distinct upgrade event exists — the newest file timestamp is a
    /// different fact and must not stand in for it), `active_downloads`
    /// (download-state wiring is out of scope for the foundation wave), and the
    /// three episode counts for series-facet titles (only per-title episode
    /// queries exist, so a preview over 50 titles would fan out).
    pub const NOT_YET_COLLECTED: &str = "not_yet_collected";

    /// The title has files, but none of their rows carry a parseable
    /// timestamp, so the first-import instant is genuinely unavailable.
    pub const FILE_TIMESTAMPS_UNAVAILABLE: &str = "file_timestamps_unavailable";
}

/// Everything the builder needs about one library, resolved once per run.
#[derive(Clone, Debug)]
pub struct MaintenanceLibraryRef {
    pub id: String,
    pub name: String,
}

/// Build the title-scoped input document for one title.
///
/// `files` must be this title's files only; callers batch-load for the whole
/// selection and group by `title_id` rather than querying per title.
pub fn build_title_input(
    evaluation_time: DateTime<Utc>,
    title: &Title,
    library: &MaintenanceLibraryRef,
    files: &[TitleMediaFile],
) -> MaintenanceInput {
    MaintenanceInput {
        schema_version: MAINTENANCE_INPUT_SCHEMA_VERSION,
        evaluation_time,
        subject: MaintenanceSubjectDoc {
            kind: MaintenanceSubjectKind::Title,
            title_id: title.id.clone(),
            season_number: None,
            episode_id: None,
            facet: title.facet.as_str().to_string(),
            name: title.name.clone(),
            year: title.year,
        },
        library: MaintenanceLibraryDoc {
            id: library.id.clone(),
            name: library.name.clone(),
        },
        facts: build_facts(title, files),
    }
}

fn build_facts(title: &Title, files: &[TitleMediaFile]) -> MaintenanceFactsDoc {
    let file_docs: Vec<MaintenanceFileDoc> = files.iter().map(file_doc).collect();
    let total_size: i64 = files.iter().map(|file| file.size_bytes).sum();

    MaintenanceFactsDoc {
        monitored: Observation::known(title.monitored),
        tags: Observation::known(title.tags.clone()),
        quality_profile_id: quality_profile_observation(&title.tags),
        added_at: Observation::known(title.created_at.to_rfc3339()),
        first_imported_at: first_imported_observation(files),
        // Scryer records no distinct upgrade event. Approximating it with the
        // newest file timestamp would make "upgraded" and "imported" the same
        // fact, so rules asking about upgrades are held instead.
        last_upgraded_at: Observation::unknown(unknown_reason::NOT_YET_COLLECTED),
        // An empty file set is a confirmed answer, not a missing one.
        has_file: Observation::known(!files.is_empty()),
        file_count: Observation::known(files.len() as i64),
        total_file_size_bytes: Observation::known(total_size),
        files: Observation::known(file_docs),
        episode_count: episode_count_observation(&title.facet),
        episode_file_count: episode_count_observation(&title.facet),
        monitored_episode_count: episode_count_observation(&title.facet),
        active_downloads: Observation::unknown(unknown_reason::NOT_YET_COLLECTED),
    }
}

/// Known when the structured tag is present and non-empty; absent when the
/// title carries no such tag, which is a confirmed "no profile assigned"
/// rather than a lookup Scryer failed to perform.
fn quality_profile_observation(tags: &[String]) -> Observation<String> {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(QUALITY_PROFILE_TAG_PREFIX))
        .map(str::trim)
        .filter(|profile_id| !profile_id.is_empty())
        .map_or_else(Observation::absent, |profile_id| {
            Observation::known(profile_id.to_string())
        })
}

/// Earliest file timestamp. Absent when the title has no files at all; unknown
/// only when files exist but carry no readable timestamp.
fn first_imported_observation(files: &[TitleMediaFile]) -> Observation<String> {
    if files.is_empty() {
        return Observation::absent();
    }

    files
        .iter()
        .filter_map(|file| parse_timestamp(&file.created_at))
        .min()
        .map_or_else(
            || Observation::unknown(unknown_reason::FILE_TIMESTAMPS_UNAVAILABLE),
            |earliest| Observation::known(earliest.to_rfc3339()),
        )
}

/// Absent for movies — a movie has no episodes, and that is a fact, not a gap.
/// Unknown for series and anime until an episode-count port exists that a
/// batched preview can afford.
fn episode_count_observation(facet: &MediaFacet) -> Observation<i64> {
    match facet {
        MediaFacet::Movie => Observation::absent(),
        MediaFacet::Series | MediaFacet::Anime => {
            Observation::unknown(unknown_reason::NOT_YET_COLLECTED)
        }
    }
}

/// Fields the media-file row does not carry stay `None`; the file document
/// declares them optional, so a rule reading one gets undefined rather than a
/// fabricated value.
fn file_doc(file: &TitleMediaFile) -> MaintenanceFileDoc {
    MaintenanceFileDoc {
        size_bytes: Some(file.size_bytes),
        quality: file.quality_label.clone(),
        video_codec: file
            .video_codec
            .or(file.video_codec_parsed)
            .map(|codec| codec.as_str().to_string()),
        video_width: file.video_width,
        video_height: file.video_height,
        audio_languages: file.audio_languages.clone(),
        subtitle_languages: file.subtitle_languages.clone(),
        added_at: parse_timestamp(&file.created_at).map(|added| added.to_rfc3339()),
    }
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|value| value.with_timezone(&Utc))
}
