use crate::release_group_db::apply_release_group_scoring;
use crate::release_parser::ParsedReleaseMetadata;
use crate::scoring_weights::{
    ScoringOverrides, ScoringPersona, ScoringWeights, audio_weight_for_codec,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct QualityProfile {
    pub id: String,
    pub name: String,
    pub criteria: QualityProfileCriteria,
}

#[derive(Debug, Clone, Serialize)]
pub struct QualityProfileCriteria {
    pub quality_tiers: Vec<String>,
    pub archival_quality: Option<String>,
    pub allow_unknown_quality: bool,
    pub source_allowlist: Vec<String>,
    pub source_blocklist: Vec<String>,
    pub video_codec_allowlist: Vec<String>,
    pub video_codec_blocklist: Vec<String>,
    pub audio_codec_allowlist: Vec<String>,
    pub audio_codec_blocklist: Vec<String>,
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
    pub min_score_to_grab: Option<i32>,
    pub facet_persona_overrides: HashMap<String, ScoringPersona>,
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

/// Score applied to any blocking rule. Massive negative value so blocked releases
/// always sort below considered ones regardless of other bonuses.
pub const BLOCK_SCORE: i32 = -10_000;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityProfileDecision {
    /// Sum of all `scoring_log` deltas.
    pub release_score: i32,
    /// Every decision point in the order it was applied.
    pub scoring_log: Vec<ScoringEntry>,
    /// Derived: true when no entry has `delta == BLOCK_SCORE`.
    pub allowed: bool,
    /// Derived: codes from entries where `delta == BLOCK_SCORE`.
    pub block_codes: Vec<String>,
    /// Kept equal to `release_score` so existing sort logic works without changes.
    pub preference_score: i32,
}

impl QualityProfileDecision {
    fn new() -> Self {
        Self {
            release_score: 0,
            scoring_log: Vec::new(),
            allowed: true,
            block_codes: Vec::new(),
            preference_score: 0,
        }
    }

    /// Record a decision point and keep the derived fields consistent.
    fn log(&mut self, code: &str, delta: i32) {
        self.log_with_source(code, delta, ScoringSource::Builtin);
    }

    /// Record a decision point with an explicit source.
    pub fn log_with_source(&mut self, code: &str, delta: i32, source: ScoringSource) {
        self.scoring_log.push(ScoringEntry {
            code: code.to_string(),
            delta,
            source,
        });
        self.release_score += delta;
        if delta == BLOCK_SCORE {
            self.allowed = false;
            self.block_codes.push(code.to_string());
        }
        self.preference_score = self.release_score;
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
pub const QUALITY_PROFILE_INHERIT_VALUE: &str = "__inherit__";
fn default_true() -> bool {
    true
}

pub fn parse_profile_catalog_from_json(
    raw_json: &str,
) -> Result<Vec<QualityProfile>, serde_json::Error> {
    let profiles = serde_json::from_str::<Vec<RawQualityProfile>>(raw_json)?;
    Ok(profiles.into_iter().map(quality_profile_from_raw).collect())
}

pub fn default_quality_profile_for_search() -> QualityProfile {
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
            atmos_preferred: true,
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
            facet_persona_overrides: HashMap::new(),
        },
    }
}

pub fn default_quality_profile_1080p_for_search() -> QualityProfile {
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
            atmos_preferred: true,
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
            facet_persona_overrides: HashMap::new(),
        },
    }
}

fn quality_profile_from_raw(raw: RawQualityProfile) -> QualityProfile {
    let criteria = raw.criteria;
    let quality_tiers = normalize_list(criteria.quality_tiers);
    let archival_quality = resolve_archival_quality(criteria.archival_quality, &quality_tiers);
    QualityProfile {
        id: raw.id,
        name: raw.name,
        criteria: QualityProfileCriteria {
            quality_tiers,
            archival_quality,
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist: normalize_list(criteria.source_allowlist),
            source_blocklist: normalize_list(criteria.source_blocklist),
            video_codec_allowlist: normalize_list(criteria.video_codec_allowlist),
            video_codec_blocklist: normalize_list(criteria.video_codec_blocklist),
            audio_codec_allowlist: normalize_list(criteria.audio_codec_allowlist),
            audio_codec_blocklist: normalize_list(criteria.audio_codec_blocklist),
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
            facet_persona_overrides: criteria.facet_persona_overrides,
        },
    }
}

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
            facet_persona_overrides: HashMap::new(),
        }
    }
}

impl QualityProfile {
    pub fn parse(raw_json: &str) -> Result<Self, serde_json::Error> {
        let raw: RawQualityProfile = serde_json::from_str(raw_json)?;
        Ok(quality_profile_from_raw(raw))
    }
}

fn normalize_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn is_in_list(candidate: &str, list: &[String]) -> bool {
    list.iter().any(|value| value == candidate)
}

fn normalize_source(raw: Option<&str>) -> Option<String> {
    raw.map(
        |value| match value.to_ascii_uppercase().replace('-', "") as String {
            source if source == "WEBRIP" => "WEBRIP".to_string(),
            source if source == "WEBDL" || source == "WEB" || source == "WEB_DL" => {
                "WEB-DL".to_string()
            }
            source if source == "BRDISK" || source == "BDISO" || source == "BDMV" => {
                "BLURAY".to_string()
            }
            source
                if source == "BLURAY" || source == "BLU" || source == "BD" || source == "UHD" =>
            {
                "BLURAY".to_string()
            }
            source if source == "RAWHD" => "HDTV".to_string(),
            source => source,
        },
    )
}

fn normalize_codec(raw: Option<&str>) -> Option<String> {
    raw.map(|value| match value.to_ascii_uppercase().as_str() {
        "H264" => "H.264".to_string(),
        "H265" => "H.265".to_string(),
        value => value.to_string(),
    })
}

fn normalized_audio_codecs(release: &ParsedReleaseMetadata) -> Vec<String> {
    let mut codecs = Vec::<String>::new();

    for codec in &release.audio_codecs {
        if let Some(normalized) = normalize_codec(Some(codec.as_str()))
            && !codecs.iter().any(|existing| existing == &normalized)
        {
            codecs.push(normalized);
        }
    }

    if codecs.is_empty()
        && let Some(normalized) = normalize_codec(release.audio.as_deref())
    {
        codecs.push(normalized);
    }

    codecs
}

fn normalize_quality(raw: Option<&str>) -> Option<String> {
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

pub fn resolve_profile_id_for_title(
    title_profile_id: Option<&str>,
    category_profile_id: Option<&str>,
    global_profile_id: Option<&str>,
) -> Option<String> {
    title_profile_id
        .map(std::string::ToString::to_string)
        .or_else(|| category_profile_id.map(std::string::ToString::to_string))
        .or_else(|| global_profile_id.map(std::string::ToString::to_string))
}

/// Check whether the currently grabbed release has reached the cutoff quality tier.
///
/// Returns `true` when the existing file's quality is at or above the cutoff tier
/// in the profile's tier ordering. When true, callers should skip upgrade consideration.
///
/// `grabbed_release` is the raw release title stored on the wanted item.
/// `cutoff_tier` and `quality_tiers` come from the quality profile criteria.
pub fn has_reached_cutoff(
    grabbed_release: Option<&str>,
    cutoff_tier: Option<&str>,
    quality_tiers: &[String],
) -> bool {
    let Some(release_title) = grabbed_release else {
        return false;
    };
    let Some(cutoff) = cutoff_tier else {
        return false;
    };
    if quality_tiers.is_empty() {
        return false;
    }

    let cutoff_normalized = match normalize_quality(Some(cutoff)) {
        Some(q) => q,
        None => return false,
    };

    let cutoff_pos = match quality_tiers.iter().position(|t| t == &cutoff_normalized) {
        Some(pos) => pos,
        None => return false,
    };

    let parsed = crate::release_parser::parse_release_metadata(release_title);
    let current_quality = match normalize_quality(parsed.quality.as_deref()) {
        Some(q) => q,
        None => return false,
    };

    match quality_tiers.iter().position(|t| t == &current_quality) {
        Some(current_pos) => current_pos <= cutoff_pos,
        None => false,
    }
}

pub fn evaluate_against_profile(
    profile: &QualityProfile,
    release: &ParsedReleaseMetadata,
    has_existing_file: bool,
    weights: &ScoringWeights,
) -> QualityProfileDecision {
    let mut d = QualityProfileDecision::new();
    let c = &profile.criteria;

    // ── Upgrade guard ────────────────────────────────────────────────────────
    if !c.allow_upgrades && has_existing_file {
        d.log("upgrade_blocked_by_profile", BLOCK_SCORE);
    }

    // ── Quality tier ─────────────────────────────────────────────────────────
    match normalize_quality(release.quality.as_deref()) {
        Some(q) if !c.quality_tiers.is_empty() => {
            if let Some(idx) = c.quality_tiers.iter().position(|t| t == &q) {
                let bonus = match idx {
                    0 => 3200,
                    1 => 900,
                    2 => 300,
                    _ => (300_i32 - (idx as i32 - 2) * 125).max(50),
                };
                d.log(&format!("quality_tier_{idx}"), bonus);
            } else {
                d.log("quality_not_in_profile_tiers", BLOCK_SCORE);
            }
        }
        Some(_) => {}
        None => {
            if c.allow_unknown_quality {
                d.log("quality_unknown_allowed", 100);
            } else {
                d.log("quality_missing_and_profile_disallows_unknown", BLOCK_SCORE);
            }
        }
    }

    // ── Source ───────────────────────────────────────────────────────────────
    match normalize_source(release.source.as_deref()) {
        Some(source) => {
            let explicitly_allowed =
                !c.source_allowlist.is_empty() && is_in_list(&source, &c.source_allowlist);
            if matches!(
                source.as_str(),
                "CAM" | "TELESYNC" | "TELECINE" | "DVDSCR" | "WORKPRINT" | "REGIONAL"
            ) && !explicitly_allowed
            {
                d.log("source_low_quality_theatrical", BLOCK_SCORE);
            } else if !c.source_blocklist.is_empty() && is_in_list(&source, &c.source_blocklist) {
                d.log("source_in_profile_blocklist", BLOCK_SCORE);
            } else if !c.source_allowlist.is_empty() && !is_in_list(&source, &c.source_allowlist) {
                d.log("source_not_in_profile_allowlist", BLOCK_SCORE);
            } else {
                let (code, delta) = match source.as_str() {
                    "BLURAY" => ("source_bluray", weights.source_bluray),
                    "WEB-DL" => ("source_webdl", weights.source_webdl),
                    "WEBRIP" => ("source_webrip", weights.source_webrip),
                    "HDTV" => ("source_hdtv", weights.source_hdtv),
                    _ => ("source_other", 0),
                };
                if delta != 0 {
                    d.log(code, delta);
                }
            }
        }
        None => {
            if !c.source_allowlist.is_empty() {
                d.log("source_missing_and_profile_requires_source", BLOCK_SCORE);
            }
        }
    }

    // ── Video codec ──────────────────────────────────────────────────────────
    if let Some(codec) = normalize_codec(release.video_codec.as_deref()) {
        if !c.video_codec_blocklist.is_empty() && is_in_list(&codec, &c.video_codec_blocklist) {
            d.log("video_codec_in_profile_blocklist", BLOCK_SCORE);
        } else if !c.video_codec_allowlist.is_empty() {
            if let Some(idx) = c.video_codec_allowlist.iter().position(|c| c == &codec) {
                let bonus = (80_i32 - idx as i32 * 20).max(0);
                d.log(&format!("video_codec_preferred_{idx}"), bonus);
            } else {
                d.log("video_codec_not_in_profile_allowlist", BLOCK_SCORE);
            }
        } else {
            let (code, delta) = match codec.as_str() {
                "H.265" | "AV1" | "VP9" => ("video_codec_quality_high", weights.video_codec_high),
                "H.264" => ("video_codec_quality_mid", weights.video_codec_mid),
                _ => ("video_codec_quality_other", 0),
            };
            if delta != 0 {
                d.log(code, delta);
            }
        }
    }

    // ── Audio codecs ─────────────────────────────────────────────────────────
    let audio_codecs = normalized_audio_codecs(release);
    if !audio_codecs.is_empty() {
        let has_allowlist_match = !c.audio_codec_allowlist.is_empty()
            && audio_codecs
                .iter()
                .any(|codec| is_in_list(codec, &c.audio_codec_allowlist));
        let all_blocklisted = !c.audio_codec_blocklist.is_empty()
            && audio_codecs
                .iter()
                .all(|codec| is_in_list(codec, &c.audio_codec_blocklist));

        if all_blocklisted {
            d.log("audio_codec_in_profile_blocklist", BLOCK_SCORE);
        } else if has_allowlist_match {
            if let Some(best_idx) =
                c.audio_codec_allowlist
                    .iter()
                    .enumerate()
                    .find_map(|(idx, allow)| {
                        audio_codecs
                            .iter()
                            .any(|codec| codec == allow)
                            .then_some(idx)
                    })
            {
                let bonus = (60_i32 - best_idx as i32 * 15).max(0);
                d.log(&format!("audio_codec_preferred_{best_idx}"), bonus);
            }
        } else {
            let best_delta = audio_codecs
                .iter()
                .map(|codec| audio_weight_for_codec(weights, codec, release.is_atmos))
                .max()
                .unwrap_or(0);
            if best_delta > 0 {
                let code = if best_delta >= 60 {
                    "audio_codec_lossless"
                } else if best_delta >= 40 {
                    "audio_codec_high"
                } else {
                    "audio_codec_standard"
                };
                d.log(code, best_delta);
            }
        }
    }

    // ── Audio channels ────────────────────────────────────────────────────────
    if let Some(ref channels) = release.audio_channels {
        let delta = match channels.as_str() {
            "7.1" => weights.channels_71,
            "5.1" | "6.1" => weights.channels_51,
            "2.0" | "2.1" => weights.channels_20,
            "1.0" => weights.channels_10,
            _ => 0,
        };
        if delta != 0 {
            d.log("audio_channels", delta);
        }
    }

    // ── Dolby Vision ─────────────────────────────────────────────────────────
    if release.is_dolby_vision {
        if c.dolby_vision_allowed {
            d.log("dolby_vision_bonus", weights.dolby_vision);
        } else {
            d.log("dolby_vision_not_allowed", BLOCK_SCORE);
        }
    }

    // ── HDR ──────────────────────────────────────────────────────────────────
    if release.detected_hdr {
        if c.detected_hdr_allowed {
            d.log("hdr_bonus", weights.hdr10);
        } else {
            d.log("hdr_not_allowed", BLOCK_SCORE);
        }
    }

    // ── BD disk ──────────────────────────────────────────────────────────────
    if release.is_bd_disk && !c.allow_bd_disk {
        d.log("bd_disk_not_allowed", BLOCK_SCORE);
    }

    // ── Remux preference ─────────────────────────────────────────────────────
    if c.prefer_remux {
        let (code, delta) = if release.is_remux {
            ("prefer_remux_match", weights.remux_bonus)
        } else {
            ("prefer_remux_missing", weights.remux_missing_penalty)
        };
        if delta != 0 {
            d.log(code, delta);
        }
    }

    // ── Atmos preference ─────────────────────────────────────────────────────
    if c.atmos_preferred {
        if release.is_atmos {
            d.log("atmos_preferred_match", weights.atmos_bonus);
        } else {
            d.log("atmos_preferred_missing", weights.atmos_missing_penalty);
        }
    }

    if !c.required_audio_languages.is_empty() {
        let release_langs: Vec<String> = release
            .languages_audio
            .iter()
            .map(|l| l.trim().to_ascii_uppercase())
            .collect();
        let all_present = c
            .required_audio_languages
            .iter()
            .all(|req| release_langs.iter().any(|rl| rl == req));
        if !all_present {
            d.log("required_audio_language_missing", BLOCK_SCORE);
        } else {
            d.log("required_audio_languages_match", 80);
        }
    }

    // ── Feature bonuses (always logged) ──────────────────────────────────────
    if release.is_proper_upload {
        d.log("proper_upload", weights.proper_bonus);
    }
    if release.is_repack {
        d.log("repack_upload", weights.repack_bonus);
    }

    // ── AI Enhanced / Upscaled penalty ──────────────────────────────────────
    if release.is_ai_enhanced {
        d.log("ai_enhanced_upscaled", weights.upscaled_penalty);
    }

    // ── Hardcoded subtitles penalty ──────────────────────────────────────────
    if release.is_hardcoded_subs {
        d.log("hardcoded_subs", weights.hardcoded_subs_penalty);
    }

    // ── Edition bonuses ──────────────────────────────────────────────────────
    if let Some(ref edition) = release.edition {
        let delta = match edition.as_str() {
            "IMAX" | "IMAX Enhanced" => weights.edition_imax,
            "Extended" | "Unrated" => weights.edition_extended,
            "Hybrid" => weights.edition_hybrid,
            "Criterion" => weights.edition_criterion,
            "Remaster" => weights.edition_remaster,
            "Director's Cut" => weights.edition_extended, // same tier as extended
            _ => 0,
        };
        if delta != 0 {
            d.log("edition_bonus", delta);
        }
    }

    // ── Streaming service tier ───────────────────────────────────────────────
    if let Some(ref service) = release.streaming_service {
        let delta = match service.as_str() {
            "Netflix" | "Apple TV+" | "Amazon" | "Disney+" => weights.streaming_tier1,
            "HBO Max" | "Paramount+" | "Hulu" | "Peacock" => weights.streaming_tier2,
            "Crunchyroll" | "Funimation" | "HIDIVE" => weights.streaming_anime,
            _ => weights.streaming_tier3,
        };
        if delta != 0 {
            d.log("streaming_service", delta);
        }
    }

    // ── SDR at 4K penalty ────────────────────────────────────────────────────
    if let Some(ref quality) = release.quality
        && quality.to_ascii_uppercase().contains("2160")
        && !release.detected_hdr
        && weights.sdr_at_4k_penalty != 0
    {
        d.log("sdr_at_4k", weights.sdr_at_4k_penalty);
    }

    // ── Anime-specific ───────────────────────────────────────────────────────
    if let Some(ver) = release.anime_version
        && ver >= 2
    {
        d.log("anime_version_bonus", weights.anime_v2_bonus);
    }

    // ── Release group reputation ─────────────────────────────────────────────
    {
        let (code, delta) = apply_release_group_scoring(
            weights,
            release.release_group.as_deref(),
            release.source.as_deref(),
            release.is_remux,
        );
        if delta != 0 {
            d.log(code, delta);
        }
    }

    if release.parse_confidence < 0.4 {
        d.log("low_parse_confidence", -75);
    }

    // ── Min score to grab gate ──────────────────────────────────────────────
    // Applied last so it considers all scoring factors above. Only blocks
    // releases that are otherwise allowed — already-blocked releases don't
    // need a second block entry.
    if let Some(min_score) = c.min_score_to_grab
        && d.allowed
        && d.release_score < min_score
    {
        d.log("score_below_minimum", BLOCK_SCORE);
    }

    d
}

/// Apply an age-based scoring adjustment to a release decision.
///
/// Fresh NZBs get a bonus while old ones get a penalty. The curve is graduated
/// to match typical usenet retention (1000+ days):
///   0–14 days    → +50  (fresh)
///   15–90 days   → +25  (recent)
///   91–365 days  →   0  (neutral)
///   366–730 days → −25  (aging)
///   731–1500 days → −50  (old)
///   1500+ days   → −100 (very old)
///
/// `published_at` is the raw string from the indexer (typically RFC 2822 from RSS).
/// If parsing fails or the value is `None`, no scoring entry is logged.
/// Parse a published_at date string in RFC2822, RFC3339, or ISO8601 format.
pub fn parse_published_at(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| DateTime::parse_from_rfc3339(raw).map(|dt| dt.with_timezone(&Utc)))
        .or_else(|_| raw.parse::<DateTime<Utc>>())
        .ok()
}

pub fn apply_age_scoring(decision: &mut QualityProfileDecision, published_at: Option<&str>) {
    let Some(raw) = published_at else {
        return;
    };

    let Some(published) = parse_published_at(raw) else {
        return;
    };

    let age_days = (Utc::now() - published).num_days();

    let (code, delta) = match age_days {
        d if d < 0 => return, // future date — skip
        0..=14 => ("age_fresh", 50),
        15..=90 => ("age_recent", 25),
        91..=365 => return, // neutral — no entry
        366..=730 => ("age_aging", -25),
        731..=1500 => ("age_old", -50),
        _ => ("age_very_old", -100),
    };

    decision.log(code, delta);
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
fn expected_bitrate_mbps(quality: Option<&str>, media_category: MediaSizeCategory) -> f64 {
    match media_category {
        MediaSizeCategory::Movie => match quality {
            Some("2160P") => 57.0,
            Some("1080P") => 9.1,
            Some("720P") => 3.4,
            Some("480P") => 1.4,
            _ => 6.8,
        },
        MediaSizeCategory::Series => match quality {
            Some("2160P") => 22.0,
            Some("1080P") => 8.5,
            Some("720P") => 3.3,
            Some("480P") => 1.4,
            _ => 5.5,
        },
        MediaSizeCategory::Anime => match quality {
            Some("2160P") => 28.0,
            Some("1080P") => 8.5,
            Some("720P") => 3.4,
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
fn codec_efficiency_factor(codec: Option<&str>) -> f64 {
    match codec {
        Some("AV1") => 0.50,
        Some("H.265") => 0.75,
        Some("VP9") => 0.75,
        Some("H.264") => 1.10,
        _ => 1.0,
    }
}

/// Source-type multiplier applied to the expected bitrate.  Bluray encodes are
/// larger than WEB-DL; remuxes larger still.
fn source_size_factor(
    source: Option<&str>,
    is_remux: bool,
    is_bd_disk: bool,
    is_anime: bool,
) -> f64 {
    let mut factor = 1.0;
    if matches!(source, Some("BLURAY")) {
        factor *= 1.35;
    }
    if is_remux && !is_anime {
        factor *= 1.45;
    }
    if is_bd_disk {
        factor *= 1.8;
    }
    if matches!(source, Some("WEB-DL") | Some("WEBRIP")) {
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

#[derive(Debug, Clone, Copy)]
struct SizeRatioThresholds {
    implausible: f64,
    excessive: f64,
    massive: f64,
    very_large: f64,
    large: f64,
    expected: f64,
    slightly_small: f64,
    small: f64,
    very_small: f64,
}

fn size_ratio_thresholds(media_category: MediaSizeCategory) -> SizeRatioThresholds {
    match media_category {
        MediaSizeCategory::Movie | MediaSizeCategory::Series => SizeRatioThresholds {
            implausible: 8.0,
            excessive: 4.0,
            massive: 2.4,
            very_large: 1.8,
            large: 1.35,
            expected: 1.0,
            slightly_small: 0.75,
            small: 0.55,
            very_small: 0.35,
        },
        MediaSizeCategory::Anime => SizeRatioThresholds {
            implausible: 6.0,
            excessive: 2.5,
            massive: 2.1,
            very_large: 1.6,
            large: 1.2,
            expected: 0.85,
            slightly_small: 0.65,
            small: 0.5,
            very_small: 0.3,
        },
    }
}

/// Apply category-aware size scoring using a bitrate-based model.
///
/// Expected file size is derived from `bitrate × runtime × codec_factor ×
/// source_factor`.  This makes scoring inherently runtime-aware: a 140-minute
/// season finale is expected to be ~3× the size of a 45-minute episode at the
/// same quality/codec/source, without needing a separate runtime multiplier.
///
/// When `runtime_minutes` is not available (TVDB metadata missing), category
/// defaults are used (120 min movies, 45 min series, 24 min anime).
pub fn apply_size_scoring_for_category(
    decision: &mut QualityProfileDecision,
    release: &ParsedReleaseMetadata,
    size_bytes: Option<i64>,
    category_hint: Option<&str>,
    runtime_minutes: Option<i32>,
    weights: &ScoringWeights,
) {
    let Some(raw_size_bytes) = size_bytes else {
        return;
    };
    if raw_size_bytes <= 0 {
        return;
    }

    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let size_gib = (raw_size_bytes as f64) / GIB;

    let quality = normalize_quality(release.quality.as_deref());
    let source = normalize_source(release.source.as_deref());
    let media_category = normalize_media_size_category(category_hint);
    let is_anime = media_category == MediaSizeCategory::Anime;

    let bitrate = expected_bitrate_mbps(quality.as_deref(), media_category);
    let codec_factor = codec_efficiency_factor(release.video_codec.as_deref());
    let source_factor = source_size_factor(
        source.as_deref(),
        release.is_remux,
        release.is_bd_disk,
        is_anime,
    );

    let runtime_min = runtime_minutes
        .filter(|&r| r > 0)
        .map(|r| r as f64)
        .unwrap_or_else(|| default_runtime_minutes(media_category));

    // bitrate (Mbps) × runtime (seconds) / 8 = megabytes → convert to GiB
    let expected_gib = bitrate * codec_factor * source_factor * (runtime_min * 60.0) / 8.0 / 1024.0;

    let ratio = size_gib / expected_gib.max(0.5);
    let thresholds = size_ratio_thresholds(media_category);

    let (code, delta) = match ratio {
        r if r >= thresholds.implausible => ("size_implausible_for_quality", BLOCK_SCORE),
        r if r >= thresholds.excessive => ("size_excessive_for_quality", weights.size_excessive),
        r if r >= thresholds.massive => ("size_massive_for_quality", weights.size_massive),
        r if r >= thresholds.very_large => ("size_very_large_for_quality", weights.size_very_large),
        r if r >= thresholds.large => ("size_large_for_quality", weights.size_large),
        r if r >= thresholds.expected => ("size_expected_for_quality", weights.size_expected),
        r if r >= thresholds.slightly_small => (
            "size_slightly_small_for_quality",
            weights.size_slightly_small,
        ),
        r if r >= thresholds.small => ("size_small_for_quality", weights.size_small),
        r if r >= thresholds.very_small => ("size_very_small_for_quality", weights.size_very_small),
        _ => ("size_tiny_for_quality", weights.size_tiny),
    };

    decision.log(code, delta);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release_parser::parse_release_metadata;
    use crate::scoring_weights::balanced_weights;

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

        let w = balanced_weights();
        let release = parse_release_metadata("Some.Movie.1080p.WEB-DL.H.265.DDP2.0");
        let result = evaluate_against_profile(&profile, &release, false, &w);
        assert!(result.allowed);

        let release = parse_release_metadata("Some.Movie.WEB-DL.H.265.DDP2.0");
        let result = evaluate_against_profile(&profile, &release, false, &w);
        assert!(!result.allowed);
        assert!(
            result
                .block_codes
                .iter()
                .any(|code| code == "quality_missing_and_profile_disallows_unknown")
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

        let w = balanced_weights();
        let release = parse_release_metadata("Some.Movie.WEB-DL.H.265.DDP2.0");
        let result = evaluate_against_profile(&profile, &release, false, &w);
        assert!(result.allowed);
    }

    #[test]
    fn profile_prefers_atmos_candidates() {
        let profile = QualityProfile::parse(
            r#"{
                "id":"anime",
                "name":"Anime",
                "criteria": {
                    "atmos_preferred":true,
                    "prefer_remux":false,
                    "allow_upgrades":true
                }
            }"#,
        )
        .expect("profile must parse");

        let w = balanced_weights();
        let with_atmos =
            parse_release_metadata("Show.2021.1080p.WEB-DL.H.265.DDP.Atmos.5.1.AAC2.0");
        let no_atmos = parse_release_metadata("Show.2021.1080p.WEB-DL.H.265.DDP.5.1.AAC2.0");

        assert!(
            evaluate_against_profile(&profile, &with_atmos, false, &w).preference_score
                > evaluate_against_profile(&profile, &no_atmos, false, &w).preference_score
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

        let w = crate::scoring_weights::build_weights(
            &crate::scoring_weights::ScoringPersona::Audiophile,
            &crate::scoring_weights::ScoringOverrides::default(),
        );
        let with_remux = parse_release_metadata("Movie.2021.1080p.WEB-DL.H.265.Remux.DDP2.0");
        let without_remux = parse_release_metadata("Movie.2021.1080p.WEB-DL.H.265.DDP2.0");

        assert!(
            evaluate_against_profile(&profile, &with_remux, false, &w).allowed
                && evaluate_against_profile(&profile, &without_remux, false, &w).allowed
        );
        assert!(
            evaluate_against_profile(&profile, &with_remux, false, &w).preference_score
                > evaluate_against_profile(&profile, &without_remux, false, &w).preference_score
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

        let w = balanced_weights();
        let release = parse_release_metadata("Movie.2021.1080p.WEB-DL.H.264.DDP2.0");
        let result = evaluate_against_profile(&profile, &release, false, &w);
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

        let w = balanced_weights();
        let hdr_release = parse_release_metadata("Movie.2021.2160p.WEB-DL.HDR.HDR10.x265.DDP");
        let regular_release = parse_release_metadata("Movie.2021.2160p.WEB-DL.H.265.DDP2.0");

        let hdr_result = evaluate_against_profile(&profile, &hdr_release, false, &w);
        let regular_result = evaluate_against_profile(&profile, &regular_release, false, &w);

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

        let w = balanced_weights();
        let release = parse_release_metadata("Movie.2024.2160p.BluRay.DTS-HD.TrueHD.7.1.H.265");
        let result = evaluate_against_profile(&profile, &release, false, &w);
        assert!(result.allowed);
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

        let w = balanced_weights();
        let release = parse_release_metadata("Movie.2024.2160p.BluRay.DTS-HD.TrueHD.7.1.H.265");
        let result = evaluate_against_profile(&profile, &release, false, &w);
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

        let w = balanced_weights();
        let hdr_release = parse_release_metadata("Movie.2021.2160p.WEB-DL.HDR.HDR10.x265.DDP");
        assert!(evaluate_against_profile(&profile, &hdr_release, false, &w).allowed);
    }

    #[test]
    fn size_scoring_heavily_prefers_larger_release_for_same_metadata() {
        let profile = default_quality_profile_for_search();
        let w = balanced_weights();
        let release = parse_release_metadata("Movie.2021.2160p.BluRay.Remux.H.265.DTSHD.Atmos");

        let mut small = evaluate_against_profile(&profile, &release, false, &w);
        apply_size_scoring_for_category(
            &mut small,
            &release,
            Some(7 * 1024 * 1024 * 1024),
            None,
            None,
            &w,
        );

        let mut large = evaluate_against_profile(&profile, &release, false, &w);
        apply_size_scoring_for_category(
            &mut large,
            &release,
            Some(45 * 1024 * 1024 * 1024),
            None,
            None,
            &w,
        );

        assert!(large.preference_score > small.preference_score);
        assert!(large.preference_score - small.preference_score >= 900);
    }

    #[test]
    fn tiny_uhd_can_rank_below_high_quality_1080() {
        let profile = default_quality_profile_for_search();
        let w = balanced_weights();

        let tiny_uhd = parse_release_metadata("Movie.2021.2160p.BluRay.Remux.H.265.DTSHD.Atmos");
        let mut tiny_uhd_decision = evaluate_against_profile(&profile, &tiny_uhd, false, &w);
        apply_size_scoring_for_category(
            &mut tiny_uhd_decision,
            &tiny_uhd,
            Some(5 * 1024 * 1024 * 1024),
            None,
            None,
            &w,
        );

        let strong_1080 = parse_release_metadata("Movie.2021.1080p.BluRay.H.264.DTS");
        let mut strong_1080_decision = evaluate_against_profile(&profile, &strong_1080, false, &w);
        apply_size_scoring_for_category(
            &mut strong_1080_decision,
            &strong_1080,
            Some(18 * 1024 * 1024 * 1024),
            None,
            None,
            &w,
        );

        assert!(strong_1080_decision.preference_score > tiny_uhd_decision.preference_score);
    }

    #[test]
    fn plausible_uhd_still_outscores_1080_due_to_tier_priority() {
        let profile = default_quality_profile_for_search();
        let w = balanced_weights();

        let plausible_uhd = parse_release_metadata("Movie.2021.2160p.BluRay.Remux.H.265.DTSHD");
        let mut plausible_uhd_decision =
            evaluate_against_profile(&profile, &plausible_uhd, false, &w);
        apply_size_scoring_for_category(
            &mut plausible_uhd_decision,
            &plausible_uhd,
            Some(35 * 1024 * 1024 * 1024),
            None,
            None,
            &w,
        );

        let strong_1080 = parse_release_metadata("Movie.2021.1080p.BluRay.H.264.DTS");
        let mut strong_1080_decision = evaluate_against_profile(&profile, &strong_1080, false, &w);
        apply_size_scoring_for_category(
            &mut strong_1080_decision,
            &strong_1080,
            Some(18 * 1024 * 1024 * 1024),
            None,
            None,
            &w,
        );

        assert!(plausible_uhd_decision.preference_score > strong_1080_decision.preference_score);
    }
}

fn resolve_archival_quality(
    archival_quality: Option<String>,
    quality_tiers: &[String],
) -> Option<String> {
    match archival_quality.and_then(|value| normalize_quality(Some(&value))) {
        Some(normalized) if !normalized.is_empty() => Some(normalized),
        _ => quality_tiers
            .first()
            .and_then(|value| normalize_quality(Some(value)))
            .or_else(|| Some("1080P".to_string())),
    }
}

#[cfg(test)]
#[path = "quality_profile_tests.rs"]
mod quality_profile_tests;
