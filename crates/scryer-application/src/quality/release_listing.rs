//! The listing facts about a release that user rules read, frozen at the
//! moment Scryer evaluated the release for grab.
//!
//! Scoring a release in any lane (grab, import, upgrade) must see the same
//! listing inputs, and a live listing can change or vanish after the grab. So
//! the values are captured once from the search result, persisted with the
//! submission, copied to the media row, and read back from this snapshot; they
//! are never re-read from an indexer.
//!
//! A snapshot never holds a secret. The indexer `extra` map can carry an
//! archive password, magnet and comment links with a tracker passkey, and
//! other key- or token-bearing values; those entries are dropped both when a
//! snapshot is captured and when a stored one is read, because the snapshot
//! is persisted on several rows and shown to anyone who can view the library.
//! The `password_protected` flag stays: it is a boolean fact, not a secret.
//!
//! Everything here is pure: capture, bounding and age computation take the
//! current time as an argument and never read the clock.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Serialization format version written into every persisted snapshot.
const SNAPSHOT_FORMAT_VERSION: u64 = 1;

/// Most keys a snapshot keeps from the indexer's `extra` map.
const MAX_EXTRA_KEYS: usize = 64;

/// Largest compact-JSON size, in bytes, of the kept `extra` map.
const MAX_EXTRA_SERIALIZED_BYTES: usize = 8 * 1024;

/// Most entries a snapshot keeps from the indexer's language list.
const MAX_INDEXER_LANGUAGES: usize = 32;

/// Longest single indexer language entry, in bytes after trimming.
const MAX_INDEXER_LANGUAGE_BYTES: usize = 64;

/// Listing facts about a release as the indexer reported them at the moment
/// Scryer evaluated it for grab. Frozen: captured once, persisted with the
/// submission, copied to the media row, never re-read from a live listing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReleaseListingSnapshot {
    /// Publish time as the indexer gave it (trimmed). Only kept when it parses.
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub thumbs_up: Option<i32>,
    #[serde(default)]
    pub thumbs_down: Option<i32>,
    #[serde(default)]
    pub is_password_protected: Option<bool>,
    /// Empty when the indexer reported none. Bounded by
    /// [`bounded_indexer_languages`].
    #[serde(default, deserialize_with = "null_as_default")]
    pub indexer_languages: Vec<String>,
    /// Indexer-specific scalars, bounded by [`bounded_extra`].
    #[serde(default, deserialize_with = "null_as_default")]
    pub extra: BTreeMap<String, Value>,
    pub captured_at: DateTime<Utc>,
}

/// Reads JSON `null` as the type's default, so one null collection does not
/// make the whole stored snapshot unreadable.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// On-the-wire shape: the format version first, then the snapshot's fields in
/// declaration order.
#[derive(Serialize)]
struct VersionedSnapshotRef<'a> {
    v: u64,
    #[serde(flatten)]
    snapshot: &'a ReleaseListingSnapshot,
}

impl ReleaseListingSnapshot {
    /// Capture the listing facts from a search result.
    pub(crate) fn capture_from_search_result(
        result: &crate::IndexerSearchResult,
        now: DateTime<Utc>,
    ) -> Self {
        let is_password_protected = result
            .extra
            .get("password_protected")
            .and_then(Value::as_bool)
            .or_else(|| password_protection_hint(result.password_hint.as_deref()));
        Self {
            published_at: parseable_published_at(result.published_at.as_deref()),
            thumbs_up: result.thumbs_up,
            thumbs_down: result.thumbs_down,
            is_password_protected,
            indexer_languages: bounded_indexer_languages(result.indexer_languages.iter().flatten()),
            extra: bounded_extra(&result.extra),
            captured_at: now,
        }
    }

    /// The persisted form of [`Self::capture_from_search_result`], shaped for
    /// the `release_listing_json` column of a submission or pending row.
    pub(crate) fn capture_json_from_search_result(
        result: &crate::IndexerSearchResult,
        now: DateTime<Utc>,
    ) -> Option<String> {
        Some(Self::capture_from_search_result(result, now).to_json_string())
    }

    /// The snapshot a search or RSS candidate is scored with, at the lane's
    /// `now`. A candidate that already carries a snapshot (a replayed pending
    /// row, or a result scored before) keeps those frozen facts; a fresh
    /// listing is captured. Either way `captured_at` is `now`, the instant its
    /// age was measured at, so a grab that persists this snapshot anchors
    /// every later read on the same instant the grab scored.
    pub(crate) fn for_scoring(result: &crate::IndexerSearchResult, now: DateTime<Utc>) -> Self {
        match result
            .release_listing_json
            .as_deref()
            .and_then(Self::from_json_str)
        {
            Some(frozen) => frozen.stamped_at(now),
            None => Self::capture_from_search_result(result, now),
        }
    }

    /// The persisted snapshot a grab or park of a search or RSS candidate
    /// carries: the one its scoring pass attached, so what is stored is what
    /// was scored. A candidate no scoring pass has seen is captured at `now`.
    pub(crate) fn json_for_candidate(
        result: &crate::IndexerSearchResult,
        now: DateTime<Utc>,
    ) -> Option<String> {
        result
            .release_listing_json
            .clone()
            .or_else(|| Self::capture_json_from_search_result(result, now))
    }

    /// The snapshot a grab from a persisted pending row carries. A grab is the
    /// instant age is anchored at from then on, so the row's frozen facts are
    /// re-stamped with `captured_at = now`. A row without a readable snapshot
    /// (written before snapshots were stored) gets a best-effort capture.
    pub(crate) fn json_for_pending_release(
        release: &crate::PendingRelease,
        now: DateTime<Utc>,
    ) -> Option<String> {
        let snapshot = release
            .release_listing_json
            .as_deref()
            .and_then(Self::from_json_str)
            .map(|frozen| frozen.stamped_at(now))
            .unwrap_or_else(|| Self::capture_from_pending_release(release, now));
        Some(snapshot.to_json_string())
    }

    /// The same listing facts, captured at `at`.
    fn stamped_at(self, at: DateTime<Utc>) -> Self {
        Self {
            captured_at: at,
            ..self
        }
    }

    /// Best-effort snapshot for a pending release that has no persisted one.
    /// A pending row keeps only its publish time and its normalized
    /// `source_password`. This is a literal secret; indexer protection flags
    /// have already been handled at the provider boundary. A nonblank value
    /// implies protection, including values such as `1` and `false`.
    pub(crate) fn capture_from_pending_release(
        release: &crate::PendingRelease,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            published_at: parseable_published_at(release.published_at.as_deref()),
            thumbs_up: None,
            thumbs_down: None,
            is_password_protected: password_protection_hint(release.source_password.as_deref()),
            indexer_languages: Vec::new(),
            extra: BTreeMap::new(),
            captured_at: now,
        }
    }

    /// Whole days from publish to `anchor`, clamped at zero. `None` when the
    /// publish time is unknown. The caller chooses the anchor; this never reads
    /// the clock.
    pub(crate) fn age_days(&self, anchor: DateTime<Utc>) -> Option<i64> {
        let published = crate::quality_profile::parse_published_at(self.published_at.as_deref()?)?;
        Some((anchor - published).num_days().max(0))
    }

    /// Compact JSON carrying a `"v"` format tag.
    pub(crate) fn to_json_string(&self) -> String {
        serde_json::to_string(&VersionedSnapshotRef {
            v: SNAPSHOT_FORMAT_VERSION,
            snapshot: self,
        })
        .expect("a release listing snapshot always serializes")
    }

    /// Read a snapshot written by [`Self::to_json_string`]. Unknown fields are
    /// ignored, a missing `"v"` is read as the current version, and a `null`
    /// `indexer_languages` or `extra` reads as empty. The stored `extra` and
    /// `indexer_languages` are bounded again with [`bounded_extra`] and
    /// [`bounded_indexer_languages`], so a row from any writer cannot carry
    /// nested values or an oversized map or list into rule input. Anything
    /// unreadable — malformed JSON, a missing `captured_at`, a mistyped field or
    /// a version this build does not know — yields `None`.
    pub(crate) fn from_json_str(raw: &str) -> Option<Self> {
        let Value::Object(mut object) = serde_json::from_str::<Value>(raw).ok()? else {
            return None;
        };
        match object.remove("v") {
            None => {}
            Some(version) if version.as_u64() == Some(SNAPSHOT_FORMAT_VERSION) => {}
            Some(_) => return None,
        }
        let mut snapshot: Self = serde_json::from_value(Value::Object(object)).ok()?;
        let stored = std::mem::take(&mut snapshot.extra);
        snapshot.extra = bound_extra_entries(stored);
        snapshot.indexer_languages = bounded_indexer_languages(&snapshot.indexer_languages);
        Some(snapshot)
    }
}

/// The read-only display form of a persisted listing snapshot, for surfaces
/// that show what a grab saw.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseListingView {
    /// Publish time the indexer reported, or `None` when it was unknown.
    pub published_at: Option<DateTime<Utc>>,
    /// Whole days from publish to capture, clamped at zero; `None` when the
    /// publish time was unknown.
    pub age_days_at_grab: Option<i64>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub is_password_protected: Option<bool>,
    pub indexer_languages: Vec<String>,
    /// Bounded indexer-specific scalars, keyed in ascending order.
    pub extra: BTreeMap<String, Value>,
    pub captured_at: DateTime<Utc>,
}

/// Read a persisted listing snapshot for display. Anything
/// [`ReleaseListingSnapshot::from_json_str`] cannot read yields `None`.
/// The listing facts of a search result in their stored form, for a store that
/// replays the result later in place of a new indexer query. Secret-bearing
/// extras are already dropped, as in every snapshot.
pub fn search_result_listing_json(
    result: &crate::IndexerSearchResult,
    now: DateTime<Utc>,
) -> Option<String> {
    ReleaseListingSnapshot::json_for_candidate(result, now)
}

pub fn release_listing_view(raw: &str) -> Option<ReleaseListingView> {
    let snapshot = ReleaseListingSnapshot::from_json_str(raw)?;
    Some(ReleaseListingView {
        published_at: snapshot
            .published_at
            .as_deref()
            .and_then(crate::quality_profile::parse_published_at),
        age_days_at_grab: snapshot.age_days(snapshot.captured_at),
        thumbs_up: snapshot.thumbs_up,
        thumbs_down: snapshot.thumbs_down,
        is_password_protected: snapshot.is_password_protected,
        indexer_languages: snapshot.indexer_languages,
        extra: snapshot.extra,
        captured_at: snapshot.captured_at,
    })
}

/// When a persisted snapshot was captured, for tests that compare it with a
/// fresh capture at the lane's own timestamp.
#[cfg(test)]
pub(crate) fn captured_at_of(json: Option<&str>) -> DateTime<Utc> {
    ReleaseListingSnapshot::from_json_str(json.expect("a listing snapshot was persisted"))
        .expect("the persisted listing snapshot is readable")
        .captured_at
}

/// Password hints carry literal secrets. Boolean protection flags live in
/// their own explicitly typed indexer field.
fn password_protection_hint(raw: Option<&str>) -> Option<bool> {
    raw.filter(|value| !value.trim().is_empty()).map(|_| true)
}

fn parseable_published_at(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    crate::quality_profile::parse_published_at(trimmed).map(|_| trimmed.to_string())
}

fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn is_bounded_value(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().all(is_scalar),
        other => is_scalar(other),
    }
}

/// Compact-JSON length of one `"key":value` map entry.
fn serialized_entry_len(key: &str, value: &Value) -> usize {
    let key_len = serde_json::to_string(key).map_or(0, |encoded| encoded.len());
    let value_len = serde_json::to_string(value).map_or(0, |encoded| encoded.len());
    key_len + 1 + value_len
}

/// Compact-JSON length of a map whose entries have the given lengths.
fn serialized_map_len(entry_lens_total: usize, entries: usize) -> usize {
    2 + entry_lens_total + entries.saturating_sub(1)
}

/// Drop every entry that could carry a credential or a credential-bearing
/// link (see [`is_secret_extra_entry`]: the exact keys in
/// [`SECRET_EXACT_KEYS`], any key containing a [`SECRET_KEY_FRAGMENTS`]
/// fragment, a URL-valued `guid`, and a `password_protected` that is not a
/// boolean), keep only scalars and all-scalar arrays from the rest, then
/// apply two caps:
///
/// - at most [`MAX_EXTRA_KEYS`] keys: over it, the lexicographically last keys
///   are dropped first. Key order carries no meaning about value size, so the
///   short scalar arrays rules read (tags, flags) are not singled out;
/// - at most [`MAX_EXTRA_SERIALIZED_BYTES`] of compact JSON: over it, the
///   entries with the longest serialized `"key":value` are dropped first, ties
///   by ascending key.
///
/// Scryer's own RSS replay markers (`_rss_` keys) are not indexer facts and
/// never enter a snapshot.
///
/// The secret rule also runs when a stored snapshot is read back, so a row
/// written before it existed never serves a secret either.
///
/// Deterministic regardless of the input map's iteration order, and
/// idempotent: a bounded map is returned unchanged.
pub(crate) fn bounded_extra(raw: &HashMap<String, Value>) -> BTreeMap<String, Value> {
    bound_extra_entries(
        raw.iter()
            .filter(|(key, _)| !key.starts_with(RSS_REPLAY_MARKER_PREFIX))
            .map(|(key, value)| (key.clone(), value.clone())),
    )
}

const RSS_REPLAY_MARKER_PREFIX: &str = "_rss_";

/// Bound an indexer language list: each entry trimmed, blanks and entries
/// longer than [`MAX_INDEXER_LANGUAGE_BYTES`] dropped, repeats dropped after
/// the first, original order kept, at most [`MAX_INDEXER_LANGUAGES`] entries.
/// Idempotent: a bounded list is returned unchanged.
pub(crate) fn bounded_indexer_languages<S: AsRef<str>>(
    languages: impl IntoIterator<Item = S>,
) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for language in languages {
        if kept.len() >= MAX_INDEXER_LANGUAGES {
            break;
        }
        let trimmed = language.as_ref().trim();
        if trimmed.is_empty()
            || trimmed.len() > MAX_INDEXER_LANGUAGE_BYTES
            || kept.iter().any(|existing| existing == trimmed)
        {
            continue;
        }
        kept.push(trimmed.to_string());
    }
    kept
}

/// Keys dropped by exact name because the value is, or can be, a credential
/// or a link that embeds one:
///
/// - `password`: the newznab plugins store the real archive password here;
/// - `magnet_url`, `magnet_uri`: a private tracker's magnet carries its
///   passkey in the `tr=` announce URL;
/// - `comment_url`, `download_url`, `link`: indexer links that can carry an
///   API key or passkey in the query string.
///
/// Most of these are also caught by [`SECRET_KEY_FRAGMENTS`]; they are named
/// here so the known offenders stay dropped whatever the fragment list says.
const SECRET_EXACT_KEYS: &[&str] = &[
    "password",
    "magnet_url",
    "magnet_uri",
    "comment_url",
    "download_url",
    "link",
];

/// A key whose lowercased form contains any of these is dropped: it names a
/// link or a credential, and a rule has no business reading either. Matching
/// is by substring, so it fails closed on unrelated keys that happen to
/// contain a fragment (an `author` key, for instance).
const SECRET_KEY_FRAGMENTS: &[&str] = &[
    "url", "uri", "apikey", "api_key", "passkey", "token", "secret", "password", "cookie", "auth",
];

/// The one password-named key that is a fact, not a secret: the indexer's
/// protected flag. Kept only while its value is a boolean.
const PASSWORD_PROTECTED_FLAG_KEY: &str = "password_protected";

/// Whether an `extra` entry could carry a credential or a credential-bearing
/// link, and so must never be stored in, or read back from, a snapshot. The
/// snapshot is persisted on several rows and served to every viewer of the
/// library, so a secret in it would leak far past the indexer settings.
fn is_secret_extra_entry(key: &str, value: &Value) -> bool {
    let lowered = key.to_ascii_lowercase();
    if lowered == PASSWORD_PROTECTED_FLAG_KEY {
        return !value.is_boolean();
    }
    if SECRET_EXACT_KEYS.contains(&lowered.as_str()) {
        return true;
    }
    if SECRET_KEY_FRAGMENTS
        .iter()
        .any(|fragment| lowered.contains(fragment))
    {
        return true;
    }
    // A `guid` is usually an opaque id, but some indexers use the item's
    // download or details URL, which can embed a key.
    lowered == "guid" && value.as_str().is_some_and(is_url_like)
}

fn is_url_like(raw: &str) -> bool {
    let lowered = raw.trim().to_ascii_lowercase();
    lowered.contains("://") || lowered.starts_with("magnet:")
}

fn bound_extra_entries(
    entries: impl IntoIterator<Item = (String, Value)>,
) -> BTreeMap<String, Value> {
    let mut seen = 0usize;
    let mut dropped_secret = 0usize;
    let mut kept: BTreeMap<String, Value> = entries
        .into_iter()
        .inspect(|_| seen += 1)
        .filter(|(key, value)| {
            let secret = is_secret_extra_entry(key, value);
            dropped_secret += usize::from(secret);
            !secret
        })
        .filter(|(_, value)| is_bounded_value(value))
        .collect();
    let dropped_unsupported = seen - dropped_secret - kept.len();

    let mut dropped_for_count = 0usize;
    while kept.len() > MAX_EXTRA_KEYS {
        kept.pop_last();
        dropped_for_count += 1;
    }

    let mut entry_lens: Vec<(usize, String)> = kept
        .iter()
        .map(|(key, value)| (serialized_entry_len(key, value), key.clone()))
        .collect();
    let mut total: usize = entry_lens.iter().map(|(len, _)| len).sum();
    // Drop order: longest entry first, then key ascending.
    entry_lens.sort_by(|left, right| (Reverse(left.0), &left.1).cmp(&(Reverse(right.0), &right.1)));

    let mut dropped_for_size = 0usize;
    for (len, key) in entry_lens {
        if serialized_map_len(total, kept.len()) <= MAX_EXTRA_SERIALIZED_BYTES {
            break;
        }
        kept.remove(&key);
        total -= len;
        dropped_for_size += 1;
    }

    if dropped_secret > 0
        || dropped_unsupported > 0
        || dropped_for_count > 0
        || dropped_for_size > 0
    {
        tracing::debug!(
            dropped_secret,
            dropped_unsupported,
            dropped_for_count,
            dropped_for_size,
            kept = kept.len(),
            "bounded release listing extra map"
        );
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
    }

    fn search_result() -> crate::IndexerSearchResult {
        crate::IndexerSearchResult {
            indexer_id: None,
            source: "synthetic-indexer".to_string(),
            title: "Synthetic.Release.2020.1080p.WEB-DL-GRP".to_string(),
            link: None,
            download_url: None,
            source_kind: None,
            size_bytes: None,
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            indexer_languages: None,
            indexer_subtitles: None,
            indexer_grabs: None,
            password_hint: None,
            parsed_release_metadata: None,
            quality_profile_decision: None,
            extra: HashMap::new(),
            response_attributes: Default::default(),
            guid: None,
            info_url: None,
            provenance: None,
            candidate_token: None,
            queue_scope: None,
            coverage_scope: None,
            auto_eligible: None,
            auto_decision_code: None,
            auto_decision_summary: None,
            release_listing_json: None,
        }
    }

    fn full_snapshot() -> ReleaseListingSnapshot {
        let mut extra = BTreeMap::new();
        extra.insert("zeta".to_string(), json!(3));
        extra.insert("alpha".to_string(), json!("x"));
        extra.insert("list".to_string(), json!([1, "two", null, true]));
        ReleaseListingSnapshot {
            published_at: Some("2024-01-02T03:04:05Z".to_string()),
            thumbs_up: Some(7),
            thumbs_down: Some(-2),
            is_password_protected: Some(false),
            indexer_languages: vec!["en".to_string(), "fr".to_string()],
            extra,
            captured_at: at(2024, 2, 1),
        }
    }

    fn entries_of(value: &BTreeMap<String, Value>) -> usize {
        serde_json::to_string(value).unwrap().len()
    }

    #[test]
    fn bounding_keeps_scalars_and_scalar_arrays_only() {
        let raw: HashMap<String, Value> = [
            ("string", json!("s")),
            ("number", json!(1.5)),
            ("bool", json!(true)),
            ("null", Value::Null),
            ("scalars", json!([1, "a", false, null])),
            ("empty_array", json!([])),
            ("nested_object", json!({"a": 1})),
            ("mixed_array", json!([1, {"a": 1}])),
            ("nested_array", json!([[1]])),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();

        let bounded = bounded_extra(&raw);
        let keys: Vec<&str> = bounded.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec!["bool", "empty_array", "null", "number", "scalars", "string"]
        );
    }

    #[test]
    fn bounding_caps_key_count() {
        let raw: HashMap<String, Value> = (0..65)
            .map(|index| (format!("k{index:02}"), json!(index)))
            .collect();
        let bounded = bounded_extra(&raw);
        assert_eq!(bounded.len(), MAX_EXTRA_KEYS);
        // The key cap drops the lexicographically last key.
        assert!(!bounded.contains_key("k64"));
        assert!(bounded.contains_key("k63"));
        assert!(bounded.contains_key("k00"));
    }

    #[test]
    fn key_cap_keeps_scalar_arrays_rules_read() {
        let mut raw: HashMap<String, Value> = (0..65)
            .map(|index| (format!("k{index:02}"), json!(index)))
            .collect();
        raw.insert(
            "indexer_flags".to_string(),
            json!(["freeleech", "internal", "scene", "double_upload"]),
        );
        let bounded = bounded_extra(&raw);
        assert_eq!(bounded.len(), MAX_EXTRA_KEYS);
        assert_eq!(
            bounded["indexer_flags"],
            json!(["freeleech", "internal", "scene", "double_upload"])
        );
        assert!(!bounded.contains_key("k64"));
        assert!(!bounded.contains_key("k63"));
    }

    #[test]
    fn bounding_caps_serialized_size_by_dropping_longest_first() {
        let mut raw: HashMap<String, Value> = HashMap::new();
        raw.insert("longest".to_string(), json!("x".repeat(5000)));
        raw.insert("medium".to_string(), json!("y".repeat(3000)));
        raw.insert("short".to_string(), json!("z".repeat(500)));
        raw.insert("tiny".to_string(), json!(1));
        let bounded = bounded_extra(&raw);
        assert!(!bounded.contains_key("longest"));
        assert!(bounded.contains_key("medium"));
        assert!(bounded.contains_key("short"));
        assert!(bounded.contains_key("tiny"));
        assert!(entries_of(&bounded) <= MAX_EXTRA_SERIALIZED_BYTES);
    }

    #[test]
    fn bounding_accounts_for_exact_serialized_size() {
        // Fill to exactly the cap, then one byte over.
        let overhead = r#"{"a":""}"#.len();
        let mut raw: HashMap<String, Value> = HashMap::new();
        raw.insert(
            "a".to_string(),
            json!("x".repeat(MAX_EXTRA_SERIALIZED_BYTES - overhead)),
        );
        let bounded = bounded_extra(&raw);
        assert_eq!(entries_of(&bounded), MAX_EXTRA_SERIALIZED_BYTES);
        assert_eq!(bounded.len(), 1);

        raw.insert(
            "a".to_string(),
            json!("x".repeat(MAX_EXTRA_SERIALIZED_BYTES - overhead + 1)),
        );
        assert!(bounded_extra(&raw).is_empty());
    }

    #[test]
    fn rss_replay_markers_never_enter_a_snapshot() {
        let mut result = search_result();
        result
            .extra
            .insert("_rss_reconstructed_pending".to_string(), json!(true));
        result
            .extra
            .insert("_rss_reconstructed_pending_id".to_string(), json!("row-1"));
        result
            .extra
            .insert("synthetic_attribute".to_string(), json!("kept"));

        let snapshot = ReleaseListingSnapshot::capture_from_search_result(&result, at(2026, 1, 1));
        assert_eq!(
            snapshot.extra.keys().collect::<Vec<_>>(),
            ["synthetic_attribute"]
        );
    }

    #[test]
    fn secret_bearing_keys_never_enter_a_snapshot() {
        let mut result = search_result();
        for (key, value) in [
            (
                "magnet_uri",
                json!(
                    "magnet:?xt=urn:btih:0000&tr=https%3A%2F%2Ftracker.example%2Fsynthetic-passkey%2Fannounce"
                ),
            ),
            ("password", json!("synthetic-archive-password")),
            (
                "comment_url",
                json!("https://indexer.example/details/1?apikey=synthetic"),
            ),
            ("tracker_passkey", json!("synthetic-passkey")),
            ("api_token", json!("synthetic-token")),
            (
                "guid",
                json!("https://indexer.example/details/1?apikey=synthetic"),
            ),
            ("tags", json!(["synthetic-tag"])),
            ("freeleech", json!(true)),
        ] {
            result.extra.insert(key.to_string(), value);
        }

        let snapshot = ReleaseListingSnapshot::capture_from_search_result(&result, at(2026, 1, 1));
        assert_eq!(
            snapshot.extra.keys().collect::<Vec<_>>(),
            ["freeleech", "tags"]
        );
    }

    #[test]
    fn opaque_guid_is_kept() {
        let mut raw = HashMap::new();
        raw.insert("guid".to_string(), json!("synthetic-guid-0001"));
        assert_eq!(
            bounded_extra(&raw).get("guid"),
            Some(&json!("synthetic-guid-0001"))
        );
    }

    #[test]
    fn password_protected_flag_survives_only_as_a_boolean() {
        let mut raw = HashMap::new();
        raw.insert("password_protected".to_string(), json!(true));
        assert_eq!(
            bounded_extra(&raw).get("password_protected"),
            Some(&json!(true))
        );

        raw.insert(
            "password_protected".to_string(),
            json!("synthetic-archive-password"),
        );
        assert!(bounded_extra(&raw).is_empty());
    }

    #[test]
    fn from_json_str_scrubs_secrets_from_a_stored_snapshot() {
        let raw = r#"{"v":1,"captured_at":"2024-02-01T12:00:00Z","extra":{"password":"synthetic-archive-password","magnet_url":"magnet:?xt=urn:btih:0000","password_protected":true,"freeleech":true}}"#;
        let parsed = ReleaseListingSnapshot::from_json_str(raw).unwrap();
        assert_eq!(
            parsed.extra.keys().collect::<Vec<_>>(),
            ["freeleech", "password_protected"]
        );
        assert_eq!(parsed.extra["password_protected"], json!(true));
    }

    #[test]
    fn bounding_is_idempotent() {
        let mut raw: HashMap<String, Value> = (0..80)
            .map(|index| (format!("key{index}"), json!("v".repeat(index * 10))))
            .collect();
        raw.insert("object".to_string(), json!({"a": 1}));
        let once = bounded_extra(&raw);
        let again = bounded_extra(&once.clone().into_iter().collect());
        assert_eq!(once, again);

        // Bound, persist, read back, bound again: nothing changes.
        let snapshot = ReleaseListingSnapshot {
            extra: once.clone(),
            ..full_snapshot()
        };
        let parsed = ReleaseListingSnapshot::from_json_str(&snapshot.to_json_string()).unwrap();
        assert_eq!(parsed.extra, once);
        assert_eq!(
            bounded_extra(&parsed.extra.clone().into_iter().collect()),
            once
        );
        assert!(once.len() <= MAX_EXTRA_KEYS);
        assert!(entries_of(&once) <= MAX_EXTRA_SERIALIZED_BYTES);
    }

    #[test]
    fn bounding_is_deterministic_across_insertion_orders() {
        let pairs: Vec<(String, Value)> = (0..90)
            .map(|index| (format!("key{index}"), json!("v".repeat((index % 7) * 60))))
            .collect();
        let mut forward = HashMap::new();
        for (key, value) in pairs.iter().cloned() {
            forward.insert(key, value);
        }
        let mut backward = HashMap::with_capacity(1);
        for (key, value) in pairs.iter().rev().cloned() {
            backward.insert(key, value);
        }
        let left = bounded_extra(&forward);
        let right = bounded_extra(&backward);
        assert_eq!(left, right);
        assert_eq!(
            serde_json::to_string(&left).unwrap(),
            serde_json::to_string(&right).unwrap()
        );
    }

    #[test]
    fn language_bounding_caps_entry_count_in_original_order() {
        let raw: Vec<String> = (0..50).map(|index| format!("lang{index}")).collect();
        let bounded = bounded_indexer_languages(&raw);
        assert_eq!(bounded.len(), MAX_INDEXER_LANGUAGES);
        assert_eq!(bounded, raw[..MAX_INDEXER_LANGUAGES].to_vec());
    }

    #[test]
    fn language_bounding_drops_over_length_entries() {
        let at_limit = "a".repeat(MAX_INDEXER_LANGUAGE_BYTES);
        let over_limit = "b".repeat(MAX_INDEXER_LANGUAGE_BYTES + 1);
        // Multi-byte characters count by bytes, not chars.
        let over_limit_multibyte = "é".repeat(MAX_INDEXER_LANGUAGE_BYTES / 2 + 1);
        let raw = vec![
            over_limit,
            at_limit.clone(),
            over_limit_multibyte,
            format!("  {at_limit}x  "),
            "en".to_string(),
        ];
        assert_eq!(
            bounded_indexer_languages(&raw),
            vec![at_limit, "en".to_string()]
        );
    }

    #[test]
    fn language_bounding_trims_drops_blanks_and_repeats() {
        let raw = ["  en ", "", "   ", "fr", "en", "\tfr\n", "de"];
        assert_eq!(bounded_indexer_languages(raw), vec!["en", "fr", "de"]);
    }

    #[test]
    fn language_bounding_counts_only_kept_entries_toward_the_cap() {
        let mut raw: Vec<String> = vec![" ".to_string(); 40];
        raw.extend((0..40).map(|index| format!("lang{index}")));
        let bounded = bounded_indexer_languages(&raw);
        assert_eq!(bounded.len(), MAX_INDEXER_LANGUAGES);
        assert_eq!(bounded.first().map(String::as_str), Some("lang0"));
    }

    #[test]
    fn language_bounding_is_idempotent_through_capture_and_read() {
        let mut raw: Vec<String> = (0..40).map(|index| format!(" lang{index} ")).collect();
        raw.push("x".repeat(MAX_INDEXER_LANGUAGE_BYTES + 5));
        raw.push("lang3".to_string());
        let once = bounded_indexer_languages(&raw);
        assert_eq!(bounded_indexer_languages(&once), once);

        let mut result = search_result();
        result.indexer_languages = Some(raw);
        let captured = ReleaseListingSnapshot::capture_from_search_result(&result, at(2024, 3, 1));
        assert_eq!(captured.indexer_languages, once);
        let parsed = ReleaseListingSnapshot::from_json_str(&captured.to_json_string()).unwrap();
        assert_eq!(parsed.indexer_languages, once);
    }

    #[test]
    fn from_json_str_rebounds_stored_languages() {
        let languages: Vec<String> = (0..40)
            .map(|index| format!(" lang{index} "))
            .chain(["".to_string(), "lang1".to_string()])
            .collect();
        let stored = serde_json::to_string(&json!({
            "v": 1,
            "indexer_languages": languages,
            "captured_at": "2024-02-01T12:00:00Z",
        }))
        .unwrap();
        let parsed = ReleaseListingSnapshot::from_json_str(&stored).unwrap();
        assert_eq!(parsed.indexer_languages.len(), MAX_INDEXER_LANGUAGES);
        assert_eq!(parsed.indexer_languages[0], "lang0");
        assert_eq!(
            parsed.indexer_languages,
            bounded_indexer_languages(&languages)
        );
    }

    #[test]
    fn serialization_round_trips_every_field() {
        let snapshot = full_snapshot();
        let encoded = snapshot.to_json_string();
        assert_eq!(
            ReleaseListingSnapshot::from_json_str(&encoded),
            Some(snapshot)
        );
    }

    #[test]
    fn serialization_has_version_tag_and_stable_key_order() {
        let encoded = full_snapshot().to_json_string();
        assert_eq!(
            encoded,
            concat!(
                r#"{"v":1,"published_at":"2024-01-02T03:04:05Z","thumbs_up":7,"#,
                r#""thumbs_down":-2,"is_password_protected":false,"#,
                r#""indexer_languages":["en","fr"],"#,
                r#""extra":{"alpha":"x","list":[1,"two",null,true],"zeta":3},"#,
                r#""captured_at":"2024-02-01T12:00:00Z"}"#
            )
        );
    }

    #[test]
    fn from_json_str_tolerates_missing_version_and_unknown_fields() {
        let snapshot = full_snapshot();
        let mut value: Value = serde_json::from_str(&snapshot.to_json_string()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("v");
        object.insert("added_later".to_string(), json!({"anything": [1, 2]}));
        assert_eq!(
            ReleaseListingSnapshot::from_json_str(&value.to_string()),
            Some(snapshot)
        );
    }

    #[test]
    fn from_json_str_defaults_missing_optional_fields() {
        let parsed = ReleaseListingSnapshot::from_json_str(
            r#"{"v":1,"captured_at":"2024-02-01T12:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(
            parsed,
            ReleaseListingSnapshot {
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                indexer_languages: Vec::new(),
                extra: BTreeMap::new(),
                captured_at: at(2024, 2, 1),
            }
        );
    }

    #[test]
    fn from_json_str_rejects_unreadable_input() {
        for raw in [
            "",
            "not json",
            "[]",
            "null",
            "42",
            r#"{"v":1}"#,
            r#"{"v":2,"captured_at":"2024-02-01T12:00:00Z"}"#,
            r#"{"v":"1","captured_at":"2024-02-01T12:00:00Z"}"#,
            r#"{"captured_at":"not a time"}"#,
            r#"{"captured_at":"2024-02-01T12:00:00Z","thumbs_up":"many"}"#,
            r#"{"captured_at":"2024-02-01T12:00:00Z""#,
        ] {
            assert_eq!(ReleaseListingSnapshot::from_json_str(raw), None, "{raw}");
        }
    }

    #[test]
    fn age_days_anchors_on_the_argument() {
        let snapshot = ReleaseListingSnapshot {
            published_at: Some("2023-01-01T12:00:00Z".to_string()),
            ..full_snapshot()
        };
        assert_eq!(snapshot.age_days(at(2023, 1, 11)), Some(10));
        assert_eq!(snapshot.age_days(at(2024, 1, 11)), Some(375));
    }

    #[test]
    fn age_days_clamps_future_publish_to_zero() {
        let snapshot = ReleaseListingSnapshot {
            published_at: Some("2025-06-01T00:00:00Z".to_string()),
            ..full_snapshot()
        };
        assert_eq!(snapshot.age_days(at(2025, 1, 1)), Some(0));
    }

    #[test]
    fn age_days_unknown_without_publish_time() {
        let snapshot = ReleaseListingSnapshot {
            published_at: None,
            ..full_snapshot()
        };
        assert_eq!(snapshot.age_days(at(2025, 1, 1)), None);
    }

    #[test]
    fn capture_copies_listing_fields() {
        let mut result = search_result();
        result.published_at = Some("  Tue, 02 Jan 2024 03:04:05 +0000  ".to_string());
        result.thumbs_up = Some(4);
        result.thumbs_down = Some(1);
        result.indexer_languages = Some(vec!["de".to_string()]);
        result.extra.insert("freeleech".to_string(), json!(true));
        result.extra.insert("nested".to_string(), json!({"a": 1}));

        let snapshot = ReleaseListingSnapshot::capture_from_search_result(&result, at(2024, 3, 1));
        assert_eq!(
            snapshot.published_at.as_deref(),
            Some("Tue, 02 Jan 2024 03:04:05 +0000")
        );
        assert_eq!(snapshot.thumbs_up, Some(4));
        assert_eq!(snapshot.thumbs_down, Some(1));
        assert_eq!(snapshot.is_password_protected, None);
        assert_eq!(snapshot.indexer_languages, vec!["de".to_string()]);
        assert_eq!(snapshot.extra.keys().collect::<Vec<_>>(), vec!["freeleech"]);
        assert_eq!(snapshot.captured_at, at(2024, 3, 1));
        assert_eq!(snapshot.age_days(at(2024, 1, 12)), Some(10));
    }

    #[test]
    fn capture_drops_unparseable_published_at() {
        let mut result = search_result();
        result.published_at = Some("sometime last week".to_string());
        let snapshot = ReleaseListingSnapshot::capture_from_search_result(&result, at(2024, 3, 1));
        assert_eq!(snapshot.published_at, None);

        result.published_at = Some("   ".to_string());
        let snapshot = ReleaseListingSnapshot::capture_from_search_result(&result, at(2024, 3, 1));
        assert_eq!(snapshot.published_at, None);
    }

    #[test]
    fn password_flag_in_extra_wins_over_hint() {
        let mut result = search_result();
        result
            .extra
            .insert("password_protected".to_string(), json!(false));
        result.password_hint = Some("synthetic-secret".to_string());
        let snapshot = ReleaseListingSnapshot::capture_from_search_result(&result, at(2024, 3, 1));
        assert_eq!(snapshot.is_password_protected, Some(false));
    }

    #[test]
    fn password_hint_decides_without_extra_flag() {
        let mut result = search_result();
        let now = at(2024, 3, 1);

        result.password_hint = Some("synthetic-secret".to_string());
        assert_eq!(
            ReleaseListingSnapshot::capture_from_search_result(&result, now).is_password_protected,
            Some(true)
        );

        result.password_hint = Some("passworded".to_string());
        assert_eq!(
            ReleaseListingSnapshot::capture_from_search_result(&result, now).is_password_protected,
            Some(true)
        );

        result.password_hint = Some("0".to_string());
        assert_eq!(
            ReleaseListingSnapshot::capture_from_search_result(&result, now).is_password_protected,
            Some(true)
        );

        result.password_hint = None;
        assert_eq!(
            ReleaseListingSnapshot::capture_from_search_result(&result, now).is_password_protected,
            None
        );
    }

    #[test]
    fn non_bool_extra_password_flag_falls_back_to_hint() {
        let mut result = search_result();
        result
            .extra
            .insert("password_protected".to_string(), json!("yes"));
        result.password_hint = None;
        assert_eq!(
            ReleaseListingSnapshot::capture_from_search_result(&result, at(2024, 3, 1))
                .is_password_protected,
            None
        );
    }

    fn pending_release(source_password: Option<&str>) -> crate::PendingRelease {
        crate::PendingRelease {
            id: "pending-1".to_string(),
            wanted_item_id: "wanted-1".to_string(),
            title_id: "title-1".to_string(),
            release_title: "Synthetic.Release.S01E01.1080p.WEB-DL-GRP".to_string(),
            release_url: None,
            source_kind: None,
            release_size_bytes: None,
            release_score: 0,
            scoring_log_json: None,
            indexer_source: None,
            indexer_id: None,
            release_guid: None,
            added_at: "2024-01-01T00:00:00Z".to_string(),
            last_observed_at: "2024-01-01T00:00:00Z".to_string(),
            delay_until: "2024-01-01T00:00:00Z".to_string(),
            status: crate::PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: source_password.map(str::to_string),
            published_at: Some("2023-12-22T12:00:00Z".to_string()),
            info_hash: None,
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: String::new(),
            coverage_identity: String::new(),
            role: crate::PendingReleaseRole::Primary,
            last_decision_code: None,
            release_age_unknown: false,
            release_listing_json: None,
        }
    }

    #[test]
    fn pending_grab_keeps_the_rows_facts_stamped_at_the_grab() {
        let mut release = pending_release(None);
        release.release_listing_json = Some(full_snapshot().to_json_string());
        let grabbed = ReleaseListingSnapshot::json_for_pending_release(&release, at(2024, 6, 1))
            .expect("a pending grab always carries a snapshot");
        assert_eq!(
            ReleaseListingSnapshot::from_json_str(&grabbed),
            Some(ReleaseListingSnapshot {
                captured_at: at(2024, 6, 1),
                ..full_snapshot()
            })
        );
    }

    #[test]
    fn pending_grab_with_an_unreadable_snapshot_captures_from_the_row() {
        let mut release = pending_release(Some("synthetic-secret"));
        release.release_listing_json = Some("not json".to_string());
        assert_eq!(
            ReleaseListingSnapshot::json_for_pending_release(&release, at(2024, 6, 1)),
            Some(
                ReleaseListingSnapshot::capture_from_pending_release(&release, at(2024, 6, 1))
                    .to_json_string()
            )
        );
    }

    #[test]
    fn scoring_captures_a_fresh_listing_at_the_lane_now() {
        let mut result = search_result();
        result.thumbs_up = Some(3);
        let scored = ReleaseListingSnapshot::for_scoring(&result, at(2024, 3, 1));
        assert_eq!(
            scored,
            ReleaseListingSnapshot::capture_from_search_result(&result, at(2024, 3, 1))
        );
    }

    #[test]
    fn scoring_keeps_a_carried_snapshot_and_stamps_it_at_the_lane_now() {
        let mut result = search_result();
        // The synthetic listing a replay is rebuilt from must not leak in.
        result.thumbs_up = Some(99);
        result.release_listing_json = Some(full_snapshot().to_json_string());
        assert_eq!(
            ReleaseListingSnapshot::for_scoring(&result, at(2024, 3, 1)),
            ReleaseListingSnapshot {
                captured_at: at(2024, 3, 1),
                ..full_snapshot()
            }
        );
    }

    #[test]
    fn a_candidate_persists_the_snapshot_it_was_scored_with() {
        let mut result = search_result();
        let scored = ReleaseListingSnapshot::for_scoring(&result, at(2024, 3, 1)).to_json_string();
        result.release_listing_json = Some(scored.clone());
        assert_eq!(
            ReleaseListingSnapshot::json_for_candidate(&result, at(2024, 9, 1)),
            Some(scored)
        );
        result.release_listing_json = None;
        assert_eq!(
            ReleaseListingSnapshot::json_for_candidate(&result, at(2024, 9, 1)),
            ReleaseListingSnapshot::capture_json_from_search_result(&result, at(2024, 9, 1))
        );
    }

    #[test]
    fn pending_grab_without_a_snapshot_captures_from_the_row() {
        let release = pending_release(Some("synthetic-secret"));
        assert_eq!(
            ReleaseListingSnapshot::json_for_pending_release(&release, at(2024, 6, 1)),
            Some(
                ReleaseListingSnapshot::capture_from_pending_release(&release, at(2024, 6, 1))
                    .to_json_string()
            )
        );
    }

    #[test]
    fn capture_from_pending_release_fills_what_the_row_has() {
        let release = pending_release(Some("synthetic-secret"));
        let snapshot =
            ReleaseListingSnapshot::capture_from_pending_release(&release, at(2024, 1, 1));
        assert_eq!(
            snapshot,
            ReleaseListingSnapshot {
                published_at: Some("2023-12-22T12:00:00Z".to_string()),
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: Some(true),
                indexer_languages: Vec::new(),
                extra: BTreeMap::new(),
                captured_at: at(2024, 1, 1),
            }
        );
        assert_eq!(snapshot.age_days(at(2024, 1, 1)), Some(10));
    }

    #[test]
    fn pending_release_literal_passwords_imply_protection() {
        for flag in ["1", "true", "protected", "0", "no"] {
            let release = pending_release(Some(flag));
            let snapshot =
                ReleaseListingSnapshot::capture_from_pending_release(&release, at(2024, 1, 1));
            assert_eq!(snapshot.is_password_protected, Some(true), "{flag}");
        }
        let snapshot = ReleaseListingSnapshot::capture_from_pending_release(
            &pending_release(None),
            at(2024, 1, 1),
        );
        assert_eq!(snapshot.is_password_protected, None);
    }

    #[test]
    fn floats_survive_serialization_exactly() {
        let mut snapshot = full_snapshot();
        snapshot
            .extra
            .insert("ratio_a".to_string(), json!(9.533333333333333));
        snapshot
            .extra
            .insert("ratio_b".to_string(), json!(1.8333333333333335));
        snapshot
            .extra
            .insert("ratio_c".to_string(), json!(11.366666666666669));
        let parsed = ReleaseListingSnapshot::from_json_str(&snapshot.to_json_string()).unwrap();
        assert_eq!(parsed, snapshot);
        let rebounded = bounded_extra(&parsed.extra.clone().into_iter().collect());
        assert_eq!(rebounded, parsed.extra);
    }

    #[test]
    fn from_json_str_rebounds_stored_extra() {
        let raw = r#"{"v":1,"captured_at":"2024-02-01T12:00:00Z","extra":{"flat":1,"nested":{"a":1},"mixed":[1,[2]]}}"#;
        let parsed = ReleaseListingSnapshot::from_json_str(raw).unwrap();
        assert_eq!(parsed.extra.keys().collect::<Vec<_>>(), vec!["flat"]);
        assert_eq!(parsed.extra["flat"], json!(1));
    }

    #[test]
    fn from_json_str_reads_null_collections_as_empty() {
        let raw = r#"{"v":1,"published_at":"2024-01-02T03:04:05Z","thumbs_up":3,"indexer_languages":null,"extra":null,"captured_at":"2024-02-01T12:00:00Z"}"#;
        let parsed = ReleaseListingSnapshot::from_json_str(raw).unwrap();
        assert!(parsed.indexer_languages.is_empty());
        assert!(parsed.extra.is_empty());
        assert_eq!(parsed.thumbs_up, Some(3));
        assert_eq!(parsed.published_at.as_deref(), Some("2024-01-02T03:04:05Z"));

        let missing_captured_at = r#"{"v":1,"extra":null,"indexer_languages":null}"#;
        assert_eq!(
            ReleaseListingSnapshot::from_json_str(missing_captured_at),
            None
        );
        let null_captured_at = r#"{"v":1,"captured_at":null}"#;
        assert_eq!(
            ReleaseListingSnapshot::from_json_str(null_captured_at),
            None
        );
    }

    #[test]
    fn age_days_respects_non_utc_offset() {
        let snapshot = ReleaseListingSnapshot {
            published_at: Some("2024-01-01T20:00:00-05:00".to_string()),
            ..full_snapshot()
        };
        // 2024-01-02T01:00Z to 2024-01-02T12:00Z is under a day.
        assert_eq!(
            snapshot.age_days(Utc.with_ymd_and_hms(2024, 1, 2, 12, 0, 0).unwrap()),
            Some(0)
        );
    }

    #[test]
    fn release_listing_view_maps_every_field_and_ages_to_the_capture() {
        let snapshot = ReleaseListingSnapshot {
            published_at: Some("2024-03-01T12:00:00Z".to_string()),
            thumbs_up: Some(7),
            thumbs_down: Some(2),
            is_password_protected: Some(true),
            indexer_languages: vec!["en".to_string(), "fr".to_string()],
            extra: BTreeMap::from([
                ("grabs".to_string(), json!(40)),
                ("tags".to_string(), json!(["internal"])),
            ]),
            captured_at: at(2024, 3, 15),
        };

        let view = release_listing_view(&snapshot.to_json_string()).expect("view reads");

        assert_eq!(view.published_at, Some(at(2024, 3, 1)));
        assert_eq!(view.age_days_at_grab, Some(14));
        assert_eq!(view.thumbs_up, Some(7));
        assert_eq!(view.thumbs_down, Some(2));
        assert_eq!(view.is_password_protected, Some(true));
        assert_eq!(view.indexer_languages, vec!["en", "fr"]);
        assert_eq!(view.extra, snapshot.extra);
        assert_eq!(view.captured_at, at(2024, 3, 15));
    }

    #[test]
    fn release_listing_view_is_none_for_garbage() {
        assert_eq!(release_listing_view("{not a snapshot"), None);
        assert_eq!(release_listing_view("[]"), None);
    }
}
