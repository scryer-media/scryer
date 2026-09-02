//! In-memory, title-less indexer search jobs (spec 0002).
//!
//! The lifecycle mirrors [`super::interactive_release_search`] — an owner-scoped
//! in-memory registry with TTL eviction, one task per indexer, results merged
//! into the snapshot as each indexer answers — but the subject is an operator's
//! raw query rather than a catalog title. There is therefore no subject
//! resolution, no scoring, no candidate token and no title-relative rejection:
//! what the operator sees is the indexers' own catalogue, annotated with the
//! facets and the context-free refusals the server can state without a title.
//!
//! A separate module rather than an enum on the interactive job (D3): the older
//! job's body is title-bound end to end, so sharing its types would turn every
//! field into an `Option`.

use super::*;

use crate::acquisition::seed_goals::{meets_minimum_seeders, seeders_from_extra};
use crate::helpers::{HashDomain, blake3_identity_hex};
use crate::quality_profile::evaluate_against_profile_for_category;
use crate::user_rule_input::{ReleaseRuntimeInfo, RuleContextInfo, build_rule_input};
use scryer_logging::{ActorContext, LogContext, ResourceContext, WorkflowContext, context_span};
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, info, warn};

/// Overall deadline for a job; stragglers past this are marked failed.
const INDEXER_SEARCH_DEADLINE: std::time::Duration = scryer_outbound_http::INDEXER_HTTP_TIMEOUT;
/// Terminal jobs are evicted this long after completion (D3).
const COMPLETED_JOB_TTL_MINUTES: i64 = 30;
/// Defensive cap: running jobs older than this are cancelled and evicted.
const RUNNING_JOB_TTL_MINUTES: i64 = 10;
/// Per-actor cap on concurrently running jobs.
const MAX_RUNNING_JOBS_PER_ACTOR: usize = 8;
/// Hard cap on releases held by one job, whatever the indexers return (D15).
const MAX_RELEASES_PER_JOB: usize = 5_000;
/// Per-indexer result limit when the request does not name one.
pub const DEFAULT_INDEXER_SEARCH_PER_INDEXER_LIMIT: i32 = 100;
/// Largest per-indexer result limit an operator may ask for.
pub const MAX_INDEXER_SEARCH_PER_INDEXER_LIMIT: i32 = 250;

// ── Request model ───────────────────────────────────────────────────────────

/// What the operator is searching for. The kind picks the search facet, the
/// id-search facet and the default newznab categories; `Raw` picks none of them
/// and sends the query as plain text (D2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexerSearchKind {
    Movie,
    Series,
    Anime,
    Raw,
}

impl IndexerSearchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Anime => "anime",
            Self::Raw => "raw",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            "anime" => Some(Self::Anime),
            "raw" => Some(Self::Raw),
            _ => None,
        }
    }

    /// The media facet this kind searches as, or `None` for a raw text search.
    pub fn facet(self) -> Option<MediaFacet> {
        match self {
            Self::Movie => Some(MediaFacet::Movie),
            Self::Series => Some(MediaFacet::Series),
            Self::Anime => Some(MediaFacet::Anime),
            Self::Raw => None,
        }
    }

    /// Newznab categories applied when the operator names none.
    fn default_categories(self) -> Vec<String> {
        match self.facet() {
            Some(facet) => crate::settings::keys::default_indexer_routing_categories_for_scope(
                facet.as_str(),
            ),
            None => Vec::new(),
        }
    }
}

/// A title-less search request.
///
/// The snapshot echoes this struct back **after** validation, so the optional
/// fields hold effective values there: `categories` is the resolved category
/// set and `per_indexer_limit` the resolved limit. `indexer_ids` keeps its
/// requested meaning — `None` or empty is "every eligible indexer".
#[derive(Clone, Debug, Default)]
pub struct IndexerSearchRequest {
    pub query: String,
    pub kind: IndexerSearchKind,
    pub indexer_ids: Option<Vec<String>>,
    pub categories: Option<Vec<String>>,
    pub min_size_bytes: Option<i64>,
    pub max_size_bytes: Option<i64>,
    pub min_seeders: Option<i32>,
    pub max_age_days: Option<i32>,
    pub per_indexer_limit: Option<i32>,
}

impl Default for IndexerSearchKind {
    fn default() -> Self {
        Self::Raw
    }
}

impl IndexerSearchRequest {
    /// Reject unusable input and resolve every defaulted field, so the runner
    /// and the snapshot echo work from the same effective request.
    fn validate(mut self) -> AppResult<Self> {
        self.query = self.query.trim().to_string();
        if self.query.is_empty() {
            return Err(AppError::Validation("search query is required".to_string()));
        }

        let limit = self
            .per_indexer_limit
            .unwrap_or(DEFAULT_INDEXER_SEARCH_PER_INDEXER_LIMIT);
        if !(1..=MAX_INDEXER_SEARCH_PER_INDEXER_LIMIT).contains(&limit) {
            return Err(AppError::Validation(format!(
                "per-indexer limit must be between 1 and {MAX_INDEXER_SEARCH_PER_INDEXER_LIMIT}"
            )));
        }
        self.per_indexer_limit = Some(limit);

        if let (Some(min), Some(max)) = (self.min_size_bytes, self.max_size_bytes)
            && min > max
        {
            return Err(AppError::Validation(
                "minimum size must not exceed maximum size".to_string(),
            ));
        }
        if self.min_size_bytes.is_some_and(|value| value < 0)
            || self.max_size_bytes.is_some_and(|value| value < 0)
        {
            return Err(AppError::Validation(
                "size limits must not be negative".to_string(),
            ));
        }
        if self.min_seeders.is_some_and(|value| value < 0) {
            return Err(AppError::Validation(
                "minimum seeders must not be negative".to_string(),
            ));
        }
        if self.max_age_days.is_some_and(|value| value < 1) {
            return Err(AppError::Validation(
                "maximum age must be at least one day".to_string(),
            ));
        }

        let categories = self
            .categories
            .take()
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|values| !values.is_empty())
            .unwrap_or_else(|| self.kind.default_categories());
        self.categories = Some(categories);

        self.indexer_ids = self.indexer_ids.take().map(|ids| {
            let mut ids = ids
                .into_iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            ids
        });

        Ok(self)
    }

    fn effective_categories(&self) -> Option<Vec<String>> {
        self.categories
            .as_ref()
            .filter(|values| !values.is_empty())
            .cloned()
    }

    fn effective_per_indexer_limit(&self) -> usize {
        self.per_indexer_limit
            .unwrap_or(DEFAULT_INDEXER_SEARCH_PER_INDEXER_LIMIT)
            .max(1) as usize
    }
}

// ── Snapshot model ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexerSearchState {
    Running,
    Completed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexerSearchIndexerStatus {
    Pending,
    Searching,
    Ok,
    Failed,
    Skipped,
}

/// Per-indexer progress and timing inside one job.
#[derive(Clone, Debug)]
pub struct IndexerSearchIndexerView {
    pub indexer_id: String,
    pub name: String,
    /// Routing priority for this indexer, `0` when routing states none.
    pub priority: i64,
    pub status: IndexerSearchIndexerStatus,
    /// Results this indexer contributed before the advanced filters ran.
    pub result_count: usize,
    pub started_at: Option<DateTime<Utc>>,
    pub elapsed_ms: Option<i64>,
    /// Short, stable failure word (`timeout`, `auth`, `rate limited`, …).
    pub failure_reason: Option<String>,
}

/// The facet values one release carries; the client filters on these locally.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexerSearchFacetValues {
    pub protocol: String,
    pub resolution: String,
    pub source: String,
    pub audio_hdr: Vec<String>,
    pub flags: Vec<String>,
}

/// One merged release, plus the full server-held search result the grab path
/// resolves download details from (D4: URLs never reach the client).
#[derive(Clone, Debug)]
pub struct IndexerSearchRelease {
    pub id: String,
    pub indexer_id: String,
    pub indexer_name: String,
    pub indexer_priority: i64,
    pub title: String,
    pub protocol: String,
    pub size_bytes: Option<i64>,
    pub published_at: Option<String>,
    pub category_label: Option<String>,
    pub file_summary: String,
    pub release_group: Option<String>,
    pub seeders: Option<i64>,
    pub leechers: Option<i64>,
    pub grabs: Option<i64>,
    /// Badge list: resolution, source, audio/HDR and flag labels in that order.
    pub flags: Vec<String>,
    pub facet_values: IndexerSearchFacetValues,
    /// Context-free refusals (D6). Advisory: the grab path re-evaluates against
    /// the real target.
    pub rejections: Vec<String>,
    pub info_url: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub is_season_pack: bool,
    pub result: IndexerSearchResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexerSearchFacetItem {
    pub value: String,
    pub label: String,
    pub count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexerSearchFacet {
    pub key: String,
    pub label: String,
    pub items: Vec<IndexerSearchFacetItem>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IndexerSearchTotals {
    /// Releases that survived the advanced filters.
    pub matched: usize,
    pub indexers_queried: usize,
    pub indexers_responded: usize,
    /// Whether the per-job release cap dropped results.
    pub truncated: bool,
}

/// Point-in-time snapshot of a title-less indexer search job.
#[derive(Clone, Debug)]
pub struct IndexerSearchSnapshot {
    pub id: String,
    pub state: IndexerSearchState,
    /// The effective request, echoed back.
    pub request: IndexerSearchRequest,
    pub totals: IndexerSearchTotals,
    pub indexers: Vec<IndexerSearchIndexerView>,
    pub facets: Vec<IndexerSearchFacet>,
    pub releases: Vec<IndexerSearchRelease>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

pub(crate) struct IndexerSearchJobEntry {
    pub(crate) snapshot: IndexerSearchSnapshot,
    pub(crate) actor_id: String,
    pub(crate) cancel: CancellationToken,
}

/// Evict stale registry entries: terminal jobs past their TTL, plus (defensive)
/// running jobs older than the running TTL, whose tokens are cancelled first.
fn evict_stale_entries(entries: &mut HashMap<String, IndexerSearchJobEntry>, now: DateTime<Utc>) {
    entries.retain(|_, entry| match entry.snapshot.state {
        IndexerSearchState::Running => {
            if now - entry.snapshot.started_at > Duration::minutes(RUNNING_JOB_TTL_MINUTES) {
                entry.cancel.cancel();
                false
            } else {
                true
            }
        }
        _ => entry.snapshot.completed_at.is_some_and(|completed| {
            now - completed <= Duration::minutes(COMPLETED_JOB_TTL_MINUTES)
        }),
    });
}

// ── Dispatch resolution ─────────────────────────────────────────────────────

/// The indexers a job will query, with everything the runner needs to build a
/// single-indexer restricted routing plan for each of them.
struct IndexerSearchDispatch {
    views: Vec<IndexerSearchIndexerView>,
    dispatch: Vec<String>,
    /// One entry per enabled indexer, so a restriction can disable the rest.
    /// Indexers absent from a routing plan are searched by default, which is
    /// why the base map must cover them all.
    routing_base: HashMap<String, IndexerRoutingEntry>,
}

/// Everything one runner pass needs, resolved once before it spawns.
struct IndexerSearchJobContext {
    job_id: String,
    actor: User,
    request: IndexerSearchRequest,
    dispatch: Vec<String>,
    routing_base: HashMap<String, IndexerRoutingEntry>,
    /// Indexer id → (name, priority), for merged releases and the health line.
    indexers: HashMap<String, (String, i64)>,
    judge: Arc<ReleaseJudge>,
    cancel: CancellationToken,
}

/// Restrict a routing plan to one indexer: every other entry is disabled, which
/// is how [`crate::catalog::discovery`] expresses the same restriction on the
/// only routing surface the multi-indexer client reads.
fn restrict_routing_to_indexer(
    base: &HashMap<String, IndexerRoutingEntry>,
    indexer_id: &str,
) -> IndexerRoutingPlan {
    IndexerRoutingPlan {
        entries: base
            .iter()
            .map(|(id, entry)| {
                (
                    id.clone(),
                    IndexerRoutingEntry {
                        enabled: id == indexer_id,
                        categories: entry.categories.clone(),
                        priority: entry.priority,
                    },
                )
            })
            .collect(),
    }
}

// ── Context-free rejections (D6) ────────────────────────────────────────────

/// The profile and rules a release is judged against when no title is in play.
///
/// `profile` is `None` for [`IndexerSearchKind::Raw`]: without a facet there is
/// no default profile to speak for, so only the seeder floor applies.
struct ReleaseJudge {
    profile: Option<QualityProfile>,
    weights: ScoringWeights,
    rules: scryer_rules::UserRulesEngine,
    /// Facet string the profile and the rules are scoped to; empty for Raw.
    category: String,
}

impl ReleaseJudge {
    fn rejections(
        &self,
        parsed: &ParsedReleaseMetadata,
        result: &IndexerSearchResult,
    ) -> Vec<String> {
        let Some(profile) = self.profile.as_ref() else {
            return Vec::new();
        };
        // `has_existing_file: false` — there is no incumbent to compare with, so
        // a rejection that only exists relative to a title (cutoff, upgrade)
        // can never fire here.
        let decision = evaluate_against_profile_for_category(
            profile,
            parsed,
            false,
            &self.weights,
            Some(self.category.as_str()),
        );
        let mut reasons = decision
            .block_codes
            .iter()
            .map(|code| profile_block_reason(code, parsed))
            .collect::<Vec<_>>();

        if !self.rules.is_empty() {
            let input = build_rule_input(
                parsed,
                profile,
                &decision,
                ReleaseRuntimeInfo {
                    size_bytes: result.size_bytes,
                    published_at: result.published_at.as_deref(),
                    thumbs_up: result.thumbs_up,
                    thumbs_down: result.thumbs_down,
                    is_password_protected: result
                        .password_hint
                        .as_ref()
                        .map(|hint| !hint.trim().is_empty()),
                    extra: Some(&result.extra),
                    indexer_languages: result.indexer_languages.as_deref(),
                },
                RuleContextInfo {
                    // An empty title context: this surface has no title, no
                    // library and no incumbent, and says so rather than
                    // borrowing one (D6).
                    title_id: None,
                    library_name: None,
                    category: Some(self.category.as_str()),
                    original_language: None,
                    original_country: None,
                    title_tags: &[],
                    has_existing_file: false,
                    existing_score: None,
                    search_mode: "raw",
                    runtime_minutes: None,
                    is_filler: false,
                },
                None,
            );
            let mut evaluator = self.rules.evaluator();
            match evaluator.evaluate(&input, self.category.as_str()) {
                Ok(evaluated) => {
                    for entry in evaluated.entries {
                        if entry.delta == crate::BLOCK_SCORE {
                            reasons.push(entry.rule_set_name);
                        }
                    }
                }
                Err(error) => {
                    warn!(error = %error, "indexer search: user rule evaluation failed");
                }
            }
        }

        reasons.dedup();
        reasons
    }
}

/// Human wording for a profile block code. Unmapped codes degrade to the code
/// with underscores spaced out rather than being hidden.
fn profile_block_reason(code: &str, parsed: &ParsedReleaseMetadata) -> String {
    match code {
        "source_low_quality_theatrical"
        | "source_in_profile_blocklist"
        | "source_not_in_profile_allowlist" => match parsed.source {
            Some(source) => format!("banned source: {}", source.as_str().to_ascii_lowercase()),
            None => "banned source".to_string(),
        },
        "source_missing_and_profile_requires_source" => "source not recognized".to_string(),
        "quality_not_in_profile_tiers" => match parsed.quality.as_deref() {
            Some(quality) => format!("quality not in profile: {quality}"),
            None => "quality not in profile".to_string(),
        },
        "quality_missing_and_profile_disallows_unknown" => "unknown quality".to_string(),
        "video_codec_in_profile_blocklist" | "video_codec_not_in_profile_allowlist" => {
            "banned video codec".to_string()
        }
        "audio_codec_in_profile_blocklist" | "audio_codec_not_in_profile_allowlist" => {
            "banned audio codec".to_string()
        }
        "dolby_vision_not_allowed" => "Dolby Vision not allowed".to_string(),
        "dolby_vision_missing_hdr_fallback" => "Dolby Vision without HDR fallback".to_string(),
        "hdr_not_allowed" => "HDR not allowed".to_string(),
        "bd_disk_not_allowed" => "full disc not allowed".to_string(),
        "required_audio_language_missing" => "required audio language missing".to_string(),
        other => other.replace('_', " "),
    }
}

// ── Facet derivation (D5) ───────────────────────────────────────────────────

fn protocol_of(result: &IndexerSearchResult) -> String {
    match result.source_kind {
        Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => "usenet".to_string(),
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
            "torrent".to_string()
        }
        None => result
            .extra
            .get("protocol")
            .and_then(serde_json::Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

fn resolution_of(parsed: &ParsedReleaseMetadata) -> String {
    let Some(quality) = parsed.quality.as_deref() else {
        return "Unknown".to_string();
    };
    let quality = quality.to_ascii_lowercase();
    if quality.contains("2160") || quality.contains("4k") || quality.contains("uhd") {
        "2160p"
    } else if quality.contains("1080") {
        "1080p"
    } else if quality.contains("720") {
        "720p"
    } else {
        "SD"
    }
    .to_string()
}

fn source_of(parsed: &ParsedReleaseMetadata) -> String {
    if parsed.is_remux {
        return "REMUX".to_string();
    }
    match parsed.source {
        Some(ReleaseSource::BluRay | ReleaseSource::BrDisk) => "BluRay",
        Some(ReleaseSource::WebDl) => "WEB-DL",
        Some(ReleaseSource::WebRip) => "WEBRip",
        Some(ReleaseSource::Hdtv) => "HDTV",
        _ => "Other",
    }
    .to_string()
}

fn audio_hdr_of(parsed: &ParsedReleaseMetadata) -> Vec<String> {
    let mut values = Vec::new();
    if parsed.is_atmos {
        values.push("Atmos".to_string());
    }
    if parsed.is_dolby_vision {
        values.push("Dolby Vision".to_string());
    }
    if parsed.detected_hdr || parsed.is_hdr10plus || parsed.is_hlg {
        values.push("HDR".to_string());
    }
    values
}

/// Whether the indexer tagged this release with `flag` (case-insensitive) in
/// its `indexer_flags` list.
fn has_indexer_flag(result: &IndexerSearchResult, flag: &str) -> bool {
    result
        .extra
        .get("indexer_flags")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|values| {
            values.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case(flag))
            })
        })
}

fn flags_of(result: &IndexerSearchResult, parsed: &ParsedReleaseMetadata) -> Vec<String> {
    let mut flags = Vec::new();
    let freeleech = has_indexer_flag(result, "freeleech")
        || result
            .extra
            .get("download_volume_factor")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|factor| factor <= 0.0);
    if freeleech {
        flags.push("Freeleech".to_string());
    }
    if parsed.is_proper_upload || parsed.is_repack {
        flags.push("Proper/Repack".to_string());
    }
    if has_indexer_flag(result, "internal") {
        flags.push("Internal".to_string());
    }
    if has_indexer_flag(result, "scene") {
        flags.push("Scene".to_string());
    }
    flags
}

/// `"1 NZB · 240 segments"` / `"1 file · 3 trackers"`, degrading to the bare
/// container when the indexer reported no detail.
fn file_summary_of(result: &IndexerSearchResult, protocol: &str) -> String {
    let count = |key: &str| {
        result.extra.get(key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_array().map(|items| items.len() as i64))
                .filter(|count| *count > 0)
        })
    };
    if protocol == "torrent" {
        match count("trackers") {
            Some(trackers) => format!("1 file · {trackers} trackers"),
            None => "1 torrent".to_string(),
        }
    } else {
        match count("segments") {
            Some(segments) => format!("1 NZB · {segments} segments"),
            None => "1 NZB".to_string(),
        }
    }
}

fn is_numeric_category(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
}

/// `"Movies/HD (2040)"` when the indexer stated both a name and an id, else
/// whichever one it stated.
fn category_label_of(result: &IndexerSearchResult) -> Option<String> {
    let categories = &result.response_attributes.categories;
    let name = categories
        .iter()
        .find(|value| !is_numeric_category(value))
        .map(String::as_str);
    let id = categories
        .iter()
        .find(|value| is_numeric_category(value))
        .map(String::as_str);
    match (name, id) {
        (Some(name), Some(id)) => Some(format!("{name} ({id})")),
        (Some(name), None) => Some(name.to_string()),
        (None, Some(id)) => Some(id.to_string()),
        (None, None) => None,
    }
}

/// Job-scoped release identity (D4). Short, stable, and derived only from
/// server-held facts, so a grab can name a release without the client ever
/// seeing its download URL.
fn release_id_for(result: &IndexerSearchResult, indexer_id: &str) -> String {
    let locator = result
        .guid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            result
                .download_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default();
    let digest = blake3_identity_hex(
        HashDomain::IndexerSearchRelease,
        format!("{indexer_id}|{locator}|{}", result.title),
    );
    digest[..16].to_string()
}

/// Age in days, or `None` when the indexer stated no usable publish time.
///
/// Both encodings are accepted deliberately: the *nab plugins pass `pubDate`
/// through as RFC 2822, while other sources normalize to RFC 3339, and an age
/// filter that silently never fires for one of them is worse than no filter.
fn published_age_days(published_at: Option<&str>, now: DateTime<Utc>) -> Option<i64> {
    let value = published_at?.trim();
    let parsed = DateTime::parse_from_rfc3339(value)
        .or_else(|_| DateTime::parse_from_rfc2822(value))
        .ok()?;
    Some((now - parsed.with_timezone(&Utc)).num_days())
}

fn facet_counts(releases: &[IndexerSearchRelease]) -> Vec<IndexerSearchFacet> {
    fn tally(
        key: &str,
        label: &str,
        releases: &[IndexerSearchRelease],
        values: impl Fn(&IndexerSearchRelease) -> Vec<String>,
    ) -> IndexerSearchFacet {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for release in releases {
            for value in values(release) {
                *counts.entry(value).or_default() += 1;
            }
        }
        let mut items = counts
            .into_iter()
            .map(|(value, count)| IndexerSearchFacetItem {
                label: value.clone(),
                value,
                count,
            })
            .collect::<Vec<_>>();
        // Most-populated first, then alphabetical, so the rail is stable across
        // polls of the same result set.
        items.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.value.cmp(&right.value))
        });
        IndexerSearchFacet {
            key: key.to_string(),
            label: label.to_string(),
            items,
        }
    }

    vec![
        tally("protocol", "Protocol", releases, |release| {
            vec![release.facet_values.protocol.clone()]
        }),
        tally("indexer", "Indexer", releases, |release| {
            vec![release.indexer_name.clone()]
        }),
        tally("resolution", "Resolution", releases, |release| {
            vec![release.facet_values.resolution.clone()]
        }),
        tally("source", "Source", releases, |release| {
            vec![release.facet_values.source.clone()]
        }),
        tally("audio_hdr", "Audio & HDR", releases, |release| {
            release.facet_values.audio_hdr.clone()
        }),
        tally("flags", "Flags", releases, |release| {
            release.facet_values.flags.clone()
        }),
    ]
}

// ── Failure wording ─────────────────────────────────────────────────────────

/// A short, stable word for a failed indexer. The health line shows this, so it
/// must stay comparable across indexers rather than echo provider prose.
fn failure_word(error: &AppError) -> String {
    let text = error.to_string().to_ascii_lowercase();
    if text.contains("timed out") || text.contains("timeout") {
        return "timeout".to_string();
    }
    if text.contains("rate limit") || text.contains("request limit") || text.contains("429") {
        return "rate limited".to_string();
    }
    if text.contains("unauthorized")
        || text.contains("forbidden")
        || text.contains("api key")
        || text.contains("apikey")
        || text.contains("401")
        || text.contains("403")
    {
        return "auth".to_string();
    }
    if let Some(status) = http_status_in(&text) {
        return format!("http {status}");
    }
    "error".to_string()
}

/// First 4xx/5xx-looking status code in an error message, if any.
fn http_status_in(text: &str) -> Option<u16> {
    text.split(|ch: char| !ch.is_ascii_digit())
        .filter(|token| token.len() == 3)
        .filter_map(|token| token.parse::<u16>().ok())
        .find(|status| (400..600).contains(status))
}

/// How a per-indexer outcome inside a successful response reads on the health
/// line. `None` means the indexer answered completely.
fn outcome_status(
    outcome: IndexerSearchOutcome,
) -> Option<(IndexerSearchIndexerStatus, &'static str)> {
    match outcome {
        IndexerSearchOutcome::Complete { .. } => None,
        // An unattested legacy response is operationally fine; it only withholds
        // convergence coverage, which this surface does not record.
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::Unattested),
            ..
        } => None,
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::RateLimited),
            ..
        } => Some((IndexerSearchIndexerStatus::Failed, "rate limited")),
        IndexerSearchOutcome::Partial {
            reason: Some(IndexerSearchIncompleteReason::UpstreamFailure),
            ..
        } => Some((IndexerSearchIndexerStatus::Failed, "upstream failure")),
        IndexerSearchOutcome::Partial { .. } => {
            Some((IndexerSearchIndexerStatus::Failed, "partial"))
        }
        IndexerSearchOutcome::Deferred { .. } => {
            Some((IndexerSearchIndexerStatus::Skipped, "deferred"))
        }
        IndexerSearchOutcome::Skipped { .. } => {
            Some((IndexerSearchIndexerStatus::Skipped, "skipped"))
        }
        IndexerSearchOutcome::Errored => Some((IndexerSearchIndexerStatus::Failed, "error")),
    }
}

// ── Application API ─────────────────────────────────────────────────────────

impl AppUseCase {
    /// Start a title-less indexer search and return its first snapshot.
    ///
    /// The job is registered before the runner spawns, so the returned id is
    /// immediately pollable.
    pub async fn start_indexer_search(
        &self,
        actor: &User,
        request: IndexerSearchRequest,
    ) -> AppResult<IndexerSearchSnapshot> {
        // The Indexers page's own gate (D13): this surface reaches every
        // configured indexer regardless of library.
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let request = request.validate()?;

        let dispatch = self.resolve_indexer_search_dispatch(&request, None).await?;
        let judge = Arc::new(self.build_release_judge(request.kind).await);
        let now = self.runtime.environment.now();
        let job_id = Id::new().0;
        let cancel = CancellationToken::new();
        let indexers = dispatch
            .views
            .iter()
            .map(|view| {
                (
                    view.indexer_id.clone(),
                    (view.name.clone(), view.priority),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut snapshot = IndexerSearchSnapshot {
            id: job_id.clone(),
            state: IndexerSearchState::Running,
            request: request.clone(),
            totals: IndexerSearchTotals::default(),
            indexers: dispatch.views,
            facets: facet_counts(&[]),
            releases: Vec::new(),
            started_at: now,
            completed_at: None,
        };
        recount_totals(&mut snapshot);

        {
            let mut registry = self.runtime.acquisition.indexer_searches.lock().await;
            evict_stale_entries(&mut registry, now);
            let running_for_actor = registry
                .values()
                .filter(|entry| {
                    entry.actor_id == actor.id
                        && entry.snapshot.state == IndexerSearchState::Running
                })
                .count();
            if running_for_actor >= MAX_RUNNING_JOBS_PER_ACTOR {
                return Err(AppError::Validation(
                    "too many concurrent indexer searches".to_string(),
                ));
            }
            registry.insert(
                job_id.clone(),
                IndexerSearchJobEntry {
                    snapshot: snapshot.clone(),
                    actor_id: actor.id.clone(),
                    cancel: cancel.clone(),
                },
            );
        }

        self.spawn_indexer_search_runner(IndexerSearchJobContext {
            job_id,
            actor: actor.clone(),
            request,
            dispatch: dispatch.dispatch,
            routing_base: dispatch.routing_base,
            indexers,
            judge,
            cancel,
        });

        Ok(snapshot)
    }

    /// Poll a job snapshot. `None` for unknown, evicted, or another actor's job
    /// (no information leak).
    pub async fn indexer_search(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<IndexerSearchSnapshot>> {
        let mut registry = self.runtime.acquisition.indexer_searches.lock().await;
        evict_stale_entries(&mut registry, self.runtime.environment.now());
        Ok(registry
            .get(id)
            .filter(|entry| entry.actor_id == actor.id)
            .map(|entry| entry.snapshot.clone()))
    }

    /// Re-dispatch only the indexers that failed, inside the same job (D9).
    ///
    /// Results already merged are kept; the retried indexers' releases dedupe on
    /// release id, so a healthy indexer's rows are never duplicated.
    pub async fn retry_indexer_search(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<IndexerSearchSnapshot> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let now = self.runtime.environment.now();
        let (request, failed) = {
            let mut registry = self.runtime.acquisition.indexer_searches.lock().await;
            evict_stale_entries(&mut registry, now);
            let entry = registry
                .get(id)
                .filter(|entry| entry.actor_id == actor.id)
                .ok_or_else(|| AppError::NotFound(format!("indexer search {id}")))?;
            if entry.snapshot.state == IndexerSearchState::Running {
                return Err(AppError::Validation(
                    "indexer search is still running".to_string(),
                ));
            }
            let failed = entry
                .snapshot
                .indexers
                .iter()
                .filter(|view| view.status == IndexerSearchIndexerStatus::Failed)
                .map(|view| view.indexer_id.clone())
                .collect::<Vec<_>>();
            (entry.snapshot.request.clone(), failed)
        };

        if failed.is_empty() {
            // Nothing to heal; the caller sees the job exactly as it stands.
            return self
                .indexer_search(actor, id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("indexer search {id}")));
        }

        let only = failed.iter().cloned().collect::<HashSet<_>>();
        let dispatch = self
            .resolve_indexer_search_dispatch(&request, Some(&only))
            .await?;
        let judge = Arc::new(self.build_release_judge(request.kind).await);
        let cancel = CancellationToken::new();

        let snapshot = {
            let mut registry = self.runtime.acquisition.indexer_searches.lock().await;
            let Some(entry) = registry.get_mut(id) else {
                return Err(AppError::NotFound(format!("indexer search {id}")));
            };
            for view in dispatch.views {
                if let Some(existing) = entry
                    .snapshot
                    .indexers
                    .iter_mut()
                    .find(|candidate| candidate.indexer_id == view.indexer_id)
                {
                    *existing = view;
                }
            }
            entry.snapshot.state = IndexerSearchState::Running;
            entry.snapshot.completed_at = None;
            entry.cancel = cancel.clone();
            recount_totals(&mut entry.snapshot);
            entry.snapshot.clone()
        };

        let indexers = snapshot
            .indexers
            .iter()
            .map(|view| (view.indexer_id.clone(), (view.name.clone(), view.priority)))
            .collect::<HashMap<_, _>>();
        self.spawn_indexer_search_runner(IndexerSearchJobContext {
            job_id: id.to_string(),
            actor: actor.clone(),
            request,
            dispatch: dispatch.dispatch,
            routing_base: dispatch.routing_base,
            indexers,
            judge,
            cancel,
        });

        Ok(snapshot)
    }

    /// Cancel a running job. `false` (not an error) when the job is unknown,
    /// foreign, or already finished.
    pub async fn cancel_indexer_search(&self, actor: &User, id: &str) -> AppResult<bool> {
        let now = self.runtime.environment.now();
        let token = {
            let mut registry = self.runtime.acquisition.indexer_searches.lock().await;
            evict_stale_entries(&mut registry, now);
            let Some(entry) = registry.get_mut(id) else {
                return Ok(false);
            };
            if entry.actor_id != actor.id || entry.snapshot.state != IndexerSearchState::Running {
                return Ok(false);
            }
            entry.snapshot.state = IndexerSearchState::Cancelled;
            entry.snapshot.completed_at = Some(now);
            entry.cancel.clone()
        };
        token.cancel();
        Ok(true)
    }

    /// Config-visible eligibility only, mirroring the interactive job: enabled +
    /// interactive-enabled, minus routing-disabled, minus config cooldown.
    async fn resolve_indexer_search_dispatch(
        &self,
        request: &IndexerSearchRequest,
        only: Option<&HashSet<String>>,
    ) -> AppResult<IndexerSearchDispatch> {
        let scope_id = request.kind.facet().map(|facet| facet.as_str().to_string());
        let routing = self
            .resolve_indexer_routing(None, scope_id.as_deref())
            .await;
        let priority_by_name = self.build_indexer_priority_by_name(routing.as_ref()).await;

        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        if let Some(requested) = request.indexer_ids.as_ref().filter(|ids| !ids.is_empty()) {
            let known = configs
                .iter()
                .map(|config| config.id.as_str())
                .collect::<HashSet<_>>();
            if let Some(unknown) = requested.iter().find(|id| !known.contains(id.as_str())) {
                return Err(AppError::Validation(format!("unknown indexer {unknown}")));
            }
        }

        // A live system backoff makes the search client skip the indexer
        // silently — its restricted call just returns empty. Reading it here
        // keeps the health line honest instead of reporting "ok, 0 results"
        // for an indexer that was never queried.
        let system_backoffs = self
            .services
            .integrations
            .indexer_configs
            .list_system_backoffs()
            .await
            .unwrap_or_default();

        let now = self.runtime.environment.now();
        let mut views = Vec::new();
        let mut dispatch = Vec::new();
        let mut routing_base = HashMap::new();
        for config in configs {
            if !config.is_enabled {
                continue;
            }
            routing_base.insert(
                config.id.clone(),
                routing
                    .as_ref()
                    .and_then(|plan| plan.entries.get(&config.id))
                    .cloned()
                    .unwrap_or(IndexerRoutingEntry {
                        enabled: true,
                        categories: Vec::new(),
                        priority: 0,
                    }),
            );
            if !config.enable_interactive_search {
                continue;
            }
            if routing
                .as_ref()
                .and_then(|plan| plan.entries.get(&config.id))
                .is_some_and(|entry| !entry.enabled)
            {
                continue;
            }
            if request
                .indexer_ids
                .as_ref()
                .is_some_and(|ids| !ids.is_empty() && !ids.contains(&config.id))
            {
                continue;
            }
            if only.is_some_and(|only| !only.contains(&config.id)) {
                continue;
            }

            let priority = priority_by_name
                .get(config.name.as_str())
                .copied()
                .unwrap_or(0);
            let mut view = IndexerSearchIndexerView {
                indexer_id: config.id.clone(),
                name: config.name.clone(),
                priority,
                status: IndexerSearchIndexerStatus::Pending,
                result_count: 0,
                started_at: None,
                elapsed_ms: None,
                failure_reason: None,
            };
            if config.disabled_until.is_some_and(|until| until > now) {
                view.status = IndexerSearchIndexerStatus::Skipped;
                view.failure_reason = Some("temporarily disabled".to_string());
            } else if system_backoffs
                .get(&config.id)
                .is_some_and(|backoff| backoff.disabled_until > now)
            {
                view.status = IndexerSearchIndexerStatus::Skipped;
                view.failure_reason = Some("temporarily backed off".to_string());
            } else {
                dispatch.push(config.id.clone());
            }
            views.push(view);
        }

        Ok(IndexerSearchDispatch {
            views,
            dispatch,
            routing_base,
        })
    }

    /// Resolve the facet's default quality profile plus the rules engine, which
    /// together are everything a context-free rejection can be based on (D6).
    async fn build_release_judge(&self, kind: IndexerSearchKind) -> ReleaseJudge {
        let default_weights = || {
            crate::build_weights_for_category(
                &ScoringPersona::default(),
                &crate::ScoringOverrides::default(),
                None,
            )
        };
        let Some(facet) = kind.facet() else {
            return ReleaseJudge {
                profile: None,
                weights: default_weights(),
                rules: scryer_rules::UserRulesEngine::empty(),
                category: String::new(),
            };
        };
        let category = facet.as_str().to_string();
        let lookup = crate::app_usecase_discovery::QualityProfileLookup {
            title_tags: &[],
            library_id: None,
            imdb_id: None,
            tvdb_id: None,
            category_hint: Some(category.as_str()),
        };
        // Best effort: an unresolvable profile means the pane shows releases
        // without profile rejections, never a failed search.
        let profile = match self.resolve_quality_profile(lookup).await {
            Ok(profile) => Some(profile),
            Err(error) => {
                warn!(
                    error = %error,
                    facet = category.as_str(),
                    "indexer search: default quality profile unresolved; rejections limited to seeders"
                );
                None
            }
        };
        let persona = self
            .resolve_scoring_persona(None, Some(category.as_str()))
            .await
            .unwrap_or_default();
        let weights = profile.as_ref().map_or_else(default_weights, |profile| {
            crate::build_weights_for_category(
                &persona,
                &profile.criteria.scoring_overrides,
                Some(category.as_str()),
            )
        });
        ReleaseJudge {
            profile,
            weights,
            rules: self.user_rules_engine_snapshot(),
            category,
        }
    }

    fn spawn_indexer_search_runner(&self, context: IndexerSearchJobContext) {
        let log_span = context_span(
            LogContext::workflow(WorkflowContext {
                kind: "indexer_search".to_owned(),
                id: context.job_id.clone(),
            })
            .with_actor(ActorContext {
                kind: if context.actor.is_system_execution_actor() {
                    "system".to_owned()
                } else {
                    "user".to_owned()
                },
                id: Some(context.actor.id.clone()),
                display_name: Some(context.actor.username.clone()),
                source: None,
            })
            .with_resource(ResourceContext {
                job_id: Some(context.job_id.clone()),
                ..ResourceContext::default()
            }),
        );
        log_span.in_scope(|| {
            info!(
                actor = context.actor.id.as_str(),
                job_id = context.job_id.as_str(),
                query = context.request.query.as_str(),
                kind = context.request.kind.as_str(),
                indexers = context.dispatch.len(),
                "starting indexer search"
            );
        });
        let app = self.clone();
        tokio::spawn(
            async move {
                app.run_indexer_search_job(context).await;
            }
            .instrument(log_span),
        );
    }

    async fn run_indexer_search_job(&self, context: IndexerSearchJobContext) {
        let IndexerSearchJobContext {
            job_id,
            // The actor is carried for the log span the caller already opened.
            actor: _,
            request,
            dispatch,
            routing_base,
            indexers,
            judge,
            cancel,
        } = context;

        let facet = request.kind.facet().map(|facet| facet.as_str().to_string());
        let newznab_categories = request.effective_categories();

        let mut set = JoinSet::new();
        for indexer_id in dispatch {
            let app = self.clone();
            let job_id = job_id.clone();
            let query = request.query.clone();
            let facet = facet.clone();
            let newznab_categories = newznab_categories.clone();
            let plan = restrict_routing_to_indexer(&routing_base, &indexer_id);
            let child_token = cancel.child_token();
            set.spawn(async move {
                let started_at = app.runtime.environment.now();
                app.set_indexer_search_status(
                    &job_id,
                    &indexer_id,
                    IndexerSearchIndexerStatus::Searching,
                    None,
                    Some(started_at),
                    None,
                )
                .await;
                let began = std::time::Instant::now();
                let outcome = app
                    .services
                    .integrations
                    .indexer_client
                    .search(
                        query,
                        HashMap::new(),
                        None,
                        // The id-search facet mirrors the search facet: this
                        // surface never carries external ids.
                        facet.clone(),
                        facet,
                        newznab_categories,
                        Some(plan),
                        SearchMode::Interactive,
                        IndexerErrorOperation::InteractiveSearch,
                        None,
                        None,
                        None,
                        None,
                        Vec::new(),
                        None,
                        child_token,
                    )
                    .await;
                let elapsed_ms = i64::try_from(began.elapsed().as_millis()).unwrap_or(i64::MAX);
                (indexer_id, elapsed_ms, outcome)
            });
        }

        let drain = async {
            while let Some(joined) = set.join_next().await {
                let (indexer_id, elapsed_ms, outcome) = match joined {
                    Ok(joined) => joined,
                    Err(error) => {
                        warn!(
                            job_id = job_id.as_str(),
                            error = %error,
                            "indexer search task panicked"
                        );
                        continue;
                    }
                };
                let (name, priority) = indexers
                    .get(&indexer_id)
                    .cloned()
                    .unwrap_or_else(|| (indexer_id.clone(), 0));
                match outcome {
                    Ok(response) => {
                        let downgrade = response
                            .indexer_outcomes
                            .iter()
                            .find(|outcome| outcome.indexer_id == indexer_id)
                            .and_then(|outcome| outcome_status(outcome.outcome));
                        self.merge_indexer_search_batch(
                            &job_id,
                            &indexer_id,
                            &name,
                            priority,
                            response.results,
                            &request,
                            &judge,
                            elapsed_ms,
                        )
                        .await;
                        if let Some((status, reason)) = downgrade {
                            self.set_indexer_search_status(
                                &job_id,
                                &indexer_id,
                                status,
                                Some(reason.to_string()),
                                None,
                                Some(elapsed_ms),
                            )
                            .await;
                        }
                    }
                    Err(error) if error.is_canceled() => {
                        // The job is being cancelled; leave the status as-is.
                    }
                    Err(error) => {
                        self.set_indexer_search_status(
                            &job_id,
                            &indexer_id,
                            IndexerSearchIndexerStatus::Failed,
                            Some(failure_word(&error)),
                            None,
                            Some(elapsed_ms),
                        )
                        .await;
                    }
                }
            }
        };
        let timed_out = tokio::time::timeout(INDEXER_SEARCH_DEADLINE, drain)
            .await
            .is_err();
        if timed_out {
            cancel.cancel();
            set.abort_all();
        }

        let now = self.runtime.environment.now();
        let mut registry = self.runtime.acquisition.indexer_searches.lock().await;
        let Some(entry) = registry.get_mut(&job_id) else {
            return;
        };
        if timed_out {
            for indexer in entry.snapshot.indexers.iter_mut() {
                if matches!(
                    indexer.status,
                    IndexerSearchIndexerStatus::Pending | IndexerSearchIndexerStatus::Searching
                ) {
                    indexer.status = IndexerSearchIndexerStatus::Failed;
                    indexer.failure_reason = Some("timeout".to_string());
                }
            }
        }
        if entry.snapshot.state == IndexerSearchState::Running {
            entry.snapshot.state = IndexerSearchState::Completed;
            entry.snapshot.completed_at = Some(now);
        }
        recount_totals(&mut entry.snapshot);
    }

    async fn set_indexer_search_status(
        &self,
        job_id: &str,
        indexer_id: &str,
        status: IndexerSearchIndexerStatus,
        failure_reason: Option<String>,
        started_at: Option<DateTime<Utc>>,
        elapsed_ms: Option<i64>,
    ) {
        let mut registry = self.runtime.acquisition.indexer_searches.lock().await;
        let Some(entry) = registry.get_mut(job_id) else {
            return;
        };
        if entry.snapshot.state != IndexerSearchState::Running {
            return;
        }
        if let Some(indexer) = entry
            .snapshot
            .indexers
            .iter_mut()
            .find(|indexer| indexer.indexer_id == indexer_id)
        {
            indexer.status = status;
            indexer.failure_reason = failure_reason;
            if started_at.is_some() {
                indexer.started_at = started_at;
            }
            if elapsed_ms.is_some() {
                indexer.elapsed_ms = elapsed_ms;
            }
        }
        recount_totals(&mut entry.snapshot);
    }

    /// Merge one indexer's batch into the job snapshot: truncate to the
    /// per-indexer limit, apply the advanced filters, derive facets and
    /// rejections, then recompute the facet counts over the whole merged set.
    #[expect(
        clippy::too_many_arguments,
        reason = "the merge carries the indexer identity, the effective request and the judge"
    )]
    async fn merge_indexer_search_batch(
        &self,
        job_id: &str,
        indexer_id: &str,
        indexer_name: &str,
        priority: i64,
        batch: Vec<IndexerSearchResult>,
        request: &IndexerSearchRequest,
        judge: &ReleaseJudge,
        elapsed_ms: i64,
    ) {
        let mut batch = batch;
        batch.truncate(request.effective_per_indexer_limit());
        let batch_len = batch.len();

        // The indexer's own configured floor, resolved once for the batch.
        let seeder_thresholds = self.minimum_seeders_for_candidates(&batch).await;
        let now = self.runtime.environment.now();
        let mut built = Vec::with_capacity(batch.len());
        for mut result in batch {
            if result
                .indexer_id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                result.indexer_id = Some(indexer_id.to_string());
            }
            let parsed = result
                .parsed_release_metadata
                .clone()
                .unwrap_or_else(|| crate::parse_release_metadata(&result.title));
            let seeders = seeders_from_extra(&result.extra);

            if let Some(min) = request.min_size_bytes
                && result.size_bytes.is_some_and(|size| size < min)
            {
                continue;
            }
            if let Some(max) = request.max_size_bytes
                && result.size_bytes.is_some_and(|size| size > max)
            {
                continue;
            }
            if let Some(min) = request.min_seeders
                && seeders.is_some_and(|count| count < i64::from(min))
            {
                continue;
            }
            if let Some(max_age) = request.max_age_days
                && published_age_days(result.published_at.as_deref(), now)
                    .is_some_and(|age| age > i64::from(max_age))
            {
                continue;
            }

            let protocol = protocol_of(&result);
            let facet_values = IndexerSearchFacetValues {
                protocol: protocol.clone(),
                resolution: resolution_of(&parsed),
                source: source_of(&parsed),
                audio_hdr: audio_hdr_of(&parsed),
                flags: flags_of(&result, &parsed),
            };
            let mut flags = vec![
                facet_values.resolution.clone(),
                facet_values.source.clone(),
            ];
            flags.extend(facet_values.audio_hdr.iter().cloned());
            flags.extend(facet_values.flags.iter().cloned());

            let mut rejections = judge.rejections(&parsed, &result);
            let threshold = seeder_thresholds
                .get(indexer_id)
                .copied()
                .unwrap_or_default();
            if !meets_minimum_seeders(result.source_kind, Some(indexer_id), seeders, threshold) {
                rejections.push("below minimum seeders".to_string());
            }

            built.push(IndexerSearchRelease {
                id: release_id_for(&result, indexer_id),
                indexer_id: indexer_id.to_string(),
                indexer_name: indexer_name.to_string(),
                indexer_priority: priority,
                title: result.title.clone(),
                protocol,
                size_bytes: result.size_bytes,
                published_at: result.published_at.clone(),
                category_label: category_label_of(&result),
                file_summary: file_summary_of(&result, &facet_values.protocol),
                release_group: parsed.release_group.clone(),
                seeders,
                leechers: result
                    .extra
                    .get("leechers")
                    .or_else(|| result.extra.get("peers"))
                    .and_then(serde_json::Value::as_i64),
                grabs: result.indexer_grabs,
                flags,
                facet_values,
                rejections,
                info_url: result.info_url.clone(),
                season: parsed.episode.as_ref().and_then(|episode| episode.season),
                episode: parsed
                    .episode
                    .as_ref()
                    .and_then(|episode| episode.episode_numbers.first().copied()),
                is_season_pack: parsed
                    .episode
                    .as_ref()
                    .is_some_and(|episode| episode.full_season || episode.is_series_pack),
                result,
            });
        }

        let mut registry = self.runtime.acquisition.indexer_searches.lock().await;
        let Some(entry) = registry.get_mut(job_id) else {
            return;
        };
        if entry.snapshot.state != IndexerSearchState::Running {
            return;
        }
        let mut seen = entry
            .snapshot
            .releases
            .iter()
            .map(|release| release.id.clone())
            .collect::<HashSet<_>>();
        for release in built {
            if entry.snapshot.releases.len() >= MAX_RELEASES_PER_JOB {
                entry.snapshot.totals.truncated = true;
                break;
            }
            if seen.insert(release.id.clone()) {
                entry.snapshot.releases.push(release);
            }
        }
        if let Some(indexer) = entry
            .snapshot
            .indexers
            .iter_mut()
            .find(|indexer| indexer.indexer_id == indexer_id)
        {
            indexer.status = IndexerSearchIndexerStatus::Ok;
            indexer.result_count = batch_len;
            indexer.elapsed_ms = Some(elapsed_ms);
            indexer.failure_reason = None;
        }
        entry.snapshot.facets = facet_counts(&entry.snapshot.releases);
        recount_totals(&mut entry.snapshot);
    }
}

/// Totals are always re-derived from the views and releases, so a retry that
/// re-dispatches one indexer never shrinks "indexers queried" to one.
fn recount_totals(snapshot: &mut IndexerSearchSnapshot) {
    snapshot.totals.matched = snapshot.releases.len();
    snapshot.totals.indexers_queried = snapshot
        .indexers
        .iter()
        .filter(|indexer| indexer.status != IndexerSearchIndexerStatus::Skipped)
        .count();
    snapshot.totals.indexers_responded = snapshot
        .indexers
        .iter()
        .filter(|indexer| indexer.status == IndexerSearchIndexerStatus::Ok)
        .count();
}
