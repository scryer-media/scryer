use async_graphql::Result as GqlResult;
use scryer_domain::{ExternalId, NewTitle};

use crate::types::{AddTitleInput, DownloadSourceKindValue, IntoApplication};

pub(crate) struct ResolvedTitleOptionsInput {
    /// `None` preserves the stored value; `Some(None)` clears it; `Some(Some(_))` sets it.
    pub quality_profile_id: Option<Option<String>>,
    pub root_folder_id: Option<Option<String>>,
    pub monitor_type: Option<Option<String>>,
    pub use_season_folders: Option<Option<bool>>,
    pub metadata_language: Option<Option<String>>,
    pub monitor_specials: Option<Option<bool>>,
    pub inter_season_movies: Option<Option<bool>>,
    pub filler_policy: Option<Option<String>>,
    pub recap_policy: Option<Option<String>>,
    pub monitor_selection: Option<Option<scryer_domain::MonitorSelection>>,
}

impl ResolvedTitleOptionsInput {
    pub(crate) fn to_application_patch(&self) -> scryer_application::TitleOptionsPatch {
        scryer_application::TitleOptionsPatch {
            quality_profile_id: self.quality_profile_id.clone(),
            root_folder_id: self.root_folder_id.clone(),
            monitor_type: self.monitor_type.clone(),
            use_season_folders: self.use_season_folders,
            monitor_specials: self.monitor_specials,
            inter_season_movies: self.inter_season_movies,
            filler_policy: self.filler_policy.clone(),
            recap_policy: self.recap_policy.clone(),
            monitor_selection: self.monitor_selection.clone(),
        }
    }
}

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

pub(crate) fn apply_title_options(tags: &mut Vec<String>, options: ResolvedTitleOptionsInput) {
    if let Some(value) = options.quality_profile_id {
        set_structured_tag(tags, "scryer:quality-profile:", value);
    }
    if let Some(value) = options.monitor_type {
        set_structured_tag(tags, "scryer:monitor-type:", value);
    }
    if let Some(value) = options.filler_policy {
        set_structured_tag(tags, "scryer:filler-policy:", value);
    }
    if let Some(value) = options.recap_policy {
        set_structured_tag(tags, "scryer:recap-policy:", value);
    }

    if let Some(use_season_folders) = options.use_season_folders {
        set_structured_tag(
            tags,
            "scryer:season-folder:",
            use_season_folders.map(|use_season_folders| {
                if use_season_folders {
                    "enabled"
                } else {
                    "disabled"
                }
                .to_string()
            }),
        );
    }

    if let Some(monitor_specials) = options.monitor_specials {
        set_structured_tag(
            tags,
            "scryer:monitor-specials:",
            monitor_specials.map(|monitor_specials| {
                if monitor_specials { "true" } else { "false" }.to_string()
            }),
        );
    }

    if let Some(inter_season_movies) = options.inter_season_movies {
        set_structured_tag(
            tags,
            "scryer:inter-season-movies:",
            inter_season_movies.map(|inter_season_movies| {
                if inter_season_movies { "true" } else { "false" }.to_string()
            }),
        );
    }
}

pub(crate) fn merge_title_option_tags(
    mut tags: Vec<String>,
    options: ResolvedTitleOptionsInput,
) -> Vec<String> {
    apply_title_options(&mut tags, options);
    tags
}

pub(crate) fn map_add_input(
    input: AddTitleInput,
    resolved_options: Option<ResolvedTitleOptionsInput>,
) -> GqlResult<NewTitle> {
    let AddTitleInput {
        name,
        facet,
        library_id: _,
        monitored,
        mut tags,
        options: _,
        external_ids,
        smg_id,
        tvdb_id,
        tmdb_id,
        imdb_id,
        source_hint: _,
        source_kind: _,
        source_title: _,
        min_availability,
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
    let root_folder_id = resolved_options
        .as_ref()
        .and_then(|options| options.root_folder_id.clone().flatten());
    if let Some(options) = resolved_options {
        apply_title_options(&mut tags, options);
    }

    let mut external_ids = external_ids
        .unwrap_or_default()
        .into_iter()
        .map(|item| ExternalId {
            source: item.source,
            value: item.value,
        })
        .collect::<Vec<_>>();
    // Every facet retains every external id it was added with. Series and anime
    // titles hold their imdb/tmdb/smg ids alongside tvdb: those ids flow on into
    // indexer search subjects, RSS candidate indexes, notification payloads and
    // the `externalIds` readback, none of which are facet-aware, and dropping
    // them here left a series unable to be matched by id at all.
    for (source, value) in [
        ("smg", smg_id.map(|id| id.to_string())),
        ("tvdb", tvdb_id),
        ("tmdb", tmdb_id.map(|id| id.to_string())),
        ("imdb", imdb_id),
    ] {
        let Some(value) = value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !external_ids.iter().any(|external_id| {
            external_id.source.eq_ignore_ascii_case(source) && external_id.value == value
        }) {
            external_ids.push(ExternalId {
                source: source.to_string(),
                value,
            });
        }
    }
    Ok(NewTitle {
        name,
        facet: parsed_facet,
        monitored,
        tags,
        external_ids,
        root_folder_id,
        min_availability,
        poster_url: None,
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

#[cfg(test)]
mod tests {
    use crate::types::MediaFacetValue;
    use scryer_domain::MediaFacet;

    #[test]
    fn media_facet_value_maps_series_to_series_domain() {
        assert_eq!(MediaFacetValue::Series.into_domain(), MediaFacet::Series);
    }
}
