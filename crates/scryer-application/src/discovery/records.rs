use super::*;

pub(super) fn public_feed_sections(
    result: &DiscoveryDashboardResult,
) -> impl Iterator<Item = &DiscoveryDashboardSection> {
    result
        .sections
        .iter()
        .filter(|section| !discovery_section_is_complete_the_collection(&section.section_type))
}

pub(super) fn discovery_section_is_complete_the_collection(section_type: &str) -> bool {
    section_type
        .trim()
        .eq_ignore_ascii_case("COMPLETE_THE_COLLECTION")
}

pub(super) fn public_feed_section_record(
    run_id: &str,
    index: usize,
    section: &DiscoveryDashboardSection,
    now: DateTime<Utc>,
) -> AppResult<DiscoverySectionRecord> {
    Ok(DiscoverySectionRecord {
        id: format!("{run_id}:section:{index}"),
        run_id: run_id.to_string(),
        section_id: section.section_id.clone(),
        section_type: section.section_type.clone(),
        surface: "public".to_string(),
        title: section.title.clone(),
        sort_index: index as i32,
        created_at: now,
        updated_at: now,
    })
}

pub(crate) fn snapshot_facet_records(
    run_id: &str,
    pages: &[DiscoveryContextSnapshotPageResult],
) -> AppResult<Vec<DiscoveryFacetRecord>> {
    let mut facets = Vec::new();
    for page in pages {
        for group in &page.facets {
            for value in &group.values {
                facets.push(DiscoveryFacetRecord {
                    run_id: run_id.to_string(),
                    facet_name: group.name.clone(),
                    facet_value: value.value.clone(),
                    smg_count: Some(i64::from(value.count)),
                    local_count: None,
                });
            }
        }
    }
    Ok(facets)
}

#[expect(
    clippy::too_many_arguments,
    reason = "discovery item persistence maps explicit run and projection fields"
)]
pub(super) fn discovery_item_record(
    run_id: &str,
    base_generation_id: &str,
    source_run_kind: &str,
    section_id: Option<String>,
    index: usize,
    item: &DiscoveryTitle,
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
    now: DateTime<Utc>,
) -> AppResult<DiscoveryItemRecord> {
    let library_provenance =
        discovery_item_library_provenance_records(item, provenance_by_subject_key);
    Ok(DiscoveryItemRecord {
        id: format!("{run_id}:item:{index}"),
        run_id: run_id.to_string(),
        base_generation_id: Some(base_generation_id.to_string()),
        source_run_kind: source_run_kind.to_string(),
        section_id,
        sort_index: index as i32,
        target_key: item.target_key.clone(),
        target_kind: item.target_kind.clone(),
        resolved: item.resolved,
        resolved_title_id: None,
        display_title: discovery_display_title(item).unwrap_or_default(),
        original_title: non_identifier_discovery_title(&item.original_title).map(str::to_string),
        sort_title: discovery_sort_title(item),
        year: item.year,
        poster_path: non_empty_string(&item.poster_path),
        poster_url: non_empty_string(&item.poster_url),
        background_url: non_empty_string(&item.background_url),
        overview: non_empty_string(&item.overview),
        content_type: non_empty_string(&item.content_type),
        canonical_tags: discovery_canonical_tags(item),
        is_adult: item.is_adult,
        content_ratings: item.content_ratings.clone(),
        rating: item.rating,
        rating_sources: item.rating_sources.clone(),
        external_ratings: item.external_ratings.clone(),
        external_ids: discovery_external_id_records(item),
        status_tags: item.status_tags.clone(),
        source_tags: discovery_source_tag_records(&item.source_tags),
        sources: item.sources.clone(),
        best_source: non_empty_string(&item.best_source),
        relation_types: item.relation_types.clone(),
        relation_subtypes: item.relation_subtypes.clone(),
        chart_signals: discovery_json_signal_values(&item.chart_signals),
        provider_signals: discovery_json_signal_values(&item.provider_signals),
        rank_components: discovery_rank_component_records(&item.rank_components),
        source_count: Some(item.source_count),
        edge_count: Some(item.edge_count),
        relation_count: Some(item.relation_count),
        source_subject_count: Some(item.source_subject_count),
        rank_score: Some(item.rank_score),
        recommendation_score: item.recommendation_score,
        base_rank: item.base_rank,
        matched_subject_keys: item.matched_subject_keys.clone(),
        matched_subject_titles: item.matched_subject_titles.clone(),
        matched_subject_count: item.matched_subject_count,
        library_provenance,
        tmdb_collection_id: item.tmdb_collection_id.map(|id| id.to_string()),
        tmdb_collection_name: non_empty_string(&item.tmdb_collection_name),
        owned_in_input: item.owned_in_input,
        studio_slug: item.studio_slug.clone(),
        person_ids: item.person_ids.clone(),
        facet_terms: discovery_canonical_facet_terms(item),
        context_terms: item.context_terms.clone(),
        change_subject_keys: item.change_subject_keys.clone(),
        removed_subject_keys: item.removed_subject_keys.clone(),
        tombstoned_by_run_id: None,
        tombstoned_at: None,
        created_at: now,
        updated_at: now,
    })
}

pub(crate) fn title_more_like_this_item_records(
    title_id: &str,
    source_target_keys: &[String],
    items: &[DiscoveryTitle],
    limit: usize,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    let run_id = format!("title:{title_id}:more_like_this");
    let provenance = HashMap::new();
    let source_keys = source_target_keys
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    for item in items {
        let target_key = item.target_key.trim();
        if target_key.is_empty()
            || discovery_target_key_parts(target_key).is_none()
            || discovery_display_title(item).is_none()
            || source_keys.contains(target_key)
            || !seen.insert(target_key.to_string())
        {
            continue;
        }
        let mut record = discovery_item_record(
            &run_id,
            &run_id,
            "title_more_like_this",
            None,
            records.len(),
            item,
            &provenance,
            now,
        )?;
        record.base_generation_id = None;
        record.matched_subject_keys.clear();
        record.matched_subject_titles.clear();
        record.matched_subject_count = 0;
        record.library_provenance.clear();
        record.owned_in_input = false;
        record.resolved_title_id = None;
        records.push(record);
        if records.len() >= limit {
            break;
        }
    }
    Ok(records)
}

pub(crate) fn title_recommendations_subject(
    title: &Title,
    external_ids: &[ExternalId],
) -> Option<(DiscoverySubjectInput, Vec<String>)> {
    let mut ids = BTreeSet::<(String, String)>::new();
    for external_id in title.external_ids.iter().chain(external_ids.iter()) {
        let raw_source = external_id.source.trim().to_ascii_lowercase();
        let source = if matches!(raw_source.as_str(), "imdb" | "imdb_id") {
            "imdb".to_string()
        } else {
            let Some(source) = normalize_supported_external_id_source(&external_id.source) else {
                continue;
            };
            source
        };
        let value = external_id.value.trim();
        if value.is_empty() {
            continue;
        }
        let value = if source == "imdb" {
            let Some(normalized) = crate::normalize::normalize_imdb_id(value) else {
                continue;
            };
            normalized
        } else {
            let Some(id) = parse_positive_external_numeric_id(value) else {
                continue;
            };
            id.to_string()
        };
        ids.insert((source, value));
    }
    if let Some(imdb_id) = title
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
    {
        ids.insert(("imdb".to_string(), imdb_id));
    }

    if ids.is_empty() {
        return None;
    }

    let mut subject = DiscoverySubjectInput {
        kind: Some(title.facet.as_str().to_string()),
        facet: Some(title.facet.as_str().to_string()),
        ..Default::default()
    };
    let mut target_keys = Vec::new();
    for (source, value) in &ids {
        subject.external_ids.push(DiscoveryExternalIdInput {
            source: source.clone(),
            value: value.clone(),
        });
        match source.as_str() {
            "tvdb" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.tvdb_id = subject.tvdb_id.or(Some(id));
                    let key = keyed_discovery_target_key("tvdb", &title.facet, value);
                    target_keys.push(key);
                }
            }
            "tmdb" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.tmdb_id = subject.tmdb_id.or(Some(id));
                    let key = keyed_discovery_target_key("tmdb", &title.facet, value);
                    target_keys.push(key);
                }
            }
            "mal" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.mal_id = subject.mal_id.or(Some(id));
                    let key = format!("mal:anime:{value}");
                    target_keys.push(key);
                }
            }
            "anidb" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.anidb_id = subject.anidb_id.or(Some(id));
                    let key = format!("anidb:anime:{value}");
                    target_keys.push(key);
                }
            }
            "anilist" => {
                let key = format!("anilist:anime:{value}");
                target_keys.push(key);
            }
            "imdb" => {
                let key = format!("imdb:title:{value}");
                target_keys.push(key);
            }
            _ => {}
        }
    }
    subject.key = title_recommendations_preferred_subject_key(&ids, &title.facet);
    Some((subject, unique_discovery_text_terms(target_keys)))
}

pub(super) fn title_recommendations_preferred_subject_key(
    ids: &BTreeSet<(String, String)>,
    facet: &MediaFacet,
) -> Option<String> {
    for source in ["tvdb", "tmdb", "mal", "anidb", "anilist", "imdb"] {
        let Some((_, value)) = ids.iter().find(|(candidate, _)| candidate == source) else {
            continue;
        };
        return Some(match source {
            "tvdb" | "tmdb" => keyed_discovery_target_key(source, facet, value),
            "mal" | "anidb" | "anilist" => format!("{source}:anime:{value}"),
            "imdb" => format!("imdb:title:{value}"),
            _ => unreachable!("source priority list contains only known sources"),
        });
    }
    None
}

pub(super) fn keyed_discovery_target_key(source: &str, facet: &MediaFacet, value: &str) -> String {
    let kind = match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Series | MediaFacet::Anime => "series",
    };
    format!("{source}:{kind}:{value}")
}

pub(super) fn parse_positive_i32(value: &str) -> Option<i32> {
    value.trim().parse::<i32>().ok().filter(|value| *value > 0)
}

pub(super) fn discovery_target_key_parts(target_key: &str) -> Option<(String, String, String)> {
    let mut parts = target_key.split(':');
    let source = parts.next()?.trim();
    let kind = parts.next()?.trim();
    let value = parts.next()?.trim();
    if source.is_empty() || kind.is_empty() || value.is_empty() {
        return None;
    }
    Some((
        source.to_ascii_lowercase(),
        kind.to_ascii_lowercase(),
        value.to_string(),
    ))
}

pub(super) fn discovery_local_external_id_sources(source: &str, kind: &str) -> Vec<String> {
    match source {
        "tvdb" => match kind {
            "movie" => vec!["tvdb".to_string(), "tvdb_movie".to_string()],
            _ => vec![
                "tvdb".to_string(),
                "tvdb_series".to_string(),
                "tvdb_show".to_string(),
            ],
        },
        "tmdb" => match kind {
            "movie" => vec!["tmdb".to_string(), "tmdb_movie".to_string()],
            _ => vec![
                "tmdb".to_string(),
                "tmdb_series".to_string(),
                "tmdb_tv".to_string(),
                "tmdb_show".to_string(),
            ],
        },
        "mal" => vec!["mal".to_string(), "myanimelist".to_string()],
        "anilist" => vec![
            "anilist".to_string(),
            "anilist_anime".to_string(),
            "anilist:anime".to_string(),
        ],
        "anidb" => vec!["anidb".to_string()],
        "imdb" => vec!["imdb".to_string()],
        _ => vec![source.to_string()],
    }
}

pub(super) fn discovery_local_external_id_values(kind: &str, value: &str) -> Vec<String> {
    unique_discovery_text_terms(vec![value.to_string(), format!("{kind}:{value}")])
}

pub(super) fn discovery_item_library_provenance_records(
    item: &DiscoveryTitle,
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
) -> Vec<DiscoveryItemLibraryProvenanceRecord> {
    let mut provenance = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push_subject_key = |subject_key: &str| {
        let subject_key = subject_key.trim();
        if subject_key.is_empty() {
            return;
        }
        let Some(entries) = provenance_by_subject_key.get(subject_key) else {
            return;
        };
        for entry in entries {
            if seen.insert((
                entry.subject_key.clone(),
                entry.title_id.clone(),
                entry.library_id.clone(),
            )) {
                provenance.push(DiscoveryItemLibraryProvenanceRecord {
                    subject_key: entry.subject_key.clone(),
                    title_id: entry.title_id.clone(),
                    library_id: Some(entry.library_id.clone()),
                });
            }
        }
    };

    for subject_key in &item.matched_subject_keys {
        push_subject_key(subject_key);
    }
    for subject_key in &item.change_subject_keys {
        push_subject_key(subject_key);
    }
    for subject_key in &item.removed_subject_keys {
        push_subject_key(subject_key);
    }
    if item.owned_in_input {
        push_subject_key(&item.target_key);
    }

    provenance
}

pub(super) fn discovery_source_tag_records(values: &[JsonValue]) -> Vec<DiscoverySourceTagRecord> {
    values
        .iter()
        .map(|value| {
            let category = json_object_string(value, &["category", "type"]);
            let name = json_object_string(value, &["name", "label", "value"]);
            DiscoverySourceTagRecord {
                category,
                name,
                values: unique_json_text_values(value),
            }
        })
        .collect()
}

pub(super) fn discovery_external_id_records(
    item: &DiscoveryTitle,
) -> Vec<DiscoveryExternalIdRecord> {
    item.external_ids
        .iter()
        .filter_map(|external_id| {
            let source = external_id.source.trim();
            let id = external_id.id.trim();
            let key = external_id.key.trim();
            if source.is_empty() || (id.is_empty() && key.is_empty()) {
                return None;
            }
            Some(DiscoveryExternalIdRecord {
                source: source.to_ascii_lowercase(),
                kind: external_id.kind.trim().to_ascii_lowercase(),
                id: id.to_string(),
                key: key.to_string(),
            })
        })
        .collect()
}

pub(super) fn discovery_canonical_facet_terms(item: &DiscoveryTitle) -> Vec<String> {
    let mut values = item.facet_terms.clone();
    for canonical_tag in &item.canonical_tags {
        values.extend(canonical_discovery_terms_from_canonical_tag(canonical_tag));
    }
    unique_discovery_text_terms(values)
}

pub(super) fn discovery_canonical_tags(
    item: &DiscoveryTitle,
) -> Vec<scryer_domain::CanonicalMediaTag> {
    item.canonical_tags
        .iter()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect()
}

pub(super) fn canonical_discovery_terms_from_canonical_tag(value: &JsonValue) -> Vec<String> {
    let mut terms = unique_json_text_values(value)
        .into_iter()
        .filter_map(|value| canonical_discovery_term(&value).map(str::to_string))
        .collect::<Vec<_>>();
    if !terms.is_empty() {
        return unique_discovery_text_terms(terms);
    }

    if !value.is_object() {
        return Vec::new();
    }
    let Some(category) = json_object_string(value, &["category", "type"]) else {
        return Vec::new();
    };
    let category = category.trim().to_ascii_lowercase();
    if category != "genre" && category != "theme" {
        return Vec::new();
    }
    let Some(label) = json_object_string(value, &["key", "name", "label", "value"]) else {
        return Vec::new();
    };
    let label = label.trim();
    if label.is_empty() {
        return Vec::new();
    }
    let tail = label
        .rsplit(':')
        .next()
        .unwrap_or(label)
        .trim()
        .to_ascii_lowercase()
        .replace(|character: char| !character.is_ascii_alphanumeric(), "-")
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if tail.is_empty() {
        return Vec::new();
    }
    terms.push(format!("canonical:{category}:{tail}"));
    unique_discovery_text_terms(terms)
}

pub(super) fn unique_discovery_text_terms(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                return None;
            }
            seen.insert(normalize_discovery_filter_value(&value))
                .then_some(value)
        })
        .collect()
}

pub(super) fn canonical_discovery_term(value: &str) -> Option<&str> {
    let value = value.trim();
    if canonical_discovery_term_tail(value, "genre").is_some()
        || canonical_discovery_term_tail(value, "theme").is_some()
    {
        Some(value)
    } else {
        None
    }
}

pub(super) fn canonical_discovery_term_tail<'a>(value: &'a str, kind: &str) -> Option<&'a str> {
    let value = value.trim();
    let mut parts = value.splitn(3, ':');
    if !parts.next()?.eq_ignore_ascii_case("canonical") {
        return None;
    }
    if !parts.next()?.eq_ignore_ascii_case(kind) {
        return None;
    }
    let tail = parts.next()?.trim();
    if tail.is_empty() {
        return None;
    }
    Some(tail)
}

pub(super) fn canonical_discovery_facet_label(value: &str, kind: &str) -> Option<String> {
    canonical_discovery_term_tail(value, kind).map(format_canonical_discovery_label)
}

pub(super) fn format_canonical_discovery_label(value: &str) -> String {
    value
        .split(|character: char| {
            character == '-' || character == '_' || character == ':' || character.is_whitespace()
        })
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => {
                    let mut word = first.to_uppercase().collect::<String>();
                    word.extend(characters.flat_map(char::to_lowercase));
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn discovery_json_signal_values(values: &[JsonValue]) -> Vec<String> {
    let mut signals = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        for signal in unique_json_text_values(value) {
            let key = normalize_discovery_filter_value(&signal);
            if !key.is_empty() && seen.insert(key) {
                signals.push(signal);
            }
        }
    }
    signals
}

pub(super) fn discovery_rank_component_records(
    values: &[JsonValue],
) -> Vec<DiscoveryRankComponentRecord> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| DiscoveryRankComponentRecord {
            component_index: index as i32,
            component_name: json_object_string(value, &["name", "key", "component", "type"]),
            component_value: json_object_string(
                value,
                &["value", "score", "weight", "contribution"],
            )
            .or_else(|| unique_json_text_values(value).first().cloned()),
        })
        .collect()
}

pub(super) fn json_object_string(value: &JsonValue, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(json_scalar_string)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

pub(super) fn json_scalar_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(value) => Some(value.clone()),
        JsonValue::Number(value) => Some(value.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

pub(super) fn unique_json_text_values(value: &JsonValue) -> Vec<String> {
    let mut values = Vec::new();
    collect_json_text_values(value, &mut values);
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                return None;
            }
            let key = normalize_discovery_filter_value(&value);
            seen.insert(key).then_some(value)
        })
        .collect()
}

pub(crate) fn pending_context_changes_need_snapshot_reconciliation(
    pending_changes: &[DiscoveryPendingContextChangeRecord],
) -> bool {
    match pending_context_changes_resolved_key_count(pending_changes) {
        Ok(count) => count > DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS,
        Err(_) => true,
    }
}

pub(crate) fn pending_context_changes_resolved_key_count(
    pending_changes: &[DiscoveryPendingContextChangeRecord],
) -> AppResult<usize> {
    let mut keys = BTreeSet::new();
    for change in pending_changes {
        let change_type = discovery_change_type_from_str(&change.change_type)?;
        match change_type {
            DiscoveryContextChangeType::Added | DiscoveryContextChangeType::Updated => {
                keys.insert(required_pending_context_subject_key(change)?);
            }
            DiscoveryContextChangeType::Removed => {
                keys.insert(
                    change
                        .previous_subject_key
                        .clone()
                        .or_else(|| change.subject_key.clone())
                        .ok_or_else(|| {
                            AppError::Validation(format!(
                                "pending discovery removal {} is missing subject key",
                                change.id
                            ))
                        })?,
                );
            }
            DiscoveryContextChangeType::Rematched => {
                keys.insert(required_pending_context_subject_key(change)?);
                keys.insert(change.previous_subject_key.clone().ok_or_else(|| {
                    AppError::Validation(format!(
                        "pending discovery rematch {} is missing previous subject key",
                        change.id
                    ))
                })?);
            }
        }
    }
    Ok(keys.len())
}

pub(super) fn required_pending_context_subject_key(
    change: &DiscoveryPendingContextChangeRecord,
) -> AppResult<String> {
    change.subject_key.clone().ok_or_else(|| {
        AppError::Validation(format!(
            "pending discovery change {} is missing subject key",
            change.id
        ))
    })
}

pub(super) fn changed_subject_from_pending(
    change: &DiscoveryPendingContextChangeRecord,
) -> AppResult<DiscoveryContextChangedSubjectInput> {
    let raw_subject = change.raw_subject_json.as_deref().ok_or_else(|| {
        AppError::Validation(format!(
            "pending discovery change {} is missing raw subject JSON",
            change.id
        ))
    })?;
    let subject = serde_json::from_str::<DiscoverySubjectInput>(raw_subject).map_err(|error| {
        AppError::Validation(format!(
            "pending discovery change {} has invalid raw subject JSON: {error}",
            change.id
        ))
    })?;
    let previous_subject = change
        .raw_previous_subject_json
        .as_deref()
        .map(|raw| {
            serde_json::from_str::<DiscoverySubjectInput>(raw).map_err(|error| {
                AppError::Validation(format!(
                    "pending discovery change {} has invalid previous subject JSON: {error}",
                    change.id
                ))
            })
        })
        .transpose()?;
    Ok(DiscoveryContextChangedSubjectInput {
        subject,
        change_type: discovery_change_type_from_str(&change.change_type)?,
        previous_subject,
    })
}

pub(super) fn discovery_change_type_from_str(value: &str) -> AppResult<DiscoveryContextChangeType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "added" | "add" => Ok(DiscoveryContextChangeType::Added),
        "updated" | "update" => Ok(DiscoveryContextChangeType::Updated),
        "removed" | "delete" | "deleted" => Ok(DiscoveryContextChangeType::Removed),
        "rematched" | "rematch" => Ok(DiscoveryContextChangeType::Rematched),
        other => Err(AppError::Validation(format!(
            "unsupported discovery context change type {other}"
        ))),
    }
}

pub(super) fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(super) fn non_identifier_discovery_title(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() || value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let mut parts = value.splitn(3, ':');
    let Some(provider) = parts.next() else {
        return Some(value);
    };
    let Some(kind) = parts.next() else {
        return Some(value);
    };
    let Some(_) = parts.next() else {
        return Some(value);
    };
    let source_like = !provider.is_empty()
        && provider
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-'))
        && provider
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
        && !kind.is_empty()
        && kind
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-'));
    (!source_like).then_some(value)
}

pub(super) fn discovery_display_title(item: &DiscoveryTitle) -> Option<String> {
    non_identifier_discovery_title(&item.display_title)
        .or_else(|| non_identifier_discovery_title(&item.original_title))
        .map(str::to_string)
}

pub(super) fn discovery_sort_title(item: &DiscoveryTitle) -> Option<String> {
    let title = discovery_display_title(item)?;
    let sort_title = title_catalog_sort_input(&title);
    non_identifier_discovery_title(&sort_title)
        .or_else(|| non_identifier_discovery_title(&title))
        .map(str::to_string)
}

pub(super) fn discovery_json_error(error: serde_json::Error) -> AppError {
    AppError::Repository(format!("failed to encode discovery payload JSON: {error}"))
}
