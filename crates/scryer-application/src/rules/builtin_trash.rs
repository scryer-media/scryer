use serde::Deserialize;

use crate::{AppError, AppResult, RulePackRegistryEntry, RulePackTemplate, VerifiedRulePack};

#[cfg(test)]
use crate::AppUseCase;
#[cfg(test)]
use chrono::Utc;
#[cfg(test)]
use scryer_domain::{MediaFacet, RuleSet};
#[cfg(test)]
use std::sync::LazyLock;

pub(crate) const BUILTIN_TRASH_PACK_ID: &str = "trash-guides-scoring-pack";
const BUILTIN_TRASH_SHA256: &str =
    "75c0c8f49651d75c496d44b551c36ab752a74f5ba82bb4092c3d40f7b1d29e51";

#[derive(Deserialize)]
struct BuiltinPackManifest {
    schema_version: u32,
    id: String,
    name: String,
    description: String,
    author: String,
    version: String,
    min_scryer_version: String,
    #[serde(default = "default_customizable")]
    customizable: bool,
    rules: Vec<RulePackTemplate>,
}

const fn default_customizable() -> bool {
    true
}

pub(crate) fn verified_pack() -> AppResult<VerifiedRulePack> {
    let manifest: BuiltinPackManifest = serde_json::from_str(include_str!("builtin_trash.json"))
        .map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::rules::BUNDLED_TRASH_PACK_INVALID,
                format!("invalid bundled TRaSH pack: {error}"),
            )
        })?;
    if manifest.schema_version != 1 || manifest.id != BUILTIN_TRASH_PACK_ID {
        return Err(AppError::validation_refused(
            crate::error_reasons::rules::BUNDLED_TRASH_PACK_UNSUPPORTED_IDENTITY_SCHEMA,
            "bundled TRaSH pack has an unsupported identity or schema".to_string(),
        ));
    }
    if manifest.rules.is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::rules::BUNDLED_TRASH_PACK_CONTAINS_NO_TEMPLATES,
            "bundled TRaSH pack contains no templates".to_string(),
        ));
    }
    Ok(VerifiedRulePack {
        revision: format!("builtin:{}", manifest.version),
        registry: RulePackRegistryEntry {
            id: manifest.id,
            name: manifest.name,
            description: manifest.description,
            author: manifest.author,
            version: manifest.version,
            digest: format!("sha256:{BUILTIN_TRASH_SHA256}"),
            source_url: "builtin://trash-guides-scoring-pack".to_string(),
            min_scryer_version: Some(manifest.min_scryer_version),
            customizable: manifest.customizable,
        },
        templates: manifest.rules,
    })
}

pub(crate) fn default_template_ids(pack: &VerifiedRulePack) -> Vec<String> {
    pack.templates
        .iter()
        .filter(|template| template.default_enabled)
        .map(|template| template.id.clone())
        .collect()
}

/// Shipped template sources that later bundled revisions supersede, keyed by
/// template id. Each historical source is an immutable matcher.
const SUPERSEDED_TEMPLATE_SOURCES: &[(&str, &str)] = &[
    (
        "trash-guides-size",
        include_str!("legacy_size_scoring.rego"),
    ),
    (
        "trash-guides-source-video",
        include_str!("legacy_source_video_scoring.rego"),
    ),
];

/// Narrow corrections for existing installations, independent of pack updates.
/// Match both tracked membership and the original source, preserving settings,
/// copies and customizations.
pub(super) fn superseded_template_updates(
    pack: &VerifiedRulePack,
    installation: &scryer_domain::RulePackInstallation,
    rules: &[scryer_domain::RuleSet],
) -> Vec<scryer_domain::RuleSet> {
    SUPERSEDED_TEMPLATE_SOURCES
        .iter()
        .filter_map(|(template_id, superseded)| {
            superseded_template_update(pack, installation, rules, template_id, superseded)
        })
        .collect()
}

fn superseded_template_update(
    pack: &VerifiedRulePack,
    installation: &scryer_domain::RulePackInstallation,
    rules: &[scryer_domain::RuleSet],
    template_id: &str,
    superseded: &str,
) -> Option<scryer_domain::RuleSet> {
    let member = installation
        .members
        .iter()
        .find(|member| member.template_id == template_id && !member.removed)?;
    let rule = rules.iter().find(|rule| rule.id == member.rule_set_id)?;
    if rule.rego_source != scryer_rules::rewrite_package_declaration(superseded, &rule.id) {
        return None;
    }
    let template = pack
        .templates
        .iter()
        .find(|template| template.id == member.template_id)?;
    let mut updated = rule.clone();
    updated.rego_source =
        scryer_rules::rewrite_package_declaration(&template.rego_source, &rule.id);
    if updated.rego_source == rule.rego_source {
        return None;
    }
    updated.updated_at = chrono::Utc::now();
    Some(updated)
}

/// Fresh-install baseline policies materialized from the bundled manifest.
/// Tests use these rather than reproducing pack scores in Rust fixtures.
#[cfg(test)]
fn baseline_rule_set_id(template_id: &str) -> String {
    template_id.replace('-', "_")
}

#[cfg(test)]
pub(crate) fn baseline_rule_sets() -> Vec<RuleSet> {
    let pack = verified_pack().expect("bundled TRaSH pack parses");
    pack.templates
        .iter()
        .filter(|template| template.default_enabled)
        .map(|template| {
            let id = baseline_rule_set_id(&template.id);
            RuleSet {
                id: id.clone(),
                name: template.title.clone(),
                description: template.description.clone(),
                rego_source: scryer_rules::rewrite_package_declaration(&template.rego_source, &id),
                enabled: true,
                priority: 0,
                evaluation_phase: template.evaluation_phase,
                exclusive_group: template.exclusive_group.clone(),
                disabled_reason_code: None,
                disabled_reason: None,
                applied_facets: template
                    .applied_facets
                    .iter()
                    .map(|facet| MediaFacet::parse(facet).expect("bundled pack facet is valid"))
                    .collect(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                is_managed: false,
                managed_key: None,
                managed_tag_filter: None,
            }
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn baseline_policies() -> Vec<scryer_rules::UserPolicy> {
    baseline_rule_sets()
        .into_iter()
        .map(|rule| scryer_rules::UserPolicy {
            id: rule.id,
            name: rule.name,
            rego_source: rule.rego_source,
            origin: scryer_rules::PolicyOrigin::System,
            applied_facets: rule
                .applied_facets
                .iter()
                .map(|facet| facet.as_str().to_string())
                .collect(),
        })
        .collect()
}

/// A shared, immutable evaluator for the baseline rules enabled by a fresh
/// bundled-pack installation.
#[cfg(test)]
pub(crate) fn baseline_engine() -> &'static scryer_rules::UserRulesEngine {
    static ENGINE: LazyLock<scryer_rules::UserRulesEngine> = LazyLock::new(|| {
        AppUseCase::build_user_rules_engine(baseline_rule_sets(), Vec::new())
            .expect("bundled TRaSH baseline builds")
    });
    &ENGINE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "runtime-plugin-trust")]
    use sha2::{Digest, Sha256};

    #[test]
    fn bundled_pack_enables_core_templates_but_not_locale_templates() {
        let pack = verified_pack().expect("bundled pack parses");
        let enabled = default_template_ids(&pack);
        assert!(enabled.contains(&"trash-guides-source-video".to_string()));
        assert!(enabled.contains(&"trash-guides-audio".to_string()));
        assert!(!enabled.contains(&"trash-guides-french-vf".to_string()));
        assert!(!enabled.contains(&"trash-guides-german".to_string()));
    }

    #[test]
    fn bundled_baseline_engine_builds() {
        let _ = baseline_engine();
    }

    #[test]
    fn size_policy_correction_preserves_settings_and_custom_rules() {
        let pack = verified_pack().unwrap();
        let mut original = baseline_rule_sets()
            .into_iter()
            .find(|r| r.id == "trash_guides_size")
            .unwrap();
        original.rego_source = scryer_rules::rewrite_package_declaration(
            include_str!("legacy_size_scoring.rego"),
            &original.id,
        );
        original.enabled = false;
        original.priority = 37;
        original.applied_facets = vec![MediaFacet::Series];
        let mut installation = scryer_domain::RulePackInstallation {
            pack_id: BUILTIN_TRASH_PACK_ID.into(),
            name: pack.registry.name.clone(),
            version: pack.registry.version.clone(),
            digest: pack.registry.digest.clone(),
            customizable: true,
            auto_update: false,
            revision: 1,
            last_updated: Utc::now(),
            last_error: None,
            members: vec![scryer_domain::RulePackMember {
                template_id: "trash-guides-size".into(),
                rule_set_id: original.id.clone(),
                removed: false,
            }],
        };
        let changes = superseded_template_updates(&pack, &installation, &[original.clone()]);
        assert_eq!(changes.len(), 1);
        let changed = &changes[0];
        assert!(!changed.enabled);
        assert_eq!(changed.priority, 37);
        assert_eq!(changed.applied_facets, original.applied_facets);
        assert_eq!(changed.created_at, original.created_at);
        assert_ne!(changed.rego_source, original.rego_source);
        assert!(superseded_template_updates(&pack, &installation, &changes).is_empty());

        let mut custom = original.clone();
        custom.rego_source.push_str("\n# Local customization\n");
        assert!(superseded_template_updates(&pack, &installation, &[custom]).is_empty());
        installation.members[0].removed = true;
        assert!(superseded_template_updates(&pack, &installation, &[original]).is_empty());
    }

    #[test]
    fn source_video_correction_replaces_only_the_shipped_source() {
        let pack = verified_pack().unwrap();
        let bundled = baseline_rule_sets()
            .into_iter()
            .find(|r| r.id == "trash_guides_source_video")
            .unwrap();
        let mut original = bundled.clone();
        original.rego_source = scryer_rules::rewrite_package_declaration(
            include_str!("legacy_source_video_scoring.rego"),
            &original.id,
        );
        assert_ne!(original.rego_source, bundled.rego_source);
        original.priority = 12;
        let installation = scryer_domain::RulePackInstallation {
            pack_id: BUILTIN_TRASH_PACK_ID.into(),
            name: pack.registry.name.clone(),
            version: pack.registry.version.clone(),
            digest: pack.registry.digest.clone(),
            customizable: true,
            auto_update: false,
            revision: 1,
            last_updated: Utc::now(),
            last_error: None,
            members: vec![scryer_domain::RulePackMember {
                template_id: "trash-guides-source-video".into(),
                rule_set_id: original.id.clone(),
                removed: false,
            }],
        };

        let changes = superseded_template_updates(&pack, &installation, &[original.clone()]);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].rego_source, bundled.rego_source);
        assert!(changes[0].rego_source.contains("video_codec_hevc_below_4k"));
        assert_eq!(changes[0].priority, 12);
        assert!(superseded_template_updates(&pack, &installation, &changes).is_empty());

        let mut edited = original.clone();
        edited.rego_source = edited.rego_source.replace(
            "native_sv_high_codec_weight := 60",
            "native_sv_high_codec_weight := 75",
        );
        assert_ne!(edited.rego_source, original.rego_source);
        assert!(superseded_template_updates(&pack, &installation, &[edited]).is_empty());
    }

    #[cfg(feature = "runtime-plugin-trust")]
    #[test]
    fn bundled_pack_checksum_matches_embedded_artifact() {
        let digest = Sha256::digest(include_bytes!("builtin_trash.json"));
        let checksum = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(checksum, BUILTIN_TRASH_SHA256);
    }
}
