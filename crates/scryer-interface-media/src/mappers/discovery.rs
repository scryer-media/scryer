use super::*;

pub fn from_discovery_sync_status(status: DiscoverySyncStatus) -> DiscoverySyncStatusPayload {
    DiscoverySyncStatusPayload {
        state: from_discovery_sync_state(status.state),
        pending_context_change_count: Long(status.pending_context_change_count),
    }
}

pub fn from_discovery_sync_state(state: DiscoverySyncStateRecord) -> DiscoverySyncStatePayload {
    DiscoverySyncStatePayload {
        last_success_generation_id: state.last_success_generation_id.map(ID::from),
        last_public_feed_generation_id: state.last_public_feed_generation_id.map(ID::from),
        last_context_snapshot_completed_at: state.last_context_snapshot_completed_at,
        last_incremental_reload_completed_at: state.last_incremental_reload_completed_at,
        last_public_feed_completed_at: state.last_public_feed_completed_at,
        next_context_snapshot_eligible_at: state.next_context_snapshot_eligible_at,
        next_incremental_reload_eligible_at: state.next_incremental_reload_eligible_at,
        next_public_feed_eligible_at: state.next_public_feed_eligible_at,
        updated_at: state.updated_at,
    }
}

const DISCOVERY_HOME_MAX_SECTION_LIMIT: i32 = 100;

fn discovery_home_canonical_tag_keys(
    field_name: &str,
    keys: Option<Vec<String>>,
) -> scryer_application::AppResult<Vec<String>> {
    let keys = keys.unwrap_or_default();
    if keys.iter().any(|key| key.trim().is_empty()) {
        return Err(scryer_application::AppError::validation_refused(
            scryer_interface_core::refusal::INTERFACE_DISCOVERY_FILTER_BLANK,
            format!("discovery home {field_name} entries must not be blank"),
        ));
    }
    Ok(keys)
}

pub fn discovery_home_query_from_input(
    input: Option<DiscoveryHomeInput>,
) -> scryer_application::AppResult<DiscoveryHomeQuery> {
    let input = input.unwrap_or_default();
    let filters = input.filters.unwrap_or_default();
    let limit_per_section = match input.limit_per_section {
        Some(value) if !(1..=DISCOVERY_HOME_MAX_SECTION_LIMIT).contains(&value) => {
            return Err(scryer_application::AppError::validation_refused(
                scryer_interface_core::refusal::INTERFACE_DISCOVERY_LIMIT_INVALID,
                format!(
                    "discovery home limitPerSection must be between 1 and {DISCOVERY_HOME_MAX_SECTION_LIMIT}"
                ),
            ));
        }
        Some(value) => value as usize,
        None => 25,
    };
    let minimum_rating = filters.minimum_rating;
    if minimum_rating.is_some_and(|value| !value.is_finite() || !(0.0..=10.0).contains(&value)) {
        return Err(scryer_application::AppError::validation_refused(
            scryer_interface_core::refusal::INTERFACE_DISCOVERY_RATING_INVALID,
            "discovery home minimumRating must be a finite value between 0 and 10".to_owned(),
        ));
    }
    if let (Some(minimum_year), Some(maximum_year)) = (filters.minimum_year, filters.maximum_year)
        && minimum_year > maximum_year
    {
        return Err(scryer_application::AppError::validation_refused(
            scryer_interface_core::refusal::INTERFACE_DISCOVERY_YEARS_INVALID,
            "discovery home minimumYear must not exceed maximumYear".to_owned(),
        ));
    }
    let genre_tag_keys = discovery_home_canonical_tag_keys("genreTagKeys", filters.genre_tag_keys)?;
    let theme_tag_keys = discovery_home_canonical_tag_keys("themeTagKeys", filters.theme_tag_keys)?;

    Ok(DiscoveryHomeQuery {
        include_public: input.include_public.unwrap_or(true),
        include_personalized: input.include_personalized.unwrap_or(true),
        include_unresolved: input.include_unresolved.unwrap_or(false),
        limit_per_section,
        filters: DiscoveryHomeFilters {
            content_types: filters
                .content_types
                .unwrap_or_default()
                .into_iter()
                .map(MediaFacetValue::as_scope_id)
                .map(str::to_owned)
                .collect(),
            genre_tag_keys,
            theme_tag_keys,
            studio_slugs: filters.studio_slugs.unwrap_or_default(),
            minimum_year: filters.minimum_year,
            maximum_year: filters.maximum_year,
            minimum_rating,
        },
    })
}

pub fn discovery_home_filter_options_query_from_input(
    input: Option<DiscoveryHomeFilterOptionsInput>,
) -> DiscoveryHomeQuery {
    let input = input.unwrap_or_default();
    DiscoveryHomeQuery {
        include_public: input.include_public.unwrap_or(true),
        include_personalized: input.include_personalized.unwrap_or(true),
        include_unresolved: input.include_unresolved.unwrap_or(false),
        ..DiscoveryHomeQuery::default()
    }
}

pub fn discovery_items_query_from_input(input: Option<DiscoveryItemsInput>) -> DiscoveryItemsQuery {
    let input = input.unwrap_or_default();
    DiscoveryItemsQuery {
        query: input.query,
        target_keys: Vec::new(),
        target_kinds: input.target_kinds.unwrap_or_default(),
        sources: input.sources.unwrap_or_default(),
        relation_types: input.relation_types.unwrap_or_default(),
        relation_subtypes: input.relation_subtypes.unwrap_or_default(),
        genres: input.genres.unwrap_or_default(),
        status_tags: input.status_tags.unwrap_or_default(),
        facet_terms: input.facet_terms.unwrap_or_default(),
        include_owned: input.include_owned.unwrap_or(false),
        include_unresolved: input.include_unresolved.unwrap_or(false),
        include_public: input.include_public.unwrap_or(false),
        limit: input.limit.map(|value| value.max(1) as usize).unwrap_or(50),
        offset: input.offset.map(|value| value.max(0) as usize).unwrap_or(0),
    }
}

pub fn discovery_item_detail_query_from_input(
    input: DiscoveryItemDetailInput,
) -> DiscoveryItemDetailQuery {
    DiscoveryItemDetailQuery {
        target_key: input.target_key,
        include_unresolved: input.include_unresolved.unwrap_or(true),
    }
}

pub fn catalog_discovery_query_from_input(input: CatalogDiscoveryInput) -> CatalogDiscoveryQuery {
    CatalogDiscoveryQuery {
        facet: input.facet.into_domain(),
        library_ids: input
            .library_ids
            .unwrap_or_default()
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        include_unresolved: input.include_unresolved.unwrap_or(true),
        limit_per_group: input
            .limit_per_group
            .map(|value| value.max(1) as usize)
            .unwrap_or(12),
        max_groups: input
            .max_groups
            .map(|value| value.max(1) as usize)
            .unwrap_or(6),
    }
}

pub fn from_discovery_home(app: &AppUseCase, result: DiscoveryHomeResult) -> DiscoveryHomePayload {
    DiscoveryHomePayload {
        status: from_discovery_sync_status(result.status),
        hero_item: result.hero_item.map(|item| from_discovery_item(app, item)),
        public_sections: result
            .public_sections
            .into_iter()
            .map(|section| from_discovery_section(app, section))
            .collect(),
        personalized_sections: result
            .personalized_sections
            .into_iter()
            .map(|section| from_discovery_section(app, section))
            .collect(),
        complete_collection: result
            .complete_collection
            .map(|section| from_discovery_section(app, section)),
        facets: result
            .facets
            .into_iter()
            .map(from_discovery_facet)
            .collect(),
        can_view_personalized: result.can_view_personalized,
    }
}

pub fn from_discovery_home_cards(
    app: &AppUseCase,
    result: DiscoveryHomeResult,
) -> scryer_application::AppResult<DiscoveryHomeCardsPayload> {
    let hero_item = result
        .hero_item
        .map(|item| from_discovery_home_hero(app, item))
        .transpose()?;
    let public_sections = result
        .public_sections
        .into_iter()
        .map(|section| from_discovery_home_section(app, section))
        .collect::<scryer_application::AppResult<Vec<_>>>()?;
    let personalized_sections = result
        .personalized_sections
        .into_iter()
        .map(|section| from_discovery_home_section(app, section))
        .collect::<scryer_application::AppResult<Vec<_>>>()?;
    let complete_collection = result
        .complete_collection
        .map(|section| from_discovery_home_section(app, section))
        .transpose()?;

    Ok(DiscoveryHomeCardsPayload {
        status: from_discovery_sync_status(result.status),
        hero_item,
        public_sections,
        personalized_sections,
        complete_collection,
        can_view_personalized: result.can_view_personalized,
    })
}

pub fn from_discovery_home_filter_options(
    options: DiscoveryHomeFilterOptions,
) -> DiscoveryHomeFilterOptionsPayload {
    DiscoveryHomeFilterOptionsPayload {
        genres: options
            .genres
            .into_iter()
            .map(|option| CanonicalTagFilterOptionPayload {
                key: option.key,
                name: option.name,
            })
            .collect(),
        themes: options
            .themes
            .into_iter()
            .map(|option| CanonicalTagFilterOptionPayload {
                key: option.key,
                name: option.name,
            })
            .collect(),
        studio_slugs: options.studio_slugs,
    }
}

pub fn from_discovery_items_result(
    app: &AppUseCase,
    result: DiscoveryItemsResult,
) -> DiscoveryItemsPayload {
    DiscoveryItemsPayload {
        items: result
            .items
            .into_iter()
            .map(|item| from_discovery_item(app, item))
            .collect(),
        total_count: Long(result.total_count),
        can_view_personalized: result.can_view_personalized,
    }
}

pub fn from_catalog_discovery(
    app: &AppUseCase,
    result: CatalogDiscoveryResult,
) -> CatalogDiscoveryPayload {
    CatalogDiscoveryPayload {
        can_view_personalized: result.can_view_personalized,
        groups: result
            .groups
            .into_iter()
            .map(|group| from_catalog_discovery_group(app, group))
            .collect(),
    }
}

fn from_catalog_discovery_group(
    app: &AppUseCase,
    group: CatalogDiscoveryGroup,
) -> CatalogDiscoveryGroupPayload {
    CatalogDiscoveryGroupPayload {
        id: group.id,
        kind: from_catalog_discovery_group_kind(group.kind),
        surface: from_catalog_discovery_surface(group.surface),
        label_value: group.label_value,
        total_count: Long(group.total_count),
        items: group
            .items
            .into_iter()
            .map(|item| from_discovery_item(app, item))
            .collect(),
    }
}

fn from_catalog_discovery_group_kind(
    kind: CatalogDiscoveryGroupKind,
) -> CatalogDiscoveryGroupKindValue {
    match kind {
        CatalogDiscoveryGroupKind::PublicTop => CatalogDiscoveryGroupKindValue::PublicTop,
        CatalogDiscoveryGroupKind::PublicSection => CatalogDiscoveryGroupKindValue::PublicSection,
        CatalogDiscoveryGroupKind::GenreAffinity => CatalogDiscoveryGroupKindValue::GenreAffinity,
        CatalogDiscoveryGroupKind::ThemeAffinity => CatalogDiscoveryGroupKindValue::ThemeAffinity,
        CatalogDiscoveryGroupKind::Acclaimed => CatalogDiscoveryGroupKindValue::Acclaimed,
        CatalogDiscoveryGroupKind::CompleteCollection => {
            CatalogDiscoveryGroupKindValue::CompleteCollection
        }
        CatalogDiscoveryGroupKind::Fallback => CatalogDiscoveryGroupKindValue::Fallback,
    }
}

fn from_catalog_discovery_surface(
    surface: CatalogDiscoverySurface,
) -> CatalogDiscoverySurfaceValue {
    match surface {
        CatalogDiscoverySurface::Public => CatalogDiscoverySurfaceValue::Public,
        CatalogDiscoverySurface::Personalized => CatalogDiscoverySurfaceValue::Personalized,
    }
}

pub fn from_discovery_section(
    app: &AppUseCase,
    section: DiscoverySectionResult,
) -> DiscoverySectionPayload {
    DiscoverySectionPayload {
        section_id: section.section_id,
        section_type: section.section_type,
        title: section.title,
        surface: section.surface,
        total_count: Long(section.total_count),
        items: section
            .items
            .into_iter()
            .map(|item| from_discovery_item(app, item))
            .collect(),
    }
}

pub(super) fn discovery_home_media_facet(
    value: &str,
    field_name: &str,
) -> scryer_application::AppResult<MediaFacetValue> {
    MediaFacetValue::parse(value).ok_or_else(|| {
        scryer_application::AppError::validation_refused(
            scryer_interface_core::refusal::INTERFACE_DISCOVERY_FACET_UNSUPPORTED,
            format!("discovery home {field_name} must be a supported media facet: {value}"),
        )
    })
}

fn from_discovery_home_section(
    app: &AppUseCase,
    section: DiscoverySectionResult,
) -> scryer_application::AppResult<DiscoveryHomeSectionPayload> {
    let surface = discovery_surface_value(&section.surface)?;
    let items = section
        .items
        .into_iter()
        .map(|item| from_discovery_home_card(app, item))
        .collect::<scryer_application::AppResult<Vec<_>>>()?;

    Ok(DiscoveryHomeSectionPayload {
        section_id: section.section_id,
        section_type: section.section_type,
        title: section.title,
        surface,
        total_count: Long(section.total_count),
        items,
    })
}

pub(super) fn discovery_surface_value(
    value: &str,
) -> scryer_application::AppResult<DiscoverySurfaceValue> {
    let surface = match value {
        "public" => DiscoverySurfaceValue::Public,
        "personalized" => DiscoverySurfaceValue::Personalized,
        "mixed" => DiscoverySurfaceValue::Mixed,
        value => {
            return Err(scryer_application::AppError::validation_refused(
                scryer_interface_core::refusal::INTERFACE_DISCOVERY_SURFACE_UNSUPPORTED,
                format!("discovery home section has an unsupported surface: {value}"),
            ));
        }
    };
    Ok(surface)
}

fn from_discovery_home_card(
    app: &AppUseCase,
    item: DiscoveryItemRecord,
) -> scryer_application::AppResult<DiscoveryHomeCardPayload> {
    let target_kind = discovery_home_media_facet(&item.target_kind, "card targetKind")?;
    let content_type = discovery_home_media_facet(
        item.content_type
            .as_deref()
            .unwrap_or(item.target_kind.as_str()),
        "card contentType",
    )?;

    let (poster_url, _) = discovery_image_urls(app, &item);
    Ok(DiscoveryHomeCardPayload {
        id: item.id.into(),
        target_key: item.target_key,
        target_kind,
        display_title: item.display_title,
        original_title: item.original_title,
        sort_title: item.sort_title,
        year: item.year,
        poster_url,
        content_type,
        is_adult: item.is_adult,
        owned_in_input: item.owned_in_input,
    })
}

fn from_discovery_home_hero(
    app: &AppUseCase,
    item: DiscoveryItemRecord,
) -> scryer_application::AppResult<DiscoveryHomeHeroPayload> {
    let target_kind = discovery_home_media_facet(&item.target_kind, "hero targetKind")?;
    let content_type = discovery_home_media_facet(
        item.content_type
            .as_deref()
            .unwrap_or(item.target_kind.as_str()),
        "hero contentType",
    )?;

    let (poster_url, background_url) = discovery_image_urls(app, &item);
    Ok(DiscoveryHomeHeroPayload {
        id: item.id.into(),
        target_key: item.target_key,
        target_kind,
        display_title: item.display_title,
        original_title: item.original_title,
        sort_title: item.sort_title,
        year: item.year,
        poster_url,
        background_url,
        overview: item.overview,
        content_type,
        is_adult: item.is_adult,
        rating: item.rating,
        rating_sources: item.rating_sources,
        external_ratings: item
            .external_ratings
            .into_iter()
            .map(|rating| DiscoveryExternalRatingPayload {
                source: rating.source,
                value: rating.value,
                score: rating.score,
                normalized: rating.normalized,
                votes: rating.votes,
                url: rating.url,
            })
            .collect(),
        genre_tags: item
            .canonical_tags
            .into_iter()
            .filter(|tag| tag.category.eq_ignore_ascii_case("genre"))
            .map(|tag| CanonicalMediaTagPayload {
                key: tag.key,
                category: tag.category,
                name: tag.name,
                confidence: tag.confidence,
                sources: tag.sources,
                source_tag_keys: tag.source_tag_keys,
                is_adult: tag.is_adult,
                is_spoiler: tag.is_spoiler,
            })
            .collect(),
        matched_subject_count: item.matched_subject_count,
        owned_in_input: item.owned_in_input,
    })
}

fn discovery_image_urls(
    app: &AppUseCase,
    item: &DiscoveryItemRecord,
) -> (Option<String>, Option<String>) {
    let (owner_type, owner_id) = item
        .resolved_title_id
        .as_deref()
        .map(|title_id| ("title", title_id))
        .unwrap_or(("discovery", item.id.as_str()));
    let poster_source =
        preferred_discovery_poster_source(item.poster_path.as_deref(), item.poster_url.as_deref());
    let poster = app.media_image_url(
        poster_source.as_deref(),
        Some(owner_type),
        Some(owner_id),
        ImageProxyKind::Poster,
        "w250",
    );
    let background = app.media_image_url(
        item.background_url.as_deref(),
        Some(owner_type),
        Some(owner_id),
        ImageProxyKind::Fanart,
        "w1280",
    );
    (poster, background)
}

pub(super) fn preferred_discovery_poster_source(
    poster_path: Option<&str>,
    poster_url: Option<&str>,
) -> Option<String> {
    let tmdb_source = poster_path.and_then(|value| {
        let value = value.trim();
        if value.starts_with("https://image.tmdb.org/")
            || value.starts_with("http://image.tmdb.org/")
        {
            Some(value.to_string())
        } else if value.starts_with('/') && !value.starts_with("//") {
            Some(format!("https://image.tmdb.org/t/p/original{value}"))
        } else {
            None
        }
    });

    tmdb_source.or_else(|| {
        poster_url
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

pub fn from_discovery_item(app: &AppUseCase, item: DiscoveryItemRecord) -> DiscoveryItemPayload {
    let (poster_url, background_url) = discovery_image_urls(app, &item);
    DiscoveryItemPayload {
        id: item.id.into(),
        target_key: item.target_key,
        target_kind: item.target_kind,
        resolved: item.resolved,
        resolved_title_id: item.resolved_title_id.map(ID::from),
        display_title: item.display_title,
        original_title: item.original_title,
        sort_title: item.sort_title,
        year: item.year,
        poster_url,
        background_url,
        overview: item.overview,
        content_type: item.content_type,
        canonical_tags: item
            .canonical_tags
            .into_iter()
            .map(|tag| CanonicalMediaTagPayload {
                key: tag.key,
                category: tag.category,
                name: tag.name,
                confidence: tag.confidence,
                sources: tag.sources,
                source_tag_keys: tag.source_tag_keys,
                is_adult: tag.is_adult,
                is_spoiler: tag.is_spoiler,
            })
            .collect(),
        is_adult: item.is_adult,
        content_ratings: item
            .content_ratings
            .into_iter()
            .map(|rating| DiscoveryContentRatingPayload {
                country: rating.country,
                certifications: rating
                    .certifications
                    .into_iter()
                    .map(|certification| DiscoveryContentCertificationPayload {
                        value: certification.value,
                        source: certification.source,
                        release_type: certification.release_type,
                    })
                    .collect(),
                age_rating: rating.age_rating,
                age_rating_source: rating.age_rating_source,
            })
            .collect(),
        rating: item.rating,
        rating_sources: item.rating_sources,
        external_ratings: item
            .external_ratings
            .into_iter()
            .map(|rating| DiscoveryExternalRatingPayload {
                source: rating.source,
                value: rating.value,
                score: rating.score,
                normalized: rating.normalized,
                votes: rating.votes,
                url: rating.url,
            })
            .collect(),
        external_ids: item
            .external_ids
            .into_iter()
            .map(|external_id| DiscoveryExternalIdPayload {
                source: external_id.source,
                kind: external_id.kind,
                id: external_id.id,
                key: external_id.key,
            })
            .collect(),
        status_tags: item.status_tags,
        source_tags: item
            .source_tags
            .into_iter()
            .flat_map(|tag| {
                tag.name
                    .into_iter()
                    .chain(tag.category)
                    .chain(tag.values)
                    .collect::<Vec<_>>()
            })
            .collect(),
        sources: item.sources,
        best_source: item.best_source,
        relation_types: item.relation_types,
        relation_subtypes: item.relation_subtypes,
        source_count: item.source_count,
        edge_count: item.edge_count,
        relation_count: item.relation_count,
        source_subject_count: item.source_subject_count,
        rank_score: item.rank_score,
        matched_subject_titles: item.matched_subject_titles,
        matched_subject_count: item.matched_subject_count,
        tmdb_collection_id: item.tmdb_collection_id,
        tmdb_collection_name: item.tmdb_collection_name,
        owned_in_input: item.owned_in_input,
        studio_slug: item.studio_slug,
        person_ids: item.person_ids,
        facet_terms: item.facet_terms,
        context_terms: item.context_terms,
    }
}

pub fn from_discovery_facet(facet: DiscoveryFacetRecord) -> DiscoveryFacetPayload {
    DiscoveryFacetPayload {
        name: facet.facet_name,
        value: facet.facet_value,
        smg_count: facet.smg_count.map(Long),
        local_count: facet.local_count.map(Long),
    }
}
