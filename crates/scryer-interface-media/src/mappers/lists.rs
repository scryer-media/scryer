//! Mapping between the public-list use cases and their GraphQL types.

use std::collections::BTreeMap;

use async_graphql::MaybeUndefined;
use scryer_application::AppError;
use scryer_application::lists::catalog::{
    ListProviderGroup, ListProviderManifest, ListProviderNote, ListProviderTile, ListSourceParam,
    ListUrlPattern, facet_of,
};
use scryer_application::lists::{
    ListExclusionView, ListMembershipPage, ListPreview, ListPreviewItem, ListProviderSettingField,
    ListProviderSettings, ListSourceDraft, MemberListPolicy, NewListExclusionInput,
    PublicListInput, PublicListPatch, TitleListMembership,
};
use scryer_domain::{
    ExternalId, ListCounts, ListExclusionScope, ListFilter, ListMembership, ListRoute,
    ListSubscription, ListSyncRun, ListSyncStatus, MediaRequestOrigin,
};

use super::*;

fn count(value: u64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn facets(values: &[MediaFacet]) -> Vec<MediaFacetValue> {
    values
        .iter()
        .cloned()
        .map(MediaFacetValue::from_domain)
        .collect()
}

fn params_payload(params: &BTreeMap<String, String>) -> Vec<ListParamPayload> {
    params
        .iter()
        .map(|(key, value)| ListParamPayload {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

fn params_from_input(params: Vec<ListParamInput>) -> BTreeMap<String, String> {
    params
        .into_iter()
        .map(|param| (param.key.trim().to_string(), param.value))
        .filter(|(key, _)| !key.is_empty())
        .collect()
}

fn id_or_none(value: Option<ID>) -> Option<String> {
    value
        .map(|id| id.to_string().trim().to_string())
        .filter(|id| !id.is_empty())
}

fn cap_from_input(value: i32) -> Result<u32, AppError> {
    u32::try_from(value)
        .map_err(|_| AppError::Validation("maxPerSync must not be negative".to_string()))
}

/// A media request's origin. A personal list is named by kind only, so every
/// viewer sees the same projection.
pub fn from_media_request_origin(
    origin: &MediaRequestOrigin,
    public_list_name: Option<String>,
) -> MediaRequestOriginPayload {
    match origin {
        MediaRequestOrigin::Manual => MediaRequestOriginPayload {
            kind: MediaRequestOriginKindValue::Manual,
            public_subscription_id: None,
            public_list_name: None,
        },
        MediaRequestOrigin::PublicList { subscription_id } => MediaRequestOriginPayload {
            kind: MediaRequestOriginKindValue::PublicList,
            public_subscription_id: Some(subscription_id.clone().into()),
            public_list_name,
        },
        MediaRequestOrigin::PersonalList { .. } => MediaRequestOriginPayload {
            kind: MediaRequestOriginKindValue::PersonalList,
            public_subscription_id: None,
            public_list_name: None,
        },
    }
}

fn from_tile(tile: ListProviderTile) -> ListProviderTilePayload {
    ListProviderTilePayload {
        bg: tile.bg,
        ink: tile.ink,
        abbr: tile.abbr,
    }
}

fn from_source_param(param: ListSourceParam) -> ListSourceParamPayload {
    ListSourceParamPayload {
        key: param.key,
        label: param.label,
        param_type: ListSourceParamTypeValue::from_domain(param.param_type),
        options: param.options,
        required: param.required,
    }
}

fn from_group(group: ListProviderGroup) -> ListProviderGroupPayload {
    ListProviderGroupPayload {
        label: group.label,
        auth_badge: ListAuthBadgeValue::from_domain(group.auth_badge),
        items: group
            .items
            .into_iter()
            .map(|item| ListProviderItemPayload {
                id: item.id,
                name: item.name,
                description: item.description,
                kinds: item
                    .kinds
                    .into_iter()
                    .map(|kind| MediaFacetValue::from_domain(facet_of(kind)))
                    .collect(),
                source_type: item.source_type,
                params: item.params.into_iter().map(from_source_param).collect(),
                personal: item.personal,
                default_interval_seconds: i32::try_from(item.default_interval_seconds)
                    .unwrap_or(i32::MAX),
            })
            .collect(),
    }
}

fn from_note(note: ListProviderNote) -> ListProviderNotePayload {
    ListProviderNotePayload {
        tone: ListNoteToneValue::from_domain(note.tone),
        text_key: note.text_key,
    }
}

fn from_url_pattern(pattern: ListUrlPattern) -> ListUrlPatternPayload {
    ListUrlPatternPayload {
        pattern: pattern.pattern,
        source_type: pattern.source_type,
        captures: pattern
            .captures
            .into_iter()
            .map(|capture| ListUrlPatternCapturePayload {
                group: capture.group,
                param: capture.param,
            })
            .collect(),
    }
}

pub fn from_list_provider(manifest: ListProviderManifest) -> ListProviderPayload {
    ListProviderPayload {
        provider_type: manifest.provider_type,
        name: manifest.name,
        summary: manifest.summary,
        blurb: manifest.blurb,
        tile: manifest.tile.map(from_tile),
        coverage: facets(&manifest.coverage),
        groups: manifest.groups.into_iter().map(from_group).collect(),
        notes: manifest.notes.into_iter().map(from_note).collect(),
        url_patterns: manifest
            .url_patterns
            .into_iter()
            .map(from_url_pattern)
            .collect(),
        config_fields: manifest
            .config_fields
            .into_iter()
            .map(|field| ListProviderSettingFieldPayload {
                value: None,
                ..from_list_provider_setting_field(field)
            })
            .collect(),
    }
}

fn from_list_provider_setting_field(
    field: ListProviderSettingField,
) -> ListProviderSettingFieldPayload {
    ListProviderSettingFieldPayload {
        key: field.key,
        label: field.label,
        help_text: field.help_text,
        field_type: PluginConfigFieldTypeValue::from_domain(field.field_type),
        required: field.required,
        secret: field.secret,
        is_set: field.is_set,
        value: if field.secret { None } else { field.value },
    }
}

pub fn from_list_provider_settings(settings: ListProviderSettings) -> ListProviderSettingsPayload {
    ListProviderSettingsPayload {
        provider_type: settings.provider_type,
        fields: settings
            .fields
            .into_iter()
            .map(from_list_provider_setting_field)
            .collect(),
    }
}

/// Setting changes keyed by setting key. A later change to the same key
/// wins; a blank key is dropped.
pub fn list_provider_setting_changes_from_input(
    changes: Vec<ListProviderSettingChangeInput>,
) -> BTreeMap<String, Option<String>> {
    changes
        .into_iter()
        .map(|change| (change.key.trim().to_string(), change.value))
        .filter(|(key, _)| !key.is_empty())
        .collect()
}

pub fn from_title_list_membership(membership: TitleListMembership) -> TitleListMembershipPayload {
    TitleListMembershipPayload {
        subscription_id: membership.subscription_id.into(),
        name: membership.name,
        state: ListMembershipStateValue::from_domain(membership.state),
        added_by_list: membership.added_by_list,
        left_at: membership.left_at,
    }
}

fn from_route(route: ListRoute) -> ListRoutePayload {
    ListRoutePayload {
        kind: MediaFacetValue::from_domain(route.kind),
        library_id: route.library_id.into(),
        quality_profile_id: route.quality_profile_id.map(Into::into),
        root_folder_id: route.root_folder_id.map(Into::into),
        monitor_type: route.monitor_type,
        min_availability: route.min_availability,
        use_season_folders: route.use_season_folders,
        release_numbering: route.release_numbering,
        tags: route.tags,
    }
}

fn route_from_input(input: ListRouteInput) -> Result<ListRoute, AppError> {
    let library_id = input.library_id.to_string().trim().to_string();
    if library_id.is_empty() {
        return Err(AppError::Validation(
            "a list route needs a library".to_string(),
        ));
    }
    Ok(ListRoute {
        kind: input.kind.into_domain(),
        library_id,
        quality_profile_id: id_or_none(input.quality_profile_id),
        root_folder_id: id_or_none(input.root_folder_id),
        monitor_type: input.monitor_type.trim().to_string(),
        min_availability: input.min_availability,
        use_season_folders: input.use_season_folders,
        release_numbering: input.release_numbering,
        tags: input.tags,
    })
}

fn from_filter(filter: ListFilter) -> ListFilterPayload {
    let empty = |kind| ListFilterPayload {
        kind,
        scale: None,
        value: None,
        from: None,
        to: None,
        values: Vec::new(),
    };
    match filter {
        ListFilter::RatingAtLeast { scale, value } => ListFilterPayload {
            scale: Some(scale),
            value: Some(value),
            ..empty(ListFilterKindValue::RatingAtLeast)
        },
        ListFilter::ReleaseYear { from, to } => ListFilterPayload {
            from,
            to,
            ..empty(ListFilterKindValue::ReleaseYear)
        },
        ListFilter::ExcludeGenres { genres } => ListFilterPayload {
            values: genres,
            ..empty(ListFilterKindValue::ExcludeGenres)
        },
        ListFilter::Format { formats } => ListFilterPayload {
            values: formats,
            ..empty(ListFilterKindValue::Format)
        },
        ListFilter::Language { languages } => ListFilterPayload {
            values: languages,
            ..empty(ListFilterKindValue::Language)
        },
        ListFilter::SkipOnMyStreamingServices => {
            empty(ListFilterKindValue::SkipOnMyStreamingServices)
        }
        ListFilter::ReleasedOnly => empty(ListFilterKindValue::ReleasedOnly),
        ListFilter::DirectorCreditsOnly => empty(ListFilterKindValue::DirectorCreditsOnly),
        ListFilter::NotSequelWithoutBase => empty(ListFilterKindValue::NotSequelWithoutBase),
    }
}

fn filter_from_input(input: ListFilterInput) -> Result<ListFilter, AppError> {
    let values = || {
        input
            .values
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
    };
    Ok(match input.kind {
        ListFilterKindValue::RatingAtLeast => {
            let scale = input
                .scale
                .as_deref()
                .map(str::trim)
                .filter(|scale| !scale.is_empty())
                .ok_or_else(|| AppError::Validation("a rating filter needs a scale".to_string()))?;
            let value = input
                .value
                .filter(|value| value.is_finite())
                .ok_or_else(|| AppError::Validation("a rating filter needs a value".to_string()))?;
            ListFilter::RatingAtLeast {
                scale: scale.to_string(),
                value,
            }
        }
        ListFilterKindValue::ReleaseYear => ListFilter::ReleaseYear {
            from: input.from,
            to: input.to,
        },
        ListFilterKindValue::ExcludeGenres => ListFilter::ExcludeGenres { genres: values() },
        ListFilterKindValue::Format => ListFilter::Format { formats: values() },
        ListFilterKindValue::Language => ListFilter::Language {
            languages: values(),
        },
        ListFilterKindValue::SkipOnMyStreamingServices => ListFilter::SkipOnMyStreamingServices,
        ListFilterKindValue::ReleasedOnly => ListFilter::ReleasedOnly,
        ListFilterKindValue::DirectorCreditsOnly => ListFilter::DirectorCreditsOnly,
        ListFilterKindValue::NotSequelWithoutBase => ListFilter::NotSequelWithoutBase,
    })
}

fn routes_from_input(routes: Vec<ListRouteInput>) -> Result<Vec<ListRoute>, AppError> {
    routes.into_iter().map(route_from_input).collect()
}

fn filters_from_input(filters: Vec<ListFilterInput>) -> Result<Vec<ListFilter>, AppError> {
    filters.into_iter().map(filter_from_input).collect()
}

fn from_sync_status(sync: ListSyncStatus) -> ListSyncStatusPayload {
    ListSyncStatusPayload {
        state: ListSyncStateValue::from_domain(sync.state),
        last_at: sync.last_at,
        next_at: sync.next_at,
        error_message: sync.error_message,
        error_at: sync.error_at,
        paused_until: sync.paused_until,
    }
}

fn from_counts(counts: ListCounts) -> ListCountsPayload {
    ListCountsPayload {
        total: count(counts.total),
        in_library: count(counts.in_library),
        added: count(counts.added),
        requested: count(counts.requested),
        held: count(counts.held),
        filtered: count(counts.filtered),
        excluded: count(counts.excluded),
        unresolved: count(counts.unresolved),
    }
}

/// A public subscription as the use case already redacted it for the viewer.
pub fn from_list_subscription(subscription: ListSubscription) -> ListSubscriptionPayload {
    ListSubscriptionPayload {
        id: subscription.id.into(),
        scope: ListScopeValue::from_domain(subscription.scope),
        name: subscription.name,
        provider_url: subscription.provider_url,
        source: ListSourcePayload {
            params: params_payload(&subscription.source.params),
            provider: subscription.source.provider,
            source_type: subscription.source.source_type,
        },
        kinds: facets(&subscription.kinds),
        enabled: subscription.enabled,
        mode: ListModeValue::from_domain(subscription.mode),
        routes: subscription.routes.into_iter().map(from_route).collect(),
        filters: subscription.filters.into_iter().map(from_filter).collect(),
        max_per_sync: subscription
            .max_per_sync
            .map(|cap| i32::try_from(cap).unwrap_or(i32::MAX)),
        on_leave: ListOnLeaveValue::from_domain(subscription.on_leave),
        interval_seconds: i32::try_from(subscription.interval_seconds).unwrap_or(i32::MAX),
        sync: from_sync_status(subscription.sync),
        counts: from_counts(subscription.counts),
        created_at: subscription.created_at,
        updated_at: subscription.updated_at,
    }
}

fn from_membership(row: ListMembership) -> ListMembershipPayload {
    ListMembershipPayload {
        item_key: row.item_key,
        rank: row.rank.map(|rank| i32::try_from(rank).unwrap_or(i32::MAX)),
        season: row.season,
        kind: MediaFacetValue::from_domain(row.kind),
        state: ListMembershipStateValue::from_domain(row.state),
        state_reason: row.state_reason,
        display_title: row.display_title,
        year: row.year,
        title_id: row.title_id.map(Into::into),
        request_id: row.request_id.map(Into::into),
        added_by_list: row.added_by_list,
        first_seen_at: row.first_seen_at,
        last_seen_at: row.last_seen_at,
        left_at: row.left_at,
    }
}

pub fn from_list_membership_page(page: ListMembershipPage) -> ListMembershipPagePayload {
    ListMembershipPagePayload {
        total_count: count(page.total_count),
        items: page.items.into_iter().map(from_membership).collect(),
    }
}

pub fn from_list_sync_run(run: ListSyncRun) -> ListSyncRunPayload {
    ListSyncRunPayload {
        id: run.id.into(),
        started_at: run.started_at,
        finished_at: run.finished_at,
        outcome: ListSyncRunOutcomeValue::from_domain(run.outcome),
        counts: from_counts(run.counts),
        error_message: run.error_message,
    }
}

fn from_preview_item(item: ListPreviewItem) -> ListPreviewItemPayload {
    ListPreviewItemPayload {
        display_title: item.display_title.unwrap_or_else(|| item.item_key.clone()),
        item_key: item.item_key,
        year: item.year,
        kind: item.kind.map(MediaFacetValue::from_domain),
        poster_url: item.poster_url,
    }
}

pub fn from_list_preview(preview: ListPreview) -> ListPreviewPayload {
    ListPreviewPayload {
        recognized: preview.recognized,
        params: params_payload(&preview.params),
        provider: preview.provider,
        source_type: preview.source_type,
        name: preview.name,
        kinds: facets(&preview.kinds),
        total: count(preview.total),
        in_library: count(preview.in_library),
        filtered: count(preview.filtered),
        excluded: count(preview.excluded),
        unresolved: count(preview.unresolved),
        would_add: preview
            .would_add
            .into_iter()
            .map(from_preview_item)
            .collect(),
    }
}

pub fn from_list_exclusion(view: ListExclusionView) -> ListExclusionPayload {
    let exclusion = view.exclusion;
    ListExclusionPayload {
        id: exclusion.id.into(),
        kind: MediaFacetValue::from_domain(exclusion.kind),
        external_ids: exclusion
            .external_ids
            .into_iter()
            .map(|id| ExternalIdPayload {
                source: id.source,
                kind: id.kind,
                value: id.value,
            })
            .collect(),
        display_title: exclusion.display_title,
        year: exclusion.year,
        scope: ListExclusionScopeValue::from_domain(&exclusion.scope),
        subscription_id: exclusion
            .scope
            .subscription_id()
            .map(|id| id.to_string().into()),
        subscription_name: view.subscription_name,
        created_at: exclusion.created_at,
    }
}

pub fn from_member_list_policy(entry: MemberListPolicy) -> MemberListPolicyPayload {
    MemberListPolicyPayload {
        user: ListMemberPayload {
            id: entry.user.id.into(),
            username: entry.user.username,
        },
        policy: ListPolicyValue::from_domain(entry.policy),
        list_requests_last_30d: count(entry.list_requests_last_30d),
    }
}

pub fn list_source_draft_from_input(input: ListSourceInput) -> ListSourceDraft {
    ListSourceDraft {
        provider: input.provider,
        source_type: input.source_type,
        params: params_from_input(input.params),
        url: input.url,
    }
}

/// A public follow. A personal scope is refused here: personal lists are
/// followed through the member's own surface.
pub fn public_list_input_from_input(
    input: SubscribeListInput,
) -> Result<PublicListInput, AppError> {
    if input.scope != ListScopeValue::Public {
        return Err(AppError::Validation(
            "only public lists can be followed here".to_string(),
        ));
    }
    Ok(PublicListInput {
        provider: input.provider,
        source_type: input.source_type,
        params: params_from_input(input.params),
        url: input.url,
        name: input.name,
        kinds: input.kinds.map(|kinds| {
            kinds
                .into_iter()
                .map(MediaFacetValue::into_domain)
                .collect()
        }),
        mode: input.mode.into_domain(),
        routes: routes_from_input(input.routes)?,
        filters: filters_from_input(input.filters)?,
        max_per_sync: input.max_per_sync.map(cap_from_input).transpose()?,
        on_leave: input.on_leave.into_domain(),
    })
}

pub fn public_list_patch_from_input(
    input: UpdateListSubscriptionInput,
) -> Result<PublicListPatch, AppError> {
    let max_per_sync = match input.max_per_sync {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(cap) => Some(Some(cap_from_input(cap)?)),
    };
    Ok(PublicListPatch {
        name: input.name,
        kinds: input.kinds.map(|kinds| {
            kinds
                .into_iter()
                .map(MediaFacetValue::into_domain)
                .collect()
        }),
        mode: input.mode.map(ListModeValue::into_domain),
        routes: input.routes.map(routes_from_input).transpose()?,
        filters: input.filters.map(filters_from_input).transpose()?,
        max_per_sync,
        on_leave: input.on_leave.map(ListOnLeaveValue::into_domain),
    })
}

pub fn list_exclusion_input_from_input(
    input: AddListExclusionInput,
) -> Result<NewListExclusionInput, AppError> {
    let scope = match input.scope {
        ListExclusionScopeValue::AllLists => ListExclusionScope::AllLists,
        ListExclusionScopeValue::List => ListExclusionScope::List {
            subscription_id: id_or_none(input.subscription_id).ok_or_else(|| {
                AppError::Validation("a list exclusion needs its list".to_string())
            })?,
        },
    };
    Ok(NewListExclusionInput {
        kind: input.kind.into_domain(),
        external_ids: input
            .external_ids
            .into_iter()
            .map(|id| ExternalId {
                source: id.source,
                kind: id.kind,
                value: id.value,
            })
            .collect(),
        display_title: input.display_title,
        year: input.year,
        scope,
    })
}

#[cfg(test)]
#[path = "lists_tests.rs"]
mod tests;
