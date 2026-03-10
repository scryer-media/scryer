use scryer_application::{QualityProfile, QualityProfileCriteria};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Default)]
pub(crate) struct RawQualityProfileCriteria {
    #[serde(default)]
    pub quality_tiers: Vec<String>,
    #[serde(default)]
    pub archival_quality: Option<String>,
    #[serde(default)]
    pub allow_unknown_quality: bool,
    #[serde(default)]
    pub source_allowlist: Vec<String>,
    #[serde(default)]
    pub source_blocklist: Vec<String>,
    #[serde(default)]
    pub video_codec_allowlist: Vec<String>,
    #[serde(default)]
    pub video_codec_blocklist: Vec<String>,
    #[serde(default)]
    pub audio_codec_allowlist: Vec<String>,
    #[serde(default)]
    pub audio_codec_blocklist: Vec<String>,
    #[serde(default)]
    pub atmos_preferred: bool,
    #[serde(default)]
    pub dolby_vision_allowed: bool,
    #[serde(default)]
    pub detected_hdr_allowed: bool,
    #[serde(default)]
    pub prefer_remux: bool,
    #[serde(default)]
    pub allow_bd_disk: bool,
    #[serde(default)]
    pub allow_upgrades: bool,
    #[serde(default)]
    pub prefer_dual_audio: bool,
    #[serde(default)]
    pub required_audio_languages: Vec<String>,
    #[serde(default)]
    pub scoring_persona: scryer_application::ScoringPersona,
    #[serde(default)]
    pub scoring_overrides: scryer_application::ScoringOverrides,
    #[serde(default)]
    pub cutoff_tier: Option<String>,
    #[serde(default)]
    pub min_score_to_grab: Option<i32>,
    #[serde(default)]
    pub facet_persona_overrides: HashMap<String, scryer_application::ScoringPersona>,
}

#[derive(Deserialize)]
struct RawQualityProfile {
    id: String,
    name: String,
    #[serde(default)]
    criteria: RawQualityProfileCriteria,
}

pub(crate) fn parse_profile_catalog_from_json(
    raw_json: &str,
) -> Result<Vec<QualityProfile>, serde_json::Error> {
    let catalog = serde_json::from_str::<Vec<RawQualityProfile>>(raw_json)?;
    Ok(catalog
        .into_iter()
        .map(|raw| {
            let criteria = raw.criteria;
            QualityProfile {
                id: raw.id,
                name: raw.name,
                criteria: QualityProfileCriteria {
                    quality_tiers: criteria.quality_tiers,
                    archival_quality: criteria.archival_quality,
                    allow_unknown_quality: criteria.allow_unknown_quality,
                    source_allowlist: criteria.source_allowlist,
                    source_blocklist: criteria.source_blocklist,
                    video_codec_allowlist: criteria.video_codec_allowlist,
                    video_codec_blocklist: criteria.video_codec_blocklist,
                    audio_codec_allowlist: criteria.audio_codec_allowlist,
                    audio_codec_blocklist: criteria.audio_codec_blocklist,
                    atmos_preferred: criteria.atmos_preferred,
                    dolby_vision_allowed: criteria.dolby_vision_allowed,
                    detected_hdr_allowed: criteria.detected_hdr_allowed,
                    prefer_remux: criteria.prefer_remux,
                    allow_bd_disk: criteria.allow_bd_disk,
                    allow_upgrades: criteria.allow_upgrades,
                    prefer_dual_audio: criteria.prefer_dual_audio,
                    required_audio_languages: criteria.required_audio_languages,
                    scoring_persona: criteria.scoring_persona,
                    scoring_overrides: criteria.scoring_overrides,
                    cutoff_tier: criteria.cutoff_tier,
                    min_score_to_grab: criteria.min_score_to_grab,
                    facet_persona_overrides: criteria.facet_persona_overrides,
                },
            }
        })
        .collect())
}

pub(crate) fn merge_quality_profiles(
    existing_profiles: Vec<QualityProfile>,
    incoming_profiles: Vec<QualityProfile>,
) -> Vec<QualityProfile> {
    let mut profile_updates_by_id = HashMap::new();
    let mut update_order = Vec::new();
    for profile in incoming_profiles {
        let normalized_id = profile.id.trim().to_string();
        if normalized_id.is_empty() {
            continue;
        }

        let normalized_profile = QualityProfile {
            id: normalized_id.clone(),
            ..profile
        };

        if !profile_updates_by_id.contains_key(&normalized_id) {
            update_order.push(normalized_id.clone());
        }
        profile_updates_by_id.insert(normalized_id, normalized_profile);
    }

    let mut merged = Vec::with_capacity(existing_profiles.len());
    let mut updated_profile_ids = HashSet::new();
    for profile in existing_profiles {
        let normalized_id = profile.id.trim().to_string();
        if normalized_id.is_empty() {
            continue;
        }

        if let Some(updated_profile) = profile_updates_by_id.remove(&normalized_id) {
            updated_profile_ids.insert(normalized_id);
            merged.push(updated_profile);
            continue;
        }

        merged.push(profile);
    }

    for update_id in update_order {
        if updated_profile_ids.contains(&update_id) {
            continue;
        }
        if let Some(updated_profile) = profile_updates_by_id.remove(&update_id) {
            merged.push(updated_profile);
        }
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    use scryer_application::QualityProfileCriteria;

    fn test_quality_profile(id: &str, name: &str) -> QualityProfile {
        QualityProfile {
            id: id.to_string(),
            name: name.to_string(),
            criteria: QualityProfileCriteria {
                quality_tiers: vec!["2160P".to_string(), "1080P".to_string()],
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
                prefer_remux: true,
                allow_bd_disk: false,
                allow_upgrades: true,
                prefer_dual_audio: false,
                required_audio_languages: vec![],
                scoring_persona: scryer_application::ScoringPersona::default(),
                scoring_overrides: scryer_application::ScoringOverrides::default(),
                cutoff_tier: None,
                min_score_to_grab: None,
                facet_persona_overrides: HashMap::new(),
            },
        }
    }

    #[test]
    fn merge_quality_profiles_updates_existing_and_appends_new() {
        let existing = vec![
            test_quality_profile("default", "4K"),
            test_quality_profile("anime", "Anime"),
        ];
        let incoming = vec![
            test_quality_profile("anime", "Anime V2"),
            test_quality_profile("new", "New"),
        ];

        let merged = merge_quality_profiles(existing, incoming);
        let names = merged
            .into_iter()
            .map(|profile| (profile.id, profile.name))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                ("default".to_string(), "4K".to_string()),
                ("anime".to_string(), "Anime V2".to_string()),
                ("new".to_string(), "New".to_string()),
            ]
        );
    }

    #[test]
    fn merge_quality_profiles_keeps_latest_update_for_duplicate_ids() {
        let existing = vec![test_quality_profile("default", "4K")];
        let incoming = vec![
            test_quality_profile("default", "Old Update"),
            test_quality_profile("default", "Latest Update"),
        ];

        let merged = merge_quality_profiles(existing, incoming);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "Latest Update");
    }
}
