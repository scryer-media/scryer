use async_graphql::{Error, Result as GqlResult};
use scryer_application::DownloadSourceKind;
use scryer_domain::{Entitlement, ExternalId, MediaFacet, NewTitle};

use crate::types::AddTitleInput;

pub(crate) fn parse_facet(raw: Option<String>) -> Option<MediaFacet> {
    raw.and_then(|value| MediaFacet::parse(&value))
}

pub(crate) fn map_add_input(input: AddTitleInput) -> NewTitle {
    NewTitle {
        name: input.name,
        facet: parse_facet(Some(input.facet)).unwrap_or_default(),
        monitored: input.monitored,
        tags: input.tags,
        external_ids: input
            .external_ids
            .unwrap_or_default()
            .into_iter()
            .map(|item| ExternalId {
                source: item.source,
                value: item.value,
            })
            .collect(),
        min_availability: input.min_availability,
        poster_url: input.poster_url,
        year: input.year,
        overview: input.overview,
        sort_title: input.sort_title,
        slug: input.slug,
        runtime_minutes: input.runtime_minutes,
        language: input.language,
        content_status: input.content_status,
    }
}

pub(crate) fn parse_download_source_kind(raw: Option<String>) -> Option<DownloadSourceKind> {
    raw.and_then(|value| DownloadSourceKind::parse(&value))
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

    use scryer_domain::Entitlement;

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
    fn parse_facet_known_values_are_mapped() {
        assert_eq!(parse_facet(Some("movie".into())), Some(MediaFacet::Movie));
        assert_eq!(parse_facet(Some("TV".into())), Some(MediaFacet::Series));
        assert_eq!(parse_facet(Some("anime".into())), Some(MediaFacet::Anime));
    }

    #[test]
    fn parse_facet_unknown_value_is_none() {
        assert_eq!(parse_facet(Some("wrong".into())), None);
    }
}
