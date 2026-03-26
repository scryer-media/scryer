use async_graphql::{Error, Result as GqlResult};
use scryer_domain::{Entitlement, ExternalId, NewTitle};

use crate::types::{AddTitleInput, DownloadSourceKindValue, TitleOptionsInput};

fn push_structured_tag(tags: &mut Vec<String>, prefix: &str, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let normalized = value.trim();
    if normalized.is_empty() {
        return;
    }
    tags.push(format!("{prefix}{normalized}"));
}

fn set_structured_tag(tags: &mut Vec<String>, prefix: &str, value: Option<String>) {
    tags.retain(|tag| !tag.starts_with(prefix));
    push_structured_tag(tags, prefix, value);
}

fn normalize_title_tag(tag: String) -> Option<String> {
    let trimmed = tag.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }

    Some(if trimmed.starts_with("scryer:") {
        trimmed
    } else {
        trimmed.to_lowercase()
    })
}

pub(crate) fn normalize_title_tags(tags: Vec<String>) -> Vec<String> {
    tags.into_iter().filter_map(normalize_title_tag).collect()
}

pub(crate) fn apply_title_options(tags: &mut Vec<String>, options: TitleOptionsInput) {
    set_structured_tag(
        tags,
        "scryer:quality-profile:",
        options.quality_profile_id.map(|value| value.trim().to_string()),
    );
    set_structured_tag(
        tags,
        "scryer:root-folder:",
        options.root_folder_path.map(|value| value.trim().to_string()),
    );
    set_structured_tag(
        tags,
        "scryer:monitor-type:",
        options
            .monitor_type
            .map(|value| value.as_tag_value().to_string()),
    );
    set_structured_tag(
        tags,
        "scryer:filler-policy:",
        options.filler_policy.map(|value| value.trim().to_string()),
    );
    set_structured_tag(
        tags,
        "scryer:recap-policy:",
        options.recap_policy.map(|value| value.trim().to_string()),
    );

    if let Some(use_season_folders) = options.use_season_folders {
        set_structured_tag(
            tags,
            "scryer:season-folder:",
            Some(
                if use_season_folders {
                    "enabled"
                } else {
                    "disabled"
                }
                .to_string(),
            ),
        );
    }

    if let Some(monitor_specials) = options.monitor_specials {
        set_structured_tag(
            tags,
            "scryer:monitor-specials:",
            Some(if monitor_specials { "true" } else { "false" }.to_string()),
        );
    }

    if let Some(inter_season_movies) = options.inter_season_movies {
        set_structured_tag(
            tags,
            "scryer:inter-season-movies:",
            Some(if inter_season_movies { "true" } else { "false" }.to_string()),
        );
    }
}

pub(crate) fn merge_title_option_tags(
    mut tags: Vec<String>,
    options: TitleOptionsInput,
) -> Vec<String> {
    apply_title_options(&mut tags, options);
    tags
}

pub(crate) fn map_add_input(input: AddTitleInput) -> GqlResult<NewTitle> {
    let AddTitleInput {
        name,
        facet,
        monitored,
        mut tags,
        options,
        external_ids,
        source_hint: _,
        source_kind: _,
        source_title: _,
        min_availability,
        poster_url,
        year,
        overview,
        sort_title,
        slug,
        runtime_minutes,
        language,
        content_status,
    } = input;

    let parsed_facet = facet.into_domain();
    tags = normalize_title_tags(tags);
    if let Some(options) = options {
        apply_title_options(&mut tags, options);
    }

    Ok(NewTitle {
        name,
        facet: parsed_facet,
        monitored,
        tags,
        external_ids: external_ids
            .unwrap_or_default()
            .into_iter()
            .map(|item| ExternalId {
                source: item.source,
                value: item.value,
            })
            .collect(),
        min_availability,
        poster_url,
        year,
        overview,
        sort_title,
        slug,
        runtime_minutes,
        language,
        content_status,
    })
}

pub(crate) fn parse_download_source_kind(
    raw: Option<DownloadSourceKindValue>,
) -> Option<scryer_application::DownloadSourceKind> {
    raw.map(DownloadSourceKindValue::into_application)
}

pub(crate) fn parse_entitlements(raw_entitlements: &[String]) -> GqlResult<Vec<Entitlement>> {
    let mut seen = std::collections::HashSet::new();
    let mut entitlements = Vec::with_capacity(raw_entitlements.len());

    for raw in raw_entitlements {
        let normalized = raw.trim().to_lowercase().replace('-', "_");
        let entitlement = match normalized.as_str() {
            "viewcatalog" | "view_catalog" => Entitlement::ViewCatalog,
            "monitortitle" | "monitor_title" => Entitlement::MonitorTitle,
            "managetitle" | "manage_title" => Entitlement::ManageTitle,
            "triggeractions" | "trigger_actions" => Entitlement::TriggerActions,
            "manageconfig" | "manage_config" => Entitlement::ManageConfig,
            "viewhistory" | "view_history" => Entitlement::ViewHistory,
            other => {
                return Err(Error::new(format!("unknown entitlement: {other}")));
            }
        };

        if seen.insert(entitlement.clone()) {
            entitlements.push(entitlement);
        }
    }

    Ok(entitlements)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::types::MediaFacetValue;
    use scryer_domain::{Entitlement, MediaFacet};

    #[test]
    fn parse_entitlements_accepts_known_values() {
        let parsed = parse_entitlements(&[
            "View_Catalog".into(),
            "manage_title".into(),
            "VIEWHISTORY".into(),
        ])
        .expect("entitlements should parse");

        assert_eq!(parsed.len(), 3);
        assert!(parsed.contains(&Entitlement::ViewCatalog));
        assert!(parsed.contains(&Entitlement::ManageTitle));
        assert!(parsed.contains(&Entitlement::ViewHistory));
    }

    #[test]
    fn parse_entitlements_rejects_unknown_value() {
        let err = parse_entitlements(&["not_a_permission".into()])
            .expect_err("unknown entitlements should fail");
        assert!(format!("{:?}", err).contains("unknown entitlement"));
    }

    #[test]
    fn media_facet_value_maps_tv_to_series_domain() {
        assert_eq!(MediaFacetValue::Tv.into_domain(), MediaFacet::Series);
    }
}
