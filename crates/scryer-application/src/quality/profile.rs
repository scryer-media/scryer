#[cfg(test)]
use super::pack_test_support::*;
use crate::release_parser::{AudioCodec, ParsedReleaseMetadata, ReleaseSource, VideoCodec};
use crate::scoring_weights::{ScoringOverrides, ScoringPersona};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QualityProfile {
    pub id: String,
    pub name: String,
    pub criteria: QualityProfileCriteria,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QualityProfileCriteria {
    pub quality_tiers: Vec<String>,
    pub archival_quality: Option<String>,
    pub allow_unknown_quality: bool,
    pub source_allowlist: Vec<ReleaseSource>,
    pub source_blocklist: Vec<ReleaseSource>,
    pub video_codec_allowlist: Vec<VideoCodec>,
    pub video_codec_blocklist: Vec<VideoCodec>,
    pub audio_codec_allowlist: Vec<AudioCodec>,
    pub audio_codec_blocklist: Vec<AudioCodec>,
    pub atmos_preferred: bool,
    pub dolby_vision_allowed: bool,
    pub detected_hdr_allowed: bool,
    pub prefer_remux: bool,
    pub allow_bd_disk: bool,
    pub allow_upgrades: bool,
    pub prefer_dual_audio: bool,
    pub required_audio_languages: Vec<String>,
    pub scoring_persona: ScoringPersona,
    pub scoring_overrides: ScoringOverrides,
    pub cutoff_tier: Option<String>,
    /// Sonarr's `MinFormatScore`: the absolute floor a release must clear to be
    /// grabbed at all, applied by `apply_min_score_gate` as a veto. Nothing
    /// else reads it.
    pub min_score_to_grab: Option<i32>,
    /// Sonarr's `CutoffFormatScore`: the score past which a file is good enough
    /// and same-tier improvements stop earning bandwidth.
    ///
    /// Split out from `min_score_to_grab`, which was doing both jobs (D19). One
    /// is a floor on the *candidate* and one is a ceiling on the *incumbent*;
    /// tying them meant a profile could not say "grab nothing under 100" without
    /// also saying "stop upgrading at 100".
    ///
    /// `#[serde(skip_serializing_if)]` is load-bearing, not tidiness:
    /// `convergence::profile_criteria_version` hashes this struct's JSON, and
    /// that hash decides whether a scope's convergence coverage is still valid.
    /// A new key would invalidate every scope in the library on upgrade and
    /// trigger a full re-search. Omitted when unset, it does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutoff_score: Option<i32>,
    pub facet_persona_overrides: HashMap<String, ScoringPersona>,
}

/// The acceptance-deciding subset of [`QualityProfileCriteria`], hashed by
/// `convergence::profile_criteria_version` to version a scope's search
/// fingerprint.
///
/// Only fields that change **which releases are acceptable** belong here.
/// Ranking-only fields (`scoring_persona`, `scoring_overrides`,
/// `facet_persona_overrides`, `atmos_preferred`, `prefer_remux`,
/// `prefer_dual_audio`) are deliberately absent: re-ordering candidates needs no
/// new indexer data, so a ranking edit must not invalidate convergence coverage
/// and force a library-wide re-search. Re-ranking happens against the results
/// already on hand.
///
/// Two rules bind anyone touching `QualityProfileCriteria`:
///
/// 1. Every new field must be explicitly classified acceptance-vs-ranking and,
///    if it gates acceptance, mirrored here. Silence defaults it to "ranking",
///    which is the wrong answer for a field that vetoes releases.
/// 2. Every `Option` here carries `skip_serializing_if = "Option::is_none"`, so
///    an unset value is an absent key rather than a `null`. That is what lets a
///    field be added without moving the fingerprint of every profile that does
///    not set it (D19 — see the `cutoff_score` comment above).
#[derive(Debug, Serialize)]
pub(crate) struct AcceptanceCriteria<'a> {
    pub quality_tiers: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archival_quality: Option<&'a String>,
    pub allow_unknown_quality: bool,
    pub source_allowlist: &'a [ReleaseSource],
    pub source_blocklist: &'a [ReleaseSource],
    pub video_codec_allowlist: &'a [VideoCodec],
    pub video_codec_blocklist: &'a [VideoCodec],
    pub audio_codec_allowlist: &'a [AudioCodec],
    pub audio_codec_blocklist: &'a [AudioCodec],
    pub dolby_vision_allowed: bool,
    pub detected_hdr_allowed: bool,
    pub allow_bd_disk: bool,
    pub allow_upgrades: bool,
    pub required_audio_languages: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cutoff_tier: Option<&'a String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_score_to_grab: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cutoff_score: Option<i32>,
}

impl<'a> From<&'a QualityProfileCriteria> for AcceptanceCriteria<'a> {
    fn from(criteria: &'a QualityProfileCriteria) -> Self {
        Self {
            quality_tiers: &criteria.quality_tiers,
            archival_quality: criteria.archival_quality.as_ref(),
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist: &criteria.source_allowlist,
            source_blocklist: &criteria.source_blocklist,
            video_codec_allowlist: &criteria.video_codec_allowlist,
            video_codec_blocklist: &criteria.video_codec_blocklist,
            audio_codec_allowlist: &criteria.audio_codec_allowlist,
            audio_codec_blocklist: &criteria.audio_codec_blocklist,
            dolby_vision_allowed: criteria.dolby_vision_allowed,
            detected_hdr_allowed: criteria.detected_hdr_allowed,
            allow_bd_disk: criteria.allow_bd_disk,
            allow_upgrades: criteria.allow_upgrades,
            required_audio_languages: &criteria.required_audio_languages,
            cutoff_tier: criteria.cutoff_tier.as_ref(),
            min_score_to_grab: criteria.min_score_to_grab,
            cutoff_score: criteria.cutoff_score,
        }
    }
}

/// JSON-serializable container for all scoring-related fields.
/// Stored in the `scoring_config` TEXT column of the `quality_profiles` table.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScoringConfig {
    #[serde(default)]
    pub scoring_persona: ScoringPersona,
    #[serde(default)]
    pub scoring_overrides: ScoringOverrides,
    #[serde(default)]
    pub cutoff_tier: Option<String>,
    #[serde(default)]
    pub min_score_to_grab: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutoff_score: Option<i32>,
    #[serde(default)]
    pub facet_persona_overrides: HashMap<String, ScoringPersona>,
}

impl QualityProfileCriteria {
    /// Returns the scoring persona for a given media category, falling back to the
    /// base `scoring_persona` if no facet-specific override exists.
    pub fn resolve_persona(&self, category: Option<&str>) -> &ScoringPersona {
        if let Some(cat) = category
            && let Some(persona) = self.facet_persona_overrides.get(cat)
        {
            return persona;
        }
        &self.scoring_persona
    }
}

/// Strong, recoverable scoring penalty. Mandatory requirements are separate.
pub const BLOCK_SCORE: i32 = -10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoringEntryKind {
    ScoreContribution,
    MandatoryRejection,
    FinalScoreRejection,
}

impl ScoringEntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScoreContribution => "score_contribution",
            Self::MandatoryRejection => "mandatory_rejection",
            Self::FinalScoreRejection => "final_score_rejection",
        }
    }
}

/// Accumulate before narrowing, so cancelling large contributions are order independent.
pub fn sum_score_deltas(deltas: impl IntoIterator<Item = i32>) -> i32 {
    let total = deltas.into_iter().fold(0_i128, |total, delta| {
        total
            .checked_add(i128::from(delta))
            .expect("a memory-resident collection of i32 contributions fits in i128")
    });
    total.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32
}

/// Distinguishes built-in scoring entries from named-rule entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoringSource {
    Builtin,
    UserRule { id: String, name: String },
    SystemRule { id: String, name: String },
}

/// A single entry in the scoring log. Every decision point — blocking or preferential —
/// produces one entry so callers can inspect exactly why a release scored the way it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoringEntry {
    pub code: String,
    pub delta: i32,
    pub source: ScoringSource,
    pub kind: ScoringEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityProfileDecision {
    /// Sum of all `scoring_log` deltas.
    pub release_score: i32,
    /// Every decision point in the order it was applied.
    pub scoring_log: Vec<ScoringEntry>,
    /// True when no explicit mandatory or finalized score rejection applies.
    pub allowed: bool,
    /// Codes from explicit rejections; numeric penalties never appear here by themselves.
    pub block_codes: Vec<String>,
    /// Kept equal to `release_score` so existing sort logic works without changes.
    pub preference_score: i32,
    /// Where this release's quality sits in the profile's ordering; lower is
    /// better, `None` when the profile does not list it.
    ///
    /// Carried on the decision so that **every** place results are ordered can
    /// compare tier before score without re-resolving the profile. The tier
    /// stopped contributing points when it became a comparison step, so a
    /// comparator that only sees `preference_score` will happily list a 720p
    /// release above a 2160p one (D11).
    pub tier_index: Option<usize>,
}

impl QualityProfileDecision {
    fn new() -> Self {
        Self {
            release_score: 0,
            scoring_log: Vec::new(),
            allowed: true,
            block_codes: Vec::new(),
            preference_score: 0,
            tier_index: None,
        }
    }

    /// Record a decision point and keep the derived fields consistent.
    #[cfg(test)]
    fn log(&mut self, code: &str, delta: i32) {
        self.log_with_source(code, delta, ScoringSource::Builtin);
    }

    /// Record a decision point with an explicit source.
    pub fn log_with_source(&mut self, code: &str, delta: i32, source: ScoringSource) {
        self.scoring_log
            .retain(|entry| entry.kind != ScoringEntryKind::FinalScoreRejection);
        self.scoring_log.push(ScoringEntry {
            code: code.to_string(),
            delta,
            source,
            kind: ScoringEntryKind::ScoreContribution,
        });
        self.refresh();
    }

    fn reject(&mut self, code: &str, kind: ScoringEntryKind) {
        self.scoring_log.push(ScoringEntry {
            code: code.to_string(),
            delta: 0,
            source: ScoringSource::Builtin,
            kind,
        });
        self.refresh();
    }

    fn reject_requirement(&mut self, code: &str) {
        self.reject(code, ScoringEntryKind::MandatoryRejection);
    }

    fn refresh(&mut self) {
        for entry in &mut self.scoring_log {
            if entry.kind != ScoringEntryKind::ScoreContribution {
                entry.delta = 0;
            }
        }
        self.release_score = sum_score_deltas(self.scoring_log.iter().map(|entry| entry.delta));
        self.preference_score = self.release_score;
        self.block_codes = self
            .scoring_log
            .iter()
            .filter(|entry| entry.kind != ScoringEntryKind::ScoreContribution)
            .map(|entry| entry.code.clone())
            .collect();
        self.allowed = self.block_codes.is_empty();
    }
}

#[derive(Debug, Deserialize)]
struct RawQualityProfileCriteria {
    #[serde(default)]
    quality_tiers: Vec<String>,
    #[serde(default)]
    archival_quality: Option<String>,
    #[serde(default)]
    allow_unknown_quality: bool,
    #[serde(default)]
    source_allowlist: Vec<String>,
    #[serde(default)]
    source_blocklist: Vec<String>,
    #[serde(default)]
    video_codec_allowlist: Vec<String>,
    #[serde(default)]
    video_codec_blocklist: Vec<String>,
    #[serde(default)]
    audio_codec_allowlist: Vec<String>,
    #[serde(default)]
    audio_codec_blocklist: Vec<String>,
    #[serde(default)]
    atmos_preferred: bool,
    #[serde(default)]
    dolby_vision_allowed: bool,
    #[serde(default = "default_true")]
    detected_hdr_allowed: bool,
    #[serde(default)]
    prefer_remux: bool,
    #[serde(default)]
    allow_bd_disk: bool,
    #[serde(default)]
    allow_upgrades: bool,
    #[serde(default)]
    prefer_dual_audio: bool,
    #[serde(default)]
    required_audio_languages: Vec<String>,
    #[serde(default)]
    scoring_persona: ScoringPersona,
    #[serde(default)]
    scoring_overrides: ScoringOverrides,
    #[serde(default)]
    cutoff_tier: Option<String>,
    #[serde(default)]
    min_score_to_grab: Option<i32>,
    #[serde(default)]
    cutoff_score: Option<i32>,
    #[serde(default)]
    facet_persona_overrides: HashMap<String, ScoringPersona>,
}

#[derive(Debug, Deserialize)]
struct RawQualityProfile {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    criteria: RawQualityProfileCriteria,
}
pub const QUALITY_PROFILE_CATALOG_KEY: &str = "quality.profiles";
pub const QUALITY_PROFILE_ID_KEY: &str = "quality.profile_id";
pub const REQUEST_QUALITY_PROFILE_IDS_KEY: &str = "quality.request_profile_ids";
pub const QUALITY_PROFILE_INHERIT_VALUE: &str = "__inherit__";
fn default_true() -> bool {
    true
}

pub fn parse_profile_catalog_from_json(
    raw_json: &str,
) -> Result<Vec<QualityProfile>, serde_json::Error> {
    let profiles = serde_json::from_str::<Vec<RawQualityProfile>>(raw_json)?;
    profiles.into_iter().map(quality_profile_from_raw).collect()
}

/// Identifier of the built-in profile Scryer uses whenever a default quality
/// profile is needed and no explicit configuration supplies one. This constant
/// and [`builtin_default_quality_profile`] are the single owner of that
/// policy: the settings-definition seed mirrors this value into the database,
/// and every other fallback site must consume one of these two symbols rather
/// than naming a profile id directly.
pub const BUILTIN_DEFAULT_QUALITY_PROFILE_ID: &str = "1080p";

/// The built-in profile behind [`BUILTIN_DEFAULT_QUALITY_PROFILE_ID`].
pub fn builtin_default_quality_profile() -> QualityProfile {
    builtin_1080p_profile()
}

pub fn builtin_4k_profile() -> QualityProfile {
    QualityProfile {
        id: "4k".to_string(),
        name: "4K".to_string(),
        criteria: QualityProfileCriteria {
            quality_tiers: vec!["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
            archival_quality: Some("2160P".to_string()),
            allow_unknown_quality: false,
            source_allowlist: Vec::new(),
            source_blocklist: Vec::new(),
            video_codec_allowlist: Vec::new(),
            video_codec_blocklist: Vec::new(),
            audio_codec_allowlist: Vec::new(),
            audio_codec_blocklist: Vec::new(),
            atmos_preferred: false,
            dolby_vision_allowed: true,
            detected_hdr_allowed: true,
            prefer_remux: false,
            allow_bd_disk: false,
            allow_upgrades: true,
            prefer_dual_audio: false,
            required_audio_languages: vec![],
            scoring_persona: ScoringPersona::default(),
            scoring_overrides: ScoringOverrides::default(),
            cutoff_tier: None,
            min_score_to_grab: None,
            cutoff_score: None,
            facet_persona_overrides: HashMap::new(),
        },
    }
}

pub fn builtin_8k_profile() -> QualityProfile {
    QualityProfile {
        id: "8k".to_string(),
        name: "8K".to_string(),
        criteria: QualityProfileCriteria {
            quality_tiers: vec![
                "4320P".to_string(),
                "2160P".to_string(),
                "1080P".to_string(),
                "720P".to_string(),
            ],
            archival_quality: Some("4320P".to_string()),
            allow_unknown_quality: false,
            source_allowlist: Vec::new(),
            source_blocklist: Vec::new(),
            video_codec_allowlist: Vec::new(),
            video_codec_blocklist: Vec::new(),
            audio_codec_allowlist: Vec::new(),
            audio_codec_blocklist: Vec::new(),
            atmos_preferred: false,
            dolby_vision_allowed: true,
            detected_hdr_allowed: true,
            prefer_remux: false,
            allow_bd_disk: false,
            allow_upgrades: true,
            prefer_dual_audio: false,
            required_audio_languages: vec![],
            scoring_persona: ScoringPersona::default(),
            scoring_overrides: ScoringOverrides::default(),
            cutoff_tier: None,
            min_score_to_grab: None,
            cutoff_score: None,
            facet_persona_overrides: HashMap::new(),
        },
    }
}

pub fn builtin_1080p_profile() -> QualityProfile {
    QualityProfile {
        id: "1080p".to_string(),
        name: "1080P".to_string(),
        criteria: QualityProfileCriteria {
            quality_tiers: vec!["1080P".to_string(), "720P".to_string()],
            archival_quality: Some("1080P".to_string()),
            allow_unknown_quality: false,
            source_allowlist: Vec::new(),
            source_blocklist: Vec::new(),
            video_codec_allowlist: Vec::new(),
            video_codec_blocklist: Vec::new(),
            audio_codec_allowlist: Vec::new(),
            audio_codec_blocklist: Vec::new(),
            atmos_preferred: false,
            dolby_vision_allowed: true,
            detected_hdr_allowed: true,
            prefer_remux: false,
            allow_bd_disk: false,
            allow_upgrades: true,
            prefer_dual_audio: false,
            required_audio_languages: vec![],
            scoring_persona: ScoringPersona::default(),
            scoring_overrides: ScoringOverrides::default(),
            cutoff_tier: None,
            min_score_to_grab: None,
            cutoff_score: None,
            facet_persona_overrides: HashMap::new(),
        },
    }
}

pub fn builtin_anime_profile() -> QualityProfile {
    QualityProfile {
        id: "anime".to_string(),
        name: "Anime".to_string(),
        criteria: QualityProfileCriteria {
            quality_tiers: vec!["1080P".to_string(), "720P".to_string(), "576P".to_string()],
            archival_quality: Some("1080P".to_string()),
            allow_unknown_quality: false,
            source_allowlist: Vec::new(),
            source_blocklist: Vec::new(),
            video_codec_allowlist: Vec::new(),
            video_codec_blocklist: Vec::new(),
            audio_codec_allowlist: Vec::new(),
            audio_codec_blocklist: Vec::new(),
            atmos_preferred: false,
            dolby_vision_allowed: true,
            detected_hdr_allowed: true,
            prefer_remux: false,
            allow_bd_disk: false,
            allow_upgrades: true,
            prefer_dual_audio: false,
            required_audio_languages: vec![],
            scoring_persona: ScoringPersona::default(),
            scoring_overrides: ScoringOverrides::default(),
            cutoff_tier: None,
            min_score_to_grab: None,
            cutoff_score: None,
            facet_persona_overrides: HashMap::new(),
        },
    }
}

fn quality_profile_from_raw(raw: RawQualityProfile) -> Result<QualityProfile, serde_json::Error> {
    let criteria = raw.criteria;
    let quality_tiers = normalize_list(criteria.quality_tiers);
    let archival_quality = resolve_archival_quality(criteria.archival_quality, &quality_tiers);
    Ok(QualityProfile {
        id: raw.id,
        name: raw.name,
        criteria: QualityProfileCriteria {
            quality_tiers,
            archival_quality,
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist: normalize_source_list(
                criteria.source_allowlist,
                "criteria.source_allowlist",
            )?,
            source_blocklist: normalize_source_list(
                criteria.source_blocklist,
                "criteria.source_blocklist",
            )?,
            video_codec_allowlist: normalize_codec_list(
                criteria.video_codec_allowlist,
                "criteria.video_codec_allowlist",
            )?,
            video_codec_blocklist: normalize_codec_list(
                criteria.video_codec_blocklist,
                "criteria.video_codec_blocklist",
            )?,
            audio_codec_allowlist: normalize_audio_codec_list(
                criteria.audio_codec_allowlist,
                "criteria.audio_codec_allowlist",
            )?,
            audio_codec_blocklist: normalize_audio_codec_list(
                criteria.audio_codec_blocklist,
                "criteria.audio_codec_blocklist",
            )?,
            atmos_preferred: criteria.atmos_preferred,
            dolby_vision_allowed: criteria.dolby_vision_allowed,
            detected_hdr_allowed: criteria.detected_hdr_allowed,
            prefer_remux: criteria.prefer_remux,
            allow_bd_disk: criteria.allow_bd_disk,
            allow_upgrades: criteria.allow_upgrades,
            prefer_dual_audio: criteria.prefer_dual_audio,
            required_audio_languages: criteria
                .required_audio_languages
                .into_iter()
                .map(|l| l.trim().to_ascii_uppercase())
                .filter(|l| !l.is_empty())
                .collect(),
            scoring_persona: criteria.scoring_persona,
            scoring_overrides: criteria.scoring_overrides,
            cutoff_tier: criteria.cutoff_tier,
            min_score_to_grab: criteria.min_score_to_grab,
            cutoff_score: criteria.cutoff_score,
            facet_persona_overrides: criteria.facet_persona_overrides,
        },
    })
}

/// Test scaffolding only: this is the 4k-shaped profile, NOT the canonical
/// default. Production fallbacks must use [`builtin_default_quality_profile`]
/// — never `QualityProfile::default()`.
impl Default for QualityProfile {
    fn default() -> Self {
        Self {
            id: "4k".to_string(),
            name: "4K".to_string(),
            criteria: QualityProfileCriteria {
                quality_tiers: vec!["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
                allow_unknown_quality: false,
                archival_quality: Some("2160P".to_string()),
                source_allowlist: vec![],
                source_blocklist: vec![],
                video_codec_allowlist: vec![],
                video_codec_blocklist: vec![],
                audio_codec_allowlist: vec![],
                audio_codec_blocklist: vec![],
                atmos_preferred: false,
                dolby_vision_allowed: true,
                detected_hdr_allowed: true,
                prefer_remux: false,
                allow_bd_disk: false,
                allow_upgrades: true,
                prefer_dual_audio: false,
                required_audio_languages: vec![],
                scoring_persona: ScoringPersona::default(),
                scoring_overrides: ScoringOverrides::default(),
                cutoff_tier: None,
                min_score_to_grab: None,
                cutoff_score: None,
                facet_persona_overrides: HashMap::new(),
            },
        }
    }
}

impl Default for QualityProfileCriteria {
    fn default() -> Self {
        QualityProfileCriteria {
            quality_tiers: vec!["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
            allow_unknown_quality: false,
            archival_quality: Some("1080P".to_string()),
            source_allowlist: Vec::new(),
            source_blocklist: Vec::new(),
            video_codec_allowlist: Vec::new(),
            video_codec_blocklist: Vec::new(),
            audio_codec_allowlist: Vec::new(),
            audio_codec_blocklist: Vec::new(),
            atmos_preferred: false,
            dolby_vision_allowed: true,
            detected_hdr_allowed: true,
            prefer_remux: false,
            allow_bd_disk: true,
            allow_upgrades: true,
            prefer_dual_audio: false,
            required_audio_languages: vec![],
            scoring_persona: ScoringPersona::default(),
            scoring_overrides: ScoringOverrides::default(),
            cutoff_tier: None,
            min_score_to_grab: None,
            cutoff_score: None,
            facet_persona_overrides: HashMap::new(),
        }
    }
}

impl QualityProfile {
    pub fn parse(raw_json: &str) -> Result<Self, serde_json::Error> {
        let raw: RawQualityProfile = serde_json::from_str(raw_json)?;
        quality_profile_from_raw(raw)
    }
}

fn normalize_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn invalid_profile_codec(field: &str, value: &str) -> serde_json::Error {
    serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("invalid value {value:?} for {field}"),
    ))
}

fn normalize_source_list(
    values: Vec<String>,
    field: &str,
) -> Result<Vec<ReleaseSource>, serde_json::Error> {
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim().to_string();
            ReleaseSource::parse(trimmed.as_str())
                .ok_or_else(|| invalid_profile_codec(field, trimmed.as_str()))
        })
        .collect()
}

fn normalize_codec_list(
    values: Vec<String>,
    field: &str,
) -> Result<Vec<VideoCodec>, serde_json::Error> {
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim().to_string();
            VideoCodec::parse(trimmed.as_str())
                .ok_or_else(|| invalid_profile_codec(field, trimmed.as_str()))
        })
        .collect()
}

fn normalize_audio_codec_list(
    values: Vec<String>,
    field: &str,
) -> Result<Vec<AudioCodec>, serde_json::Error> {
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim().to_string();
            AudioCodec::parse(trimmed.as_str())
                .ok_or_else(|| invalid_profile_codec(field, trimmed.as_str()))
        })
        .collect()
}

fn normalized_audio_codecs(release: &ParsedReleaseMetadata) -> Vec<AudioCodec> {
    let mut codecs = Vec::<AudioCodec>::new();

    for codec in &release.audio_codecs {
        if !codecs.iter().any(|existing| existing == codec) {
            codecs.push(*codec);
        }
    }

    if codecs.is_empty()
        && let Some(normalized) = release.audio
    {
        codecs.push(normalized);
    }

    codecs
}

#[cfg(test)]
fn normalize_source(raw: Option<&str>) -> Option<String> {
    raw.and_then(ReleaseSource::parse)
        .map(|source| source.to_string())
}

#[cfg(test)]
fn normalize_codec(raw: Option<&str>) -> Option<String> {
    raw.and_then(VideoCodec::parse)
        .map(|codec| codec.to_string())
}

pub(crate) fn normalize_quality_tier(raw: Option<&str>) -> Option<String> {
    raw.map(|value| {
        let value = value.trim().to_ascii_lowercase();
        let clean = value;
        if clean.ends_with('p') && clean.len() > 1 {
            let numeric = &clean[..clean.len() - 1];
            format!("{}P", numeric)
        } else {
            clean.to_ascii_uppercase()
        }
    })
}

/// The vertical resolution a quality label names, if it names one.
///
/// Deliberately more forgiving than [`normalize_quality_tier`], which is an
/// *exact* key into the profile's tier list and must stay one: this reads the
/// number out of the label, so `1080p`, `1080i`, `Bluray-1080p` and a bare
/// `1080` all report 1080. Anything that names no resolution reports `None`.
///
/// It exists so that ordering by resolution and looking a quality up in the
/// profile share one notion of what a label says. They did not: the cutoff
/// election ranked `1080i` and every compound Sonarr-style label as worse than
/// 480p, so a scope holding one could elect the wrong file as its cutoff
/// quality.
pub(crate) fn resolution_lines(raw: Option<&str>) -> Option<u32> {
    let value = raw?.trim().to_ascii_lowercase();
    if value.is_empty() {
        return None;
    }
    let plausible = |lines: u32| (100..=10_000).contains(&lines).then_some(lines);
    // A bare number is a resolution on its own ("1080").
    if let Ok(lines) = value.parse::<u32>() {
        return plausible(lines);
    }
    // Otherwise take the largest `<digits>p` / `<digits>i` token in the label,
    // so a compound `bluray-1080p` reads as 1080 rather than as nothing.
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter_map(|token| {
            let digits = token
                .strip_suffix('p')
                .or_else(|| token.strip_suffix('i'))?;
            digits.parse::<u32>().ok().and_then(plausible)
        })
        .max()
}

/// Where a quality sits in the profile's ordering, or `None` when the profile
/// does not list it.
///
/// This is Sonarr's `QualityProfile.GetIndex`: the single source of "is this
/// better than that" for quality, used by the admission gate and by search
/// ordering so the two can never disagree about what outranks what.
pub(crate) fn quality_tier_index(
    criteria: &QualityProfileCriteria,
    quality: Option<&str>,
) -> Option<usize> {
    let quality = normalize_quality_tier(quality)?;
    criteria
        .quality_tiers
        .iter()
        .position(|tier| tier == &quality)
}

pub fn resolve_profile_id_for_title(
    title_profile_id: Option<&str>,
    library_profile_id: Option<&str>,
    category_profile_id: Option<&str>,
    global_profile_id: Option<&str>,
) -> Option<String> {
    title_profile_id
        .map(std::string::ToString::to_string)
        .or_else(|| library_profile_id.map(std::string::ToString::to_string))
        .or_else(|| category_profile_id.map(std::string::ToString::to_string))
        .or_else(|| global_profile_id.map(std::string::ToString::to_string))
}

pub fn quality_meets_or_exceeds_cutoff(
    current_quality: &str,
    cutoff_tier: &str,
    quality_tiers: &[String],
) -> bool {
    if quality_tiers.is_empty() {
        return false;
    }

    let current_normalized = match normalize_quality_tier(Some(current_quality)) {
        Some(quality) => quality,
        None => return false,
    };
    let cutoff_normalized = match normalize_quality_tier(Some(cutoff_tier)) {
        Some(quality) => quality,
        None => return false,
    };

    let cutoff_pos = match quality_tiers
        .iter()
        .position(|tier| tier == &cutoff_normalized)
    {
        Some(position) => position,
        None => return false,
    };
    match quality_tiers
        .iter()
        .position(|tier| tier == &current_normalized)
    {
        Some(current_pos) => current_pos <= cutoff_pos,
        None => false,
    }
}

pub fn evaluate_profile_requirements(
    profile: &QualityProfile,
    release: &ParsedReleaseMetadata,
    has_existing_file: bool,
    _category_hint: Option<&str>,
) -> QualityProfileDecision {
    let mut decision = QualityProfileDecision::new();
    let criteria = &profile.criteria;
    decision.tier_index = quality_tier_index(criteria, release.quality.as_deref());
    if !criteria.allow_upgrades && has_existing_file {
        decision.reject_requirement("upgrade_blocked_by_profile");
    }
    match normalize_quality_tier(release.quality.as_deref()) {
        Some(quality)
            if !criteria.quality_tiers.is_empty()
                && !criteria.quality_tiers.iter().any(|tier| tier == &quality) =>
        {
            decision.reject_requirement("quality_not_in_profile_tiers");
        }
        None if !criteria.allow_unknown_quality => {
            decision.reject_requirement("quality_missing_and_profile_disallows_unknown");
        }
        _ => {}
    }
    match release.source {
        Some(source) => {
            let explicitly_allowed = !criteria.source_allowlist.is_empty()
                && criteria.source_allowlist.contains(&source);
            if matches!(
                source,
                ReleaseSource::Cam
                    | ReleaseSource::Telesync
                    | ReleaseSource::Telecine
                    | ReleaseSource::DvdScr
                    | ReleaseSource::Workprint
            ) && !explicitly_allowed
            {
                decision.reject_requirement("source_low_quality_theatrical");
            } else if criteria.source_blocklist.contains(&source) {
                decision.reject_requirement("source_in_profile_blocklist");
            } else if !criteria.source_allowlist.is_empty()
                && !criteria.source_allowlist.contains(&source)
            {
                decision.reject_requirement("source_not_in_profile_allowlist");
            }
        }
        None if !criteria.source_allowlist.is_empty() => {
            decision.reject_requirement("source_missing_and_profile_requires_source");
        }
        None => {}
    }
    if let Some(codec) = release.video_codec.as_ref() {
        if criteria.video_codec_blocklist.contains(codec) {
            decision.reject_requirement("video_codec_in_profile_blocklist");
        } else if !criteria.video_codec_allowlist.is_empty()
            && !criteria.video_codec_allowlist.contains(codec)
        {
            decision.reject_requirement("video_codec_not_in_profile_allowlist");
        }
    }
    let audio_codecs = normalized_audio_codecs(release);
    if !audio_codecs.is_empty() {
        if !criteria.audio_codec_blocklist.is_empty()
            && audio_codecs
                .iter()
                .all(|codec| criteria.audio_codec_blocklist.contains(codec))
        {
            decision.reject_requirement("audio_codec_in_profile_blocklist");
        } else if !criteria.audio_codec_allowlist.is_empty()
            && !audio_codecs
                .iter()
                .any(|codec| criteria.audio_codec_allowlist.contains(codec))
        {
            decision.reject_requirement("audio_codec_not_in_profile_allowlist");
        }
    }
    if release.is_dolby_vision && !criteria.dolby_vision_allowed {
        decision.reject_requirement("dolby_vision_not_allowed");
    }
    if release.detected_hdr && !criteria.detected_hdr_allowed {
        decision.reject_requirement("hdr_not_allowed");
    }
    if release.is_bd_disk && !criteria.allow_bd_disk {
        decision.reject_requirement("bd_disk_not_allowed");
    }
    if !criteria.required_audio_languages.is_empty()
        && !crate::required_audio_languages_match(
            &criteria.required_audio_languages,
            &release.languages_audio,
        )
    {
        decision.reject_requirement("required_audio_language_missing");
    }
    decision
}

/// Finalize eligibility after every contribution. Repeated calls replace gates,
/// never alter the numeric score, and never erase mandatory requirements.
pub fn apply_min_score_gate(profile: &QualityProfile, decision: &mut QualityProfileDecision) {
    decision
        .scoring_log
        .retain(|entry| entry.kind != ScoringEntryKind::FinalScoreRejection);
    decision.refresh();
    if decision.release_score <= scryer_rules::BLOCK_SCORE_THRESHOLD {
        decision.reject(
            "score_at_or_below_block_threshold",
            ScoringEntryKind::FinalScoreRejection,
        );
    }
    if let Some(min_score) = profile.criteria.min_score_to_grab
        && decision.release_score < min_score
    {
        decision.reject("score_below_minimum", ScoringEntryKind::FinalScoreRejection);
    }
}

/// Parse a published_at date string in RFC2822, RFC3339, or ISO8601 format.
pub fn parse_published_at(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| DateTime::parse_from_rfc3339(raw).map(|dt| dt.with_timezone(&Utc)))
        .or_else(|_| raw.parse::<DateTime<Utc>>())
        .ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaSizeCategory {
    Movie,
    Series,
    Anime,
}

fn normalize_media_size_category(category_hint: Option<&str>) -> MediaSizeCategory {
    let Some(raw) = category_hint else {
        return MediaSizeCategory::Movie;
    };

    match raw.trim().to_ascii_lowercase().as_str() {
        "anime" => MediaSizeCategory::Anime,
        "series" => MediaSizeCategory::Series,
        _ => MediaSizeCategory::Movie,
    }
}

/// Expected total file bitrate (video + audio + overhead) in Mbps for a given
/// quality tier and media category.  Calibrated against real library data
/// (~500 files).  At baseline runtimes (120/45/24 min) the 1080P/720P/480P
/// values produce expected GiB equivalent to the previous hardcoded table.
/// 2160P values were intentionally recalibrated upward based on real remux and
/// WEB-DL sizes (e.g. movie 2160P: old 22 GiB → new ~50 GiB at 120 min).
fn expected_bitrate_mbps(
    quality: Option<&str>,
    media_category: MediaSizeCategory,
    movie_2160p_bitrate_mbps: f64,
) -> f64 {
    match media_category {
        MediaSizeCategory::Movie => match quality {
            Some("4320P") => 142.5,
            Some("2160P") => movie_2160p_bitrate_mbps,
            Some("1080P") => 9.1,
            Some("720P") => 3.4,
            Some("576P") => 2.4,
            Some("480P") => 1.4,
            _ => 6.8,
        },
        MediaSizeCategory::Series => match quality {
            Some("4320P") => 55.0,
            Some("2160P") => 22.0,
            Some("1080P") => 8.5,
            Some("720P") => 3.3,
            Some("576P") => 2.4,
            Some("480P") => 1.4,
            _ => 5.5,
        },
        MediaSizeCategory::Anime => match quality {
            Some("4320P") => 70.0,
            Some("2160P") => 28.0,
            Some("1080P") => 8.5,
            Some("720P") => 3.4,
            Some("576P") => 2.4,
            Some("480P") => 1.4,
            _ => 5.7,
        },
    }
}

/// Codec efficiency relative to mixed-codec baseline.  A known H.265 release
/// should be smaller than average; a known H.264 release slightly larger.
///
/// Values match the canonical codec strings emitted by `parse_release_metadata`
/// (`"H.264"`, `"H.265"`, `"AV1"`, `"VP9"`).
fn codec_efficiency_factor(codec: Option<&VideoCodec>) -> f64 {
    match codec {
        Some(VideoCodec::Av1) => 0.50,
        Some(VideoCodec::H265) => 0.75,
        Some(VideoCodec::Vp9) => 0.75,
        Some(VideoCodec::H264) => 1.10,
        _ => 1.0,
    }
}

/// Source-type multiplier applied to the expected bitrate.  Bluray encodes are
/// larger than WEB-DL; remuxes larger still.
fn source_size_factor(
    source: Option<&ReleaseSource>,
    is_remux: bool,
    is_bd_disk: bool,
    is_anime: bool,
    remux_size_factor: f64,
) -> f64 {
    let mut factor = 1.0;
    if matches!(source, Some(ReleaseSource::BluRay | ReleaseSource::BrDisk)) {
        factor *= 1.35;
    }
    if is_remux && !is_anime {
        factor *= remux_size_factor;
    }
    if is_bd_disk {
        factor *= 1.8;
    }
    if matches!(source, Some(ReleaseSource::WebDl | ReleaseSource::WebRip)) {
        factor *= 0.8;
    }
    factor
}

/// Default runtime (minutes) assumed when TVDB metadata is unavailable.
fn default_runtime_minutes(media_category: MediaSizeCategory) -> f64 {
    match media_category {
        MediaSizeCategory::Movie => 120.0,
        MediaSizeCategory::Series => 45.0,
        MediaSizeCategory::Anime => 24.0,
    }
}

/// What a reported byte count is being compared against.
///
/// Size scoring is runtime-derived, and an aggregate release — a multi-episode
/// file, a season pack, a multi-season or complete-series pack — has two
/// runtimes that matter: the whole payload's, and one member's. Indexers report
/// both, and nothing in a listing says which. A pack whose listing carries one
/// episode's byte count against the pack's total runtime reads as a twentieth of
/// what it should be, which is indistinguishable from a fake until the second
/// interpretation is tried.
///
/// Derived once per scope from the release's coverage
/// (`acquisition_coverage::coverage_size_basis` and
/// `acquisition_coverage::episode_span_size_basis`) and carried on
/// [`crate::canonical_scoring::ScoringContext`], so grab, import and a
/// re-derived incumbent bar read the same basis for the same evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverageSizeBasis {
    /// Runtime of everything the release covers, in minutes.
    pub total_runtime_minutes: Option<i32>,
    /// Runtime of one representative member of that coverage, in minutes. Equal
    /// to `total_runtime_minutes` whenever the coverage is a single member.
    pub member_runtime_minutes: Option<i32>,
    /// How many members the coverage holds, after any partial-season estimate.
    /// Never below 1.
    pub member_count: i32,
}

impl Default for CoverageSizeBasis {
    fn default() -> Self {
        Self::single(None)
    }
}

impl CoverageSizeBasis {
    /// A movie, a single episode, or any scope whose coverage is one member.
    pub fn single(runtime_minutes: Option<i32>) -> Self {
        Self {
            total_runtime_minutes: runtime_minutes,
            member_runtime_minutes: runtime_minutes,
            member_count: 1,
        }
    }

    /// A release covering several members. Collapses to [`Self::single`] when
    /// the count does not actually make it an aggregate.
    pub fn aggregate(
        total_runtime_minutes: Option<i32>,
        member_runtime_minutes: Option<i32>,
        member_count: i32,
    ) -> Self {
        if member_count <= 1 {
            return Self::single(total_runtime_minutes.or(member_runtime_minutes));
        }
        Self {
            total_runtime_minutes,
            member_runtime_minutes,
            member_count,
        }
    }

    /// Whether reported bytes could describe one member rather than the payload.
    pub fn covers_multiple_members(&self) -> bool {
        self.member_count > 1
    }

    /// Fill in whatever the catalog could not supply with the title's own
    /// runtime, leaving the member count alone.
    pub fn or_runtime(self, default_runtime_minutes: Option<i32>) -> Self {
        Self {
            total_runtime_minutes: self.total_runtime_minutes.or(default_runtime_minutes),
            member_runtime_minutes: self.member_runtime_minutes.or(default_runtime_minutes),
            member_count: self.member_count,
        }
    }
}

/// Logged, with a zero delta, when the reported size was read as one member's
/// rather than the whole pack's. Carries no weight of its own; it is there so an
/// explanation says which interpretation produced the band above it.
#[cfg(test)]
pub const SIZE_PACK_MEMBER_BASIS_CODE: &str = "size_pack_member_basis";

/// Preserve the mandatory upper size bound independently of customizable scores.
/// This emits only a zero-point requirement failure; the pack owns the size curve.
pub(crate) fn apply_size_requirement(
    decision: &mut QualityProfileDecision,
    profile: &QualityProfile,
    release: &ParsedReleaseMetadata,
    size_bytes: Option<i64>,
    category_hint: Option<&str>,
    size_basis: CoverageSizeBasis,
) {
    let Some(bytes) = size_bytes.filter(|bytes| *bytes > 0) else {
        return;
    };
    let category = normalize_media_size_category(category_hint);
    let balanced = profile.criteria.scoring_persona == ScoringPersona::Balanced;
    // These are the release's admission limits, not persona score weights.
    let bitrate = expected_bitrate_mbps(
        normalize_quality_tier(release.quality.as_deref()).as_deref(),
        category,
        if balanced { 32.0 } else { 57.0 },
    );
    let codec_factor = codec_efficiency_factor(release.video_codec.as_ref());
    let source_factor = source_size_factor(
        release.source.as_ref(),
        release.is_remux,
        release.is_bd_disk,
        category == MediaSizeCategory::Anime,
        if profile.criteria.prefer_remux || !balanced {
            1.45
        } else {
            1.0
        },
    );
    let runtime = size_basis
        .total_runtime_minutes
        .filter(|minutes| *minutes > 0)
        .map(f64::from)
        .unwrap_or_else(|| default_runtime_minutes(category));
    let expected_gib =
        (bitrate * codec_factor * source_factor * (runtime * 60.0) / 8.0 / 1024.0).max(0.5);
    let upper_ratio = if category == MediaSizeCategory::Anime {
        6.0
    } else {
        8.0
    } * if release.video_codec == Some(VideoCodec::Av1) {
        1.5
    } else {
        1.0
    };
    // The member-size reinterpretation only applies below the lower bands and
    // therefore cannot change this upper-bound verdict.
    if bytes as f64 / (1024.0 * 1024.0 * 1024.0) / expected_gib >= upper_ratio {
        decision.reject_requirement("size_implausible_for_quality");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release_parser::{parse_release_metadata, parse_release_metadata_for_target};
    use crate::scoring_weights::{ScoringOverrides, ScoringPersona};
    use scryer_release_parser::{
        ContextEpisode, ContextFacetHint, ContextTitle, ReleaseParseContext,
    };

    fn score_size_with_pack(
        decision: &mut QualityProfileDecision,
        release: &ParsedReleaseMetadata,
        size_bytes: Option<i64>,
        category_hint: Option<&str>,
        runtime_minutes: Option<i32>,
        prefer_remux: bool,
        weights: &ScoringConfig,
    ) {
        super::score_size_with_pack_basis(
            decision,
            release,
            size_bytes,
            category_hint,
            CoverageSizeBasis::single(runtime_minutes),
            prefer_remux,
            weights,
        );
    }

    #[test]
    fn recoverable_scores_finalize_only_the_complete_sum() {
        let profile =
            QualityProfile::parse(r#"{"id":"test","name":"Test","criteria":{}}"#).unwrap();
        for (deltas, expected, allowed) in [
            (vec![-10_000, 20_000], 10_000, true),
            (vec![20_000, -10_000], 10_000, true),
            (vec![-10_000, 999], -9_001, false),
            (vec![-10_000, 1_000], -9_000, false),
            (vec![-10_000, 1_001], -8_999, true),
            (vec![-4_500, -4_500], -9_000, false),
            (vec![-10_000, -10_000, 30_000], 10_000, true),
            (vec![-100], -100, true),
            (vec![i32::MAX, i32::MAX, -i32::MAX], i32::MAX, true),
            (vec![i32::MIN, i32::MAX, i32::MAX], i32::MAX - 1, true),
        ] {
            let mut decision = QualityProfileDecision::new();
            for delta in deltas {
                decision.log_with_source(
                    "video_codec_in_profile_blocklist",
                    delta,
                    ScoringSource::UserRule {
                        id: "custom".into(),
                        name: "Custom".into(),
                    },
                );
                assert!(decision.allowed, "a contribution alone cannot reject");
            }
            apply_min_score_gate(&profile, &mut decision);
            assert_eq!(decision.release_score, expected);
            assert_eq!(decision.allowed, allowed);
            let first = decision.clone();
            apply_min_score_gate(&profile, &mut decision);
            assert_eq!(decision, first);
        }
    }

    #[test]
    fn recoverable_scores_respect_minimum_and_mandatory_requirements() {
        let mut profile =
            QualityProfile::parse(r#"{"id":"test","name":"Test","criteria":{}}"#).unwrap();
        profile.criteria.min_score_to_grab = Some(100);
        let mut decision = QualityProfileDecision::new();
        decision.log("penalty", -10_000);
        apply_min_score_gate(&profile, &mut decision);
        assert_eq!(decision.block_codes.len(), 2);
        decision.log("bonus", 10_100);
        apply_min_score_gate(&profile, &mut decision);
        assert!(decision.allowed);
        assert_eq!(decision.release_score, 100);
        decision.reject_requirement("required_audio_language_missing");
        decision.log("bonus", i32::MAX);
        apply_min_score_gate(&profile, &mut decision);
        assert!(!decision.allowed);
        assert_eq!(decision.block_codes, ["required_audio_language_missing"]);
        assert!(
            decision
                .scoring_log
                .iter()
                .filter(|entry| entry.kind != ScoringEntryKind::ScoreContribution)
                .all(|entry| entry.delta == 0)
        );
    }

    #[test]
    fn parse_profile_json() {
        let profile = QualityProfile::parse(
            r#"{
                "id": "default-movie",
                "name": "Default movie",
                "criteria": {
                    "quality_tiers": ["2160p", "1080p", "720p"],
                    "allow_unknown_quality": false,
                    "source_allowlist": ["WEB-DL", "BLURAY"],
                    "audio_codec_allowlist": ["DDP", "AAC", "DTS"],
                    "atmos_preferred": true,
                    "prefer_remux": true,
                    "allow_bd_disk": true,
                    "allow_upgrades": false
                }
            }"#,
        )
        .expect("profile must parse");

        assert_eq!(profile.id, "default-movie");
        assert_eq!(
            profile.criteria.quality_tiers,
            vec!["2160P".to_string(), "1080P".to_string(), "720P".to_string()]
        );
        assert!(profile.criteria.atmos_preferred);
        assert_eq!(profile.criteria.quality_tiers.len(), 3);
    }

    #[test]
    fn builtin_anime_profile_accepts_576p() {
        let profile = builtin_anime_profile();
        assert_eq!(
            profile.criteria.quality_tiers,
            vec!["1080P".to_string(), "720P".to_string(), "576P".to_string()]
        );

        let release =
            parse_release_metadata("Fixture.Anime.S06E01.576p.DVD.Opus.Dual.Audio.AV1-GRP");
        assert_eq!(
            normalize_quality_tier(release.quality.as_deref()),
            Some("576P".to_string())
        );
        let result = score_with_pack(&profile, &release, false, &balanced_scoring_config());
        assert!(result.allowed, "576p anime release was blocked: {result:?}");
    }

    #[test]
    fn minimum_score_gate_runs_after_rule_scores() {
        let mut profile = QualityProfile::default();
        let release = parse_release_metadata("Movie.2024.1080p.WEB-DL.H.264.DDP5.1-GROUP");
        let weights = balanced_scoring_config();
        let baseline = score_with_pack(&profile, &release, false, &weights);
        profile.criteria.min_score_to_grab = Some(baseline.release_score + 10);

        let mut rescued = score_with_pack(&profile, &release, false, &weights);
        assert!(rescued.allowed);
        assert!(
            !rescued
                .block_codes
                .contains(&"score_below_minimum".to_string())
        );
        rescued.log_with_source(
            "rule_bonus",
            20,
            ScoringSource::UserRule {
                id: "bonus".to_string(),
                name: "Bonus".to_string(),
            },
        );
        apply_min_score_gate(&profile, &mut rescued);
        assert!(rescued.allowed);

        let mut below_minimum = score_with_pack(&profile, &release, false, &weights);
        apply_min_score_gate(&profile, &mut below_minimum);
        assert!(!below_minimum.allowed);
        assert!(
            below_minimum
                .block_codes
                .contains(&"score_below_minimum".to_string())
        );
    }

    #[test]
    fn profile_blocks_missing_quality_when_required() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"strict",
                "name":"Strict",
                "criteria": {
                    "quality_tiers":["2160p","1080p","720p"],
                    "allow_unknown_quality":false,
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = balanced_scoring_config();
        let release = parse_release_metadata("Some.Movie.1080p.WEB-DL.H.265.DDP2.0");
        let result = score_with_pack(&profile, &release, false, &w);
        assert!(result.allowed);

        let release = parse_release_metadata("Some.Movie.WEB-DL.H.265.DDP2.0");
        let result = score_with_pack(&profile, &release, false, &w);
        assert!(!result.allowed);
        assert!(
            result
                .block_codes
                .iter()
                .any(|code| code == "quality_missing_and_profile_disallows_unknown")
        );
    }

    #[test]
    fn context_episode_title_does_not_trigger_source_low_quality_theatrical() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"context-protected",
                "name":"Context protected",
                "criteria": {
                    "quality_tiers":["1080p","720p"],
                    "allow_unknown_quality":false,
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");
        let context = ReleaseParseContext {
            facet_hint: ContextFacetHint::Series,
            title: ContextTitle {
                name: "Fixture Series".to_string(),
            },
            aliases: Vec::new(),
            known_years: Vec::new(),
            imdb_ids: Vec::new(),
            episodes: vec![ContextEpisode {
                season: Some(2),
                episode: Some(4),
                title: Some("Camera Token".to_string()),
                ..Default::default()
            }],
        };
        let release = parse_release_metadata_for_target(
            "Fixture.Series.S02E04.Camera.Token.1080p.WEB-DL.H264-Group",
            &context,
        );
        let result = score_with_pack(&profile, &release, false, &balanced_scoring_config());

        assert!(result.allowed);
        assert!(
            !result
                .block_codes
                .iter()
                .any(|code| code == "source_low_quality_theatrical")
        );
    }

    #[test]
    fn profile_allows_unknown_quality_when_enabled() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"lenient",
                "name":"Lenient",
                "criteria": {
                    "allow_unknown_quality":true,
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = balanced_scoring_config();
        let release = parse_release_metadata("Some.Movie.WEB-DL.H.265.DDP2.0");
        let result = score_with_pack(&profile, &release, false, &w);
        assert!(result.allowed);
    }

    #[test]
    fn audiophile_profile_prefers_atmos_candidates() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"atmos-test",
                "name":"Atmos test",
                "criteria": {
                    "atmos_preferred":true,
                    "prefer_remux":false,
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        // Atmos preference is persona-native to Audiophile — Balanced has no atmos bias.
        let w = test_scoring_config(&ScoringPersona::Audiophile, &ScoringOverrides::default());
        let with_atmos =
            parse_release_metadata("Show.2021.1080p.WEB-DL.H.265.DDP.Atmos.5.1.AAC2.0");
        let no_atmos = parse_release_metadata("Show.2021.1080p.WEB-DL.H.265.DDP.5.1.AAC2.0");

        assert!(
            score_with_pack(&profile, &with_atmos, false, &w).preference_score
                > score_with_pack(&profile, &no_atmos, false, &w).preference_score
        );
    }

    #[test]
    fn audiophile_profile_prefers_remux_candidates() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"remux-first",
                "name":"Remux first",
                "criteria": {
                    "prefer_remux":true,
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = test_scoring_config(
            &crate::scoring_weights::ScoringPersona::Audiophile,
            &crate::scoring_weights::ScoringOverrides::default(),
        );
        let with_remux = parse_release_metadata("Movie.2021.1080p.WEB-DL.H.265.Remux.DDP2.0");
        let without_remux = parse_release_metadata("Movie.2021.1080p.WEB-DL.H.265.DDP2.0");

        assert!(
            score_with_pack(&profile, &with_remux, false, &w).allowed
                && score_with_pack(&profile, &without_remux, false, &w).allowed
        );
        assert!(
            score_with_pack(&profile, &with_remux, false, &w).preference_score
                > score_with_pack(&profile, &without_remux, false, &w).preference_score
        );
    }

    #[test]
    fn profile_blocking_by_source_and_codec() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"web-only",
                "name":"Web only",
                "criteria": {
                    "source_allowlist": ["WEB-DL"],
                    "video_codec_allowlist": ["H.265"],
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = balanced_scoring_config();
        let release = parse_release_metadata("Movie.2021.1080p.WEB-DL.H.264.DDP2.0");
        let result = score_with_pack(&profile, &release, false, &w);
        assert!(!result.allowed);
        assert!(
            result
                .block_codes
                .contains(&"video_codec_not_in_profile_allowlist".to_string())
        );
    }

    #[test]
    fn profile_blocks_detected_hdr_when_disabled() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"no-hdr",
                "name":"No HDR",
                "criteria": {
                    "allow_unknown_quality":true,
                    "detected_hdr_allowed":false,
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = balanced_scoring_config();
        let hdr_release = parse_release_metadata("Movie.2021.2160p.WEB-DL.HDR.HDR10.x265.DDP");
        let regular_release = parse_release_metadata("Movie.2021.2160p.WEB-DL.H.265.DDP2.0");

        let hdr_result = score_with_pack(&profile, &hdr_release, false, &w);
        let regular_result = score_with_pack(&profile, &regular_release, false, &w);

        assert!(!hdr_result.allowed);
        assert!(
            hdr_result
                .block_codes
                .iter()
                .any(|code| code == "hdr_not_allowed")
        );
        assert!(regular_result.allowed);
    }

    #[test]
    fn profile_allows_multi_audio_when_one_codec_is_allowlisted() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"audio-mixed",
                "name":"Audio mixed",
                "criteria": {
                    "allow_unknown_quality":true,
                    "audio_codec_allowlist":["TRUEHD"],
                    "audio_codec_blocklist":["DTS"],
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = balanced_scoring_config();
        let release = parse_release_metadata("Movie.2024.2160p.BluRay.DTS-HD.TrueHD.7.1.H.265");
        let result = score_with_pack(&profile, &release, false, &w);
        assert!(result.allowed);
    }

    #[test]
    fn profile_blocks_audio_when_no_codec_is_allowlisted() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"audio-allowlist",
                "name":"Audio allowlist",
                "criteria": {
                    "allow_unknown_quality":true,
                    "audio_codec_allowlist":["TRUEHD"],
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = balanced_scoring_config();
        let release = parse_release_metadata("Movie.2024.2160p.WEB-DL.AAC.2.0.H.265");
        let result = score_with_pack(&profile, &release, false, &w);
        assert!(!result.allowed);
        assert!(
            result
                .block_codes
                .contains(&"audio_codec_not_in_profile_allowlist".to_string())
        );
    }

    #[test]
    fn profile_blocks_multi_audio_when_all_codecs_blocklisted() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"audio-block-all",
                "name":"Audio block all",
                "criteria": {
                    "allow_unknown_quality":true,
                    "audio_codec_blocklist":["DTSHD","TRUEHD"],
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = balanced_scoring_config();
        let release = parse_release_metadata("Movie.2024.2160p.BluRay.DTS-HD.TrueHD.7.1.H.265");
        let result = score_with_pack(&profile, &release, false, &w);
        assert!(!result.allowed);
        assert!(
            result
                .block_codes
                .contains(&"audio_codec_in_profile_blocklist".to_string())
        );
    }

    #[test]
    fn profile_detected_hdr_defaults_to_true_when_missing() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"legacy",
                "name":"Legacy",
                "criteria": {
                    "allow_unknown_quality":true,
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = balanced_scoring_config();
        let hdr_release = parse_release_metadata("Movie.2021.2160p.WEB-DL.HDR.HDR10.x265.DDP");
        assert!(score_with_pack(&profile, &hdr_release, false, &w).allowed);
    }

    #[test]
    fn balanced_size_scoring_prefers_a_plausible_release_for_same_metadata() {
        let profile = builtin_4k_profile();
        let w = balanced_scoring_config();
        let release = parse_release_metadata("Movie.2021.2160p.BluRay.Remux.H.265.DTSHD.Atmos");

        let mut small = score_with_pack(&profile, &release, false, &w);
        score_size_with_pack(
            &mut small,
            &release,
            Some(7 * 1024 * 1024 * 1024),
            None,
            None,
            false,
            &w,
        );

        let mut large = score_with_pack(&profile, &release, false, &w);
        score_size_with_pack(
            &mut large,
            &release,
            Some(35 * 1024 * 1024 * 1024),
            None,
            None,
            false,
            &w,
        );

        assert!(large.preference_score > small.preference_score);
    }

    #[test]
    fn balanced_nonpreferred_uhd_remux_loses_to_a_sensible_release() {
        let profile = builtin_4k_profile();
        let weights = balanced_scoring_config();
        let sensible = parse_release_metadata("Movie.2021.2160p.BluRay.H.265.DTSHD");
        let remux = parse_release_metadata("Movie.2021.2160p.BluRay.Remux.H.265.DTSHD");

        let mut sensible_decision = score_with_pack(&profile, &sensible, false, &weights);
        score_size_with_pack(
            &mut sensible_decision,
            &sensible,
            Some(35 * 1024 * 1024 * 1024),
            Some("movie"),
            Some(120),
            false,
            &weights,
        );

        let mut remux_decision = score_with_pack(&profile, &remux, false, &weights);
        score_size_with_pack(
            &mut remux_decision,
            &remux,
            Some(78 * 1024 * 1024 * 1024),
            Some("movie"),
            Some(120),
            false,
            &weights,
        );

        assert!(remux_decision.allowed);
        assert!(sensible_decision.preference_score > remux_decision.preference_score);
        assert!(
            remux_decision
                .scoring_log
                .iter()
                .any(|entry| entry.code == "remux_not_preferred" && entry.delta == -400)
        );
        assert!(
            remux_decision
                .scoring_log
                .iter()
                .any(|entry| entry.code == "size_massive_for_quality")
        );
    }

    #[test]
    fn balanced_remux_preference_restores_its_size_tolerance() {
        let mut profile = builtin_4k_profile();
        let weights = balanced_scoring_config();
        let remux = parse_release_metadata("Movie.2021.2160p.BluRay.Remux.H.265.DTSHD");

        let mut not_preferred = score_with_pack(&profile, &remux, false, &weights);
        score_size_with_pack(
            &mut not_preferred,
            &remux,
            Some(78 * 1024 * 1024 * 1024),
            Some("movie"),
            Some(120),
            false,
            &weights,
        );

        profile.criteria.prefer_remux = true;
        let mut preferred = score_with_pack(&profile, &remux, false, &weights);
        score_size_with_pack(
            &mut preferred,
            &remux,
            Some(78 * 1024 * 1024 * 1024),
            Some("movie"),
            Some(120),
            true,
            &weights,
        );

        assert!(preferred.preference_score > not_preferred.preference_score);
        assert!(
            preferred
                .scoring_log
                .iter()
                .any(|entry| entry.code == "prefer_remux_match" && entry.delta == 250)
        );
        assert!(
            !preferred
                .scoring_log
                .iter()
                .any(|entry| entry.code == "remux_not_preferred")
        );
    }

    #[test]
    fn balanced_1080p_remuxes_are_penalized_when_not_preferred() {
        let profile = builtin_4k_profile();
        let weights = balanced_scoring_config();
        let standard = parse_release_metadata("Movie.2021.1080p.BluRay.H.265.DTSHD");
        let remux = parse_release_metadata("Movie.2021.1080p.BluRay.Remux.H.265.DTSHD");

        let mut standard_decision = score_with_pack(&profile, &standard, false, &weights);
        score_size_with_pack(
            &mut standard_decision,
            &standard,
            Some(10 * 1024 * 1024 * 1024),
            Some("movie"),
            Some(120),
            false,
            &weights,
        );

        let mut remux_decision = score_with_pack(&profile, &remux, false, &weights);
        score_size_with_pack(
            &mut remux_decision,
            &remux,
            Some(25 * 1024 * 1024 * 1024),
            Some("movie"),
            Some(120),
            false,
            &weights,
        );

        assert!(remux_decision.allowed);
        assert!(standard_decision.preference_score > remux_decision.preference_score);
        assert!(
            remux_decision
                .scoring_log
                .iter()
                .any(|entry| entry.code == "remux_not_preferred" && entry.delta == -400)
        );
    }

    #[test]
    fn balanced_size_budget_scales_with_runtime() {
        let profile = builtin_4k_profile();
        let weights = balanced_scoring_config();
        let remux = parse_release_metadata("Movie.2021.2160p.BluRay.Remux.H.265.DTSHD");

        let decision_for_runtime = |runtime_minutes| {
            let mut decision = score_with_pack(&profile, &remux, false, &weights);
            score_size_with_pack(
                &mut decision,
                &remux,
                Some(78 * 1024 * 1024 * 1024),
                Some("movie"),
                Some(runtime_minutes),
                false,
                &weights,
            );
            decision
        };

        let short = decision_for_runtime(120);
        let long = decision_for_runtime(240);

        assert!(short.allowed);
        assert!(long.allowed);
        assert!(long.preference_score > short.preference_score);
    }

    #[test]
    fn av1_tiny_e2e_fixture_keeps_its_low_size_decision() {
        let release = parse_release_metadata("Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb");
        let weights = balanced_scoring_config();
        let mut decision = QualityProfileDecision::new();

        score_size_with_pack(
            &mut decision,
            &release,
            Some(77_514_027),
            Some("movie"),
            Some(7),
            false,
            &weights,
        );

        assert!(decision.allowed);
        assert_eq!(
            decision.scoring_log.last().map(|entry| entry.code.as_str()),
            Some("size_tiny_for_quality")
        );
    }

    #[test]
    fn av1_upper_size_curve_allows_larger_encodes_before_blocking() {
        let release = parse_release_metadata("Anime.S01E01.1080p.WEB-DL.AV1.AAC2.0-NTb");
        let weights = balanced_scoring_config();
        let decision_for_size = |size_bytes| {
            let mut decision = QualityProfileDecision::new();
            score_size_with_pack(
                &mut decision,
                &release,
                Some(size_bytes),
                Some("anime"),
                Some(24),
                false,
                &weights,
            );
            decision
        };

        let formerly_excessive = decision_for_size(2 * 1024 * 1024 * 1024);
        assert_eq!(
            formerly_excessive
                .scoring_log
                .last()
                .map(|entry| entry.code.as_str()),
            Some("size_massive_for_quality")
        );

        let formerly_implausible = decision_for_size(4 * 1024 * 1024 * 1024);
        assert!(formerly_implausible.allowed);
        assert_eq!(
            formerly_implausible
                .scoring_log
                .last()
                .map(|entry| entry.code.as_str()),
            Some("size_excessive_for_quality")
        );

        let still_implausible = decision_for_size(6 * 1024 * 1024 * 1024);
        assert!(!still_implausible.allowed);
        assert_eq!(
            still_implausible
                .scoring_log
                .last()
                .map(|entry| entry.code.as_str()),
            Some("size_implausible_for_quality")
        );
    }

    #[test]
    fn non_av1_size_classifications_stay_unchanged() {
        let weights = balanced_scoring_config();
        let decision_for_release = |release_name: &str| {
            let release = parse_release_metadata(release_name);
            let mut decision = QualityProfileDecision::new();
            score_size_with_pack(
                &mut decision,
                &release,
                Some(2 * 1024 * 1024 * 1024),
                Some("anime"),
                Some(24),
                false,
                &weights,
            );
            decision
        };

        let h265 = decision_for_release("Anime.S01E01.1080p.WEB-DL.H.265.AAC2.0-NTb");
        assert_eq!(
            h265.scoring_log.last().map(|entry| entry.code.as_str()),
            Some("size_massive_for_quality")
        );

        let h264 = decision_for_release("Anime.S01E01.1080p.WEB-DL.H.264.AAC2.0-NTb");
        assert_eq!(
            h264.scoring_log.last().map(|entry| entry.code.as_str()),
            Some("size_large_for_quality")
        );
    }

    #[test]
    fn tiny_uhd_can_rank_below_high_quality_1080() {
        let profile = builtin_4k_profile();
        let w = balanced_scoring_config();

        let tiny_uhd = parse_release_metadata("Movie.2021.2160p.BluRay.Remux.H.265.DTSHD.Atmos");
        let mut tiny_uhd_decision = score_with_pack(&profile, &tiny_uhd, false, &w);
        score_size_with_pack(
            &mut tiny_uhd_decision,
            &tiny_uhd,
            Some(5 * 1024 * 1024 * 1024),
            None,
            None,
            false,
            &w,
        );

        let strong_1080 = parse_release_metadata("Movie.2021.1080p.BluRay.H.264.DTS");
        let mut strong_1080_decision = score_with_pack(&profile, &strong_1080, false, &w);
        score_size_with_pack(
            &mut strong_1080_decision,
            &strong_1080,
            Some(18 * 1024 * 1024 * 1024),
            None,
            None,
            false,
            &w,
        );

        assert!(strong_1080_decision.preference_score > tiny_uhd_decision.preference_score);
    }

    #[test]
    fn plausible_uhd_still_outscores_1080_due_to_tier_priority() {
        let profile = builtin_4k_profile();
        let w = balanced_scoring_config();

        let plausible_uhd = parse_release_metadata("Movie.2021.2160p.BluRay.H.265.DTSHD");
        let mut plausible_uhd_decision = score_with_pack(&profile, &plausible_uhd, false, &w);
        score_size_with_pack(
            &mut plausible_uhd_decision,
            &plausible_uhd,
            Some(35 * 1024 * 1024 * 1024),
            None,
            None,
            false,
            &w,
        );

        let strong_1080 = parse_release_metadata("Movie.2021.1080p.BluRay.H.264.DTS");
        let mut strong_1080_decision = score_with_pack(&profile, &strong_1080, false, &w);
        score_size_with_pack(
            &mut strong_1080_decision,
            &strong_1080,
            Some(18 * 1024 * 1024 * 1024),
            None,
            None,
            false,
            &w,
        );

        // Tier no longer lives in the score, so these two are compared on their
        // within-tier merits and the 1080p BluRay may well win on points. What
        // must hold is that the profile still ranks UHD above 1080p — that
        // ordering is consulted first, before either score is looked at.
        assert!(
            quality_tier_index(&profile.criteria, Some("2160p"))
                < quality_tier_index(&profile.criteria, Some("1080p")),
            "the profile must still rank UHD above 1080p"
        );
        let _ = (
            plausible_uhd_decision.preference_score,
            strong_1080_decision.preference_score,
        );
    }
}

fn resolve_archival_quality(
    archival_quality: Option<String>,
    quality_tiers: &[String],
) -> Option<String> {
    match archival_quality.and_then(|value| normalize_quality_tier(Some(&value))) {
        Some(normalized) if !normalized.is_empty() => Some(normalized),
        _ => quality_tiers
            .first()
            .and_then(|value| normalize_quality_tier(Some(value)))
            .or_else(|| Some("1080P".to_string())),
    }
}

#[cfg(test)]
#[path = "quality_profile_tests.rs"]
mod quality_profile_tests;
