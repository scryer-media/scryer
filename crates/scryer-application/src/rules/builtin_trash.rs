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
    "1fecd3fce839b290dc3fad3f85c459774ce737eafadb411e115632ea6552c034";

#[derive(Deserialize)]
struct BuiltinPackManifest {
    schema_version: u32,
    id: String,
    name: String,
    description: String,
    author: String,
    version: String,
    min_scryer_version: String,
    rules: Vec<RulePackTemplate>,
}

pub(crate) fn verified_pack() -> AppResult<VerifiedRulePack> {
    let manifest: BuiltinPackManifest = serde_json::from_str(include_str!("builtin_trash.json"))
        .map_err(|error| AppError::Validation(format!("invalid bundled TRaSH pack: {error}")))?;
    if manifest.schema_version != 1 || manifest.id != BUILTIN_TRASH_PACK_ID {
        return Err(AppError::Validation(
            "bundled TRaSH pack has an unsupported identity or schema".to_string(),
        ));
    }
    if manifest.rules.is_empty() {
        return Err(AppError::Validation(
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
