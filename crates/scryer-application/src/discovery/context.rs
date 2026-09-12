use super::*;

pub(crate) fn build_discovery_library_context(
    titles: &[Title],
    defaults: DiscoveryContextDefaults,
) -> DiscoveryLibraryContext {
    let mut subject_provenance = titles
        .iter()
        .filter_map(build_discovery_library_subject)
        .collect::<Vec<_>>();

    subject_provenance.sort_by(|left, right| {
        left.subject_key
            .cmp(&right.subject_key)
            .then_with(|| left.canonical.cmp(&right.canonical))
            .then_with(|| left.library_id.cmp(&right.library_id))
            .then_with(|| left.title_id.cmp(&right.title_id))
    });
    subject_provenance.dedup_by(|left, right| {
        left.subject_key == right.subject_key
            && left.title_id == right.title_id
            && left.library_id == right.library_id
    });

    let mut subjects = subject_provenance.clone();
    subjects.dedup_by(|left, right| left.subject_key == right.subject_key);

    let canonical_subjects = subjects
        .iter()
        .map(|subject| subject.canonical.clone())
        .collect::<Vec<_>>();

    let medium_mix = DiscoveryLibraryMediumMix::from_titles(titles);

    DiscoveryLibraryContext {
        subjects,
        subject_provenance,
        medium_mix,
        fingerprint: discovery_context_fingerprint(&defaults, medium_mix, &canonical_subjects),
    }
}

impl DiscoveryLibraryContext {
    pub(crate) fn snapshot_submit_input(
        &self,
        defaults: &DiscoveryContextDefaults,
    ) -> DiscoveryContextSnapshotSubmitInput {
        DiscoveryContextSnapshotSubmitInput {
            subjects: self
                .subjects
                .iter()
                .map(|subject| subject.subject.clone())
                .collect(),
            region: defaults.region.clone(),
            language: defaults.language.clone(),
            max_items: defaults.max_items as i32,
            include_owned: defaults.include_owned,
            include_unresolved: defaults.include_unresolved,
            medium_mix: self.medium_mix.to_context_input(),
            context_fingerprint: Some(self.fingerprint.clone()),
        }
    }

    pub(crate) fn incremental_changes_input(
        &self,
        defaults: &DiscoveryContextDefaults,
        pending_changes: &[DiscoveryPendingContextChangeRecord],
        previous_context_fingerprint: &str,
    ) -> AppResult<DiscoveryContextChangesInput> {
        let resolved_key_count = pending_context_changes_resolved_key_count(pending_changes)?;
        if resolved_key_count > DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS {
            return Err(AppError::Validation(format!(
                "discovery incremental reload resolves to {resolved_key_count} changed subjects, above SMG limit {}",
                DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS
            )));
        }

        let context_subject_keys = self
            .subjects
            .iter()
            .map(|subject| subject.subject_key.clone())
            .collect();
        let changed_subjects = pending_changes
            .iter()
            .map(changed_subject_from_pending)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(DiscoveryContextChangesInput {
            context_subject_keys,
            changed_subjects,
            region: defaults.region.clone(),
            language: defaults.language.clone(),
            max_items: defaults.max_items as i32,
            include_owned: defaults.include_owned,
            include_unresolved: defaults.include_unresolved,
            medium_mix: self.medium_mix.to_context_input(),
            context_fingerprint: Some(self.fingerprint.clone()),
            previous_context_fingerprint: Some(previous_context_fingerprint.to_string()),
        })
    }

    pub(crate) fn submitted_subject_records(
        &self,
        run_id: &str,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>> {
        self.subject_provenance
            .iter()
            .map(|subject| {
                let external_ids_json = serde_json::to_string(&subject.subject.external_ids)
                    .map_err(discovery_json_error)?;
                let raw_subject_json =
                    serde_json::to_string(&subject.subject).map_err(discovery_json_error)?;
                Ok(DiscoverySubmittedSubjectRecord {
                    run_id: run_id.to_string(),
                    subject_key: subject.subject_key.clone(),
                    title_id: Some(subject.title_id.clone()),
                    library_id: Some(subject.library_id.clone()),
                    library_facet: Some(subject.facet.clone()),
                    title_kind: subject.subject.kind.clone(),
                    display_title: Some(subject.title_name.clone()),
                    external_ids_json,
                    raw_subject_json,
                })
            })
            .collect()
    }

    pub(crate) fn subject_provenance_by_key(
        &self,
    ) -> HashMap<String, Vec<DiscoveryLibraryProvenance>> {
        let mut provenance_by_key = HashMap::<String, Vec<DiscoveryLibraryProvenance>>::new();
        for subject in &self.subject_provenance {
            provenance_by_key
                .entry(subject.subject_key.clone())
                .or_default()
                .push(DiscoveryLibraryProvenance {
                    subject_key: subject.subject_key.clone(),
                    title_id: Some(subject.title_id.clone()),
                    library_id: subject.library_id.clone(),
                });
        }
        provenance_by_key
    }
}

pub(crate) fn pending_context_change_from_domain_event(
    scope_key: &str,
    event: &DomainEvent,
) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
    match &event.payload {
        DomainEventPayload::TitleAdded(data) => {
            title_context_change_record(scope_key, event, &data.title, None, "added", None)
        }
        DomainEventPayload::TitleUpdated(data) => {
            title_context_change_record(scope_key, event, &data.title, None, "updated", None)
        }
        DomainEventPayload::TitleDeleted(data) => {
            title_context_change_record(scope_key, event, &data.title, None, "removed", None)
        }
        DomainEventPayload::TitleRematched(data) => {
            let mut current_ids = data.title.external_ids.clone();
            current_ids.tvdb_id = Some(data.new_tvdb_id.clone());
            let previous_ids = data.old_tvdb_id.as_ref().map(|old_tvdb_id| {
                let mut external_ids = data.title.external_ids.clone();
                external_ids.tvdb_id = Some(old_tvdb_id.clone());
                external_ids
            });
            title_context_change_record(
                scope_key,
                event,
                &data.title,
                previous_ids.as_ref(),
                "rematched",
                Some(&current_ids),
            )
        }
        _ => Ok(None),
    }
}

pub(super) fn build_discovery_library_subject(title: &Title) -> Option<DiscoveryLibrarySubject> {
    let parts =
        build_discovery_subject_parts(&title.facet, normalized_supported_external_ids(title))?;
    Some(DiscoveryLibrarySubject {
        title_id: title.id.clone(),
        library_id: title.library_id.clone(),
        title_name: title.name.clone(),
        facet: parts.facet,
        subject_key: parts.subject_key,
        subject: parts.subject,
        canonical: parts.canonical,
    })
}

pub(super) fn title_context_change_record(
    scope_key: &str,
    event: &DomainEvent,
    title: &TitleContextSnapshot,
    previous_external_ids: Option<&DomainExternalIds>,
    change_type: &str,
    current_external_ids: Option<&DomainExternalIds>,
) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
    let current = match build_discovery_title_context_subject(
        title,
        current_external_ids.unwrap_or(&title.external_ids),
    ) {
        Some(subject) => subject,
        None => return Ok(None),
    };
    let previous = previous_external_ids
        .and_then(|external_ids| build_discovery_title_context_subject(title, external_ids));
    let title_id = event.title_id.clone();
    let identity = title_id.as_deref().unwrap_or(current.subject_key.as_str());
    let raw_subject_json = serde_json::to_string(&current.subject).map_err(discovery_json_error)?;
    let raw_previous_subject_json = previous
        .as_ref()
        .map(|subject| serde_json::to_string(&subject.subject).map_err(discovery_json_error))
        .transpose()?;

    Ok(Some(DiscoveryPendingContextChangeRecord {
        id: format!("{scope_key}:title:{identity}"),
        scope_key: scope_key.to_string(),
        subject_key: Some(current.subject_key),
        previous_subject_key: previous.map(|subject| subject.subject_key),
        change_type: change_type.to_string(),
        title_id,
        previous_title_id: None,
        library_facet: Some(current.facet),
        raw_subject_json: Some(raw_subject_json),
        raw_previous_subject_json,
        first_seen_sequence: Some(event.sequence),
        last_seen_sequence: Some(event.sequence),
        first_seen_at: event.occurred_at,
        last_seen_at: event.occurred_at,
    }))
}

pub(crate) fn coalesce_pending_context_change(
    existing: Option<&DiscoveryPendingContextChangeRecord>,
    incoming: DiscoveryPendingContextChangeRecord,
) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
    let Some(existing) = existing else {
        return Ok(Some(incoming));
    };

    let existing_type = discovery_change_type_from_str(&existing.change_type)?;
    let incoming_type = discovery_change_type_from_str(&incoming.change_type)?;

    if matches!(existing_type, DiscoveryContextChangeType::Added)
        && matches!(incoming_type, DiscoveryContextChangeType::Removed)
    {
        return Ok(None);
    }

    let mut merged = incoming;
    merged.id = existing.id.clone();
    merged.scope_key = existing.scope_key.clone();
    merged.first_seen_sequence = existing.first_seen_sequence.or(merged.first_seen_sequence);
    merged.first_seen_at = existing.first_seen_at;

    match (existing_type, incoming_type) {
        (DiscoveryContextChangeType::Added, _) => {
            merged.change_type = "added".to_string();
            merged.previous_subject_key = None;
            merged.previous_title_id = None;
            merged.raw_previous_subject_json = None;
        }
        (_, DiscoveryContextChangeType::Removed) => {
            merged.change_type = "removed".to_string();
            if merged.previous_subject_key.is_none() {
                merged.previous_subject_key = existing
                    .previous_subject_key
                    .clone()
                    .or_else(|| existing.subject_key.clone());
            }
            if merged.raw_previous_subject_json.is_none() {
                merged.raw_previous_subject_json = existing
                    .raw_previous_subject_json
                    .clone()
                    .or_else(|| existing.raw_subject_json.clone());
            }
            if merged.previous_title_id.is_none() {
                merged.previous_title_id = existing
                    .previous_title_id
                    .clone()
                    .or_else(|| existing.title_id.clone());
            }
        }
        (DiscoveryContextChangeType::Removed, DiscoveryContextChangeType::Added)
        | (DiscoveryContextChangeType::Removed, DiscoveryContextChangeType::Updated) => {
            merged.change_type = "rematched".to_string();
            merged.previous_subject_key = existing
                .previous_subject_key
                .clone()
                .or_else(|| existing.subject_key.clone());
            merged.raw_previous_subject_json = existing
                .raw_previous_subject_json
                .clone()
                .or_else(|| existing.raw_subject_json.clone());
            merged.previous_title_id = existing
                .previous_title_id
                .clone()
                .or_else(|| existing.title_id.clone());
        }
        (DiscoveryContextChangeType::Updated, DiscoveryContextChangeType::Updated) => {
            merged.change_type = "updated".to_string();
            merged.previous_subject_key = existing.previous_subject_key.clone();
            merged.raw_previous_subject_json = existing.raw_previous_subject_json.clone();
            merged.previous_title_id = existing.previous_title_id.clone();
        }
        (DiscoveryContextChangeType::Removed, DiscoveryContextChangeType::Rematched)
        | (DiscoveryContextChangeType::Updated, DiscoveryContextChangeType::Rematched)
        | (DiscoveryContextChangeType::Rematched, DiscoveryContextChangeType::Updated)
        | (DiscoveryContextChangeType::Rematched, DiscoveryContextChangeType::Rematched) => {
            merged.change_type = "rematched".to_string();
            if merged.previous_subject_key.is_none() {
                merged.previous_subject_key = existing
                    .previous_subject_key
                    .clone()
                    .or_else(|| existing.subject_key.clone());
            }
            if merged.raw_previous_subject_json.is_none() {
                merged.raw_previous_subject_json = existing
                    .raw_previous_subject_json
                    .clone()
                    .or_else(|| existing.raw_subject_json.clone());
            }
            if merged.previous_title_id.is_none() {
                merged.previous_title_id = existing
                    .previous_title_id
                    .clone()
                    .or_else(|| existing.title_id.clone());
            }
        }
        (_, DiscoveryContextChangeType::Added) => {
            merged.change_type = "added".to_string();
            merged.previous_subject_key = None;
            merged.previous_title_id = None;
            merged.raw_previous_subject_json = None;
        }
    }

    Ok(Some(merged))
}

pub(super) fn build_discovery_title_context_subject(
    title: &TitleContextSnapshot,
    external_ids: &DomainExternalIds,
) -> Option<DiscoverySubjectParts> {
    build_discovery_subject_parts(
        &title.facet,
        normalized_supported_domain_external_ids(external_ids),
    )
}

pub(super) fn build_discovery_subject_parts(
    facet: &MediaFacet,
    external_ids: Vec<CanonicalExternalId>,
) -> Option<DiscoverySubjectParts> {
    if external_ids.is_empty() {
        return None;
    }

    let facet_name = facet.as_str().to_string();
    let kind = discovery_resolver_kind_from_facet(facet);
    let tvdb_id = unique_i32_external_id(&external_ids, "tvdb");
    let tmdb_id = unique_i32_external_id(&external_ids, "tmdb");
    let mal_id = unique_i32_external_id(&external_ids, "mal");
    let anidb_id = unique_i32_external_id(&external_ids, "anidb");
    let subject_key =
        fallback_discovery_subject_key(&kind, &external_ids, tvdb_id, tmdb_id, mal_id, anidb_id);
    let canonical = CanonicalSubject {
        subject_key: subject_key.clone(),
        key: None,
        kind,
        facet: facet_name.clone(),
        external_ids,
    };

    let subject = DiscoverySubjectInput {
        key: canonical.key.clone(),
        tvdb_id,
        tmdb_id,
        mal_id,
        anidb_id,
        kind: Some(canonical.kind.clone()),
        facet: Some(canonical.facet.clone()),
        external_ids: canonical
            .external_ids
            .iter()
            .map(|external_id| DiscoveryExternalIdInput {
                source: external_id.source.clone(),
                value: external_id.value.clone(),
            })
            .collect(),
    };

    Some(DiscoverySubjectParts {
        facet: facet_name,
        subject_key,
        subject,
        canonical,
    })
}

pub(super) fn normalized_supported_external_ids(title: &Title) -> Vec<CanonicalExternalId> {
    title
        .external_ids
        .iter()
        .filter_map(|external_id| {
            normalize_supported_external_id(&external_id.source, &external_id.value)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn normalized_supported_domain_external_ids(
    external_ids: &DomainExternalIds,
) -> Vec<CanonicalExternalId> {
    [
        ("tvdb", external_ids.tvdb_id.as_deref()),
        ("tmdb", external_ids.tmdb_id.as_deref()),
        ("anidb", external_ids.anidb_id.as_deref()),
    ]
    .into_iter()
    .filter_map(|(source, value)| normalize_supported_external_id(source, value?))
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect()
}

pub(super) fn discovery_resolver_kind_from_facet(facet: &MediaFacet) -> String {
    match facet {
        MediaFacet::Anime => "series".to_string(),
        _ => facet.as_str().to_string(),
    }
}

pub(super) fn normalize_supported_external_id(
    source: &str,
    value: &str,
) -> Option<CanonicalExternalId> {
    let source = normalize_supported_external_id_source(source)?;
    let value = parse_positive_external_numeric_id(value)?.to_string();
    Some(CanonicalExternalId { source, value })
}

pub(super) fn normalize_supported_external_id_source(source: &str) -> Option<String> {
    match source.trim().to_ascii_lowercase().as_str() {
        "tvdb" | "thetvdb" | "tvdb_show" | "tvdb_series" | "tvdb_movie" => Some("tvdb".to_string()),
        "tmdb" | "themoviedb" | "tmdb_tv" | "tmdb_show" | "tmdb_series" | "tmdb_movie" => {
            Some("tmdb".to_string())
        }
        "anidb" => Some("anidb".to_string()),
        "mal" | "myanimelist" => Some("mal".to_string()),
        "anilist" | "anilist_anime" | "anilist:anime" => Some("anilist".to_string()),
        _ => None,
    }
}

pub(super) fn parse_positive_external_numeric_id(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let value = value.rsplit(':').next().unwrap_or(value).trim();
    value.parse::<i64>().ok().filter(|id| *id > 0)
}

pub(super) fn unique_i32_external_id(
    external_ids: &[CanonicalExternalId],
    source: &str,
) -> Option<i32> {
    let values = external_ids
        .iter()
        .filter(|external_id| external_id.source == source)
        .filter_map(|external_id| external_id.value.parse::<i32>().ok())
        .filter(|id| *id > 0)
        .collect::<BTreeSet<_>>();

    if values.len() == 1 {
        values.into_iter().next()
    } else {
        None
    }
}

pub(super) fn fallback_discovery_subject_key(
    kind: &str,
    external_ids: &[CanonicalExternalId],
    tvdb_id: Option<i32>,
    tmdb_id: Option<i32>,
    mal_id: Option<i32>,
    anidb_id: Option<i32>,
) -> String {
    if let Some(tvdb_id) = tvdb_id {
        return format!("tvdb:{kind}:{tvdb_id}");
    }
    if let Some(tmdb_id) = tmdb_id {
        return format!("tmdb:{kind}:{tmdb_id}");
    }
    if let Some(mal_id) = mal_id {
        return format!("mal:anime:{mal_id}");
    }
    if let Some(anidb_id) = anidb_id {
        return format!("anidb:anime:{anidb_id}");
    }

    for source in ["tvdb", "tmdb", "anidb", "mal", "anilist"] {
        for external_id in external_ids
            .iter()
            .filter(|external_id| external_id.source == source)
        {
            match source {
                "tvdb" => return format!("tvdb:{kind}:{}", external_id.value),
                "tmdb" => return format!("tmdb:{kind}:{}", external_id.value),
                "mal" => return format!("mal:anime:{}", external_id.value),
                "anidb" => return format!("anidb:anime:{}", external_id.value),
                "anilist" => return format!("anilist:anime:{}", external_id.value),
                _ => {}
            }
        }
    }

    let bytes = serde_json::to_vec(external_ids)
        .expect("discovery subject key fallback input should always serialize");
    format!("local:{}", blake3::hash(&bytes).to_hex())
}

/// Schema version 2 adds the library's medium mix: the same counts SMG uses to
/// exclude a medium are part of what identifies the run, so a library that
/// gains (or loses) its first anime gets a fresh pool instead of reusing one
/// built under the old mix.
pub(super) fn discovery_context_fingerprint(
    defaults: &DiscoveryContextDefaults,
    medium_mix: DiscoveryLibraryMediumMix,
    subjects: &[CanonicalSubject],
) -> String {
    let context = CanonicalContext {
        schema_version: 2,
        defaults,
        medium_mix: medium_mix.to_context_input(),
        subjects,
    };
    let bytes = serde_json::to_vec(&context)
        .expect("discovery context fingerprint input should always serialize");
    format!("blake3:{}", blake3::hash(&bytes).to_hex())
}

pub(crate) fn snapshot_item_records(
    run_id: &str,
    base_generation_id: &str,
    items: &[DiscoveryTitle],
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            discovery_item_record(
                run_id,
                base_generation_id,
                "context_snapshot",
                None,
                index,
                item,
                provenance_by_subject_key,
                now,
            )
        })
        .collect()
}

pub(crate) fn incremental_item_records(
    run_id: &str,
    base_generation_id: &str,
    items: &[DiscoveryTitle],
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            discovery_item_record(
                run_id,
                base_generation_id,
                "context_incremental",
                None,
                index,
                item,
                provenance_by_subject_key,
                now,
            )
        })
        .collect()
}

pub(crate) fn public_feed_section_records(
    run_id: &str,
    result: &DiscoveryDashboardResult,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoverySectionRecord>> {
    public_feed_sections(result)
        .enumerate()
        .map(|(index, section)| public_feed_section_record(run_id, index, section, now))
        .collect()
}

pub(crate) fn public_feed_item_records(
    run_id: &str,
    result: &DiscoveryDashboardResult,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    let mut records = Vec::new();
    let empty_provenance = HashMap::<String, Vec<DiscoveryLibraryProvenance>>::new();
    for (section_index, section) in public_feed_sections(result).enumerate() {
        for (item_index, item) in section.items.iter().enumerate() {
            let mut record = discovery_item_record(
                run_id,
                run_id,
                "public_feed",
                Some(section.section_id.clone()),
                section_index * 10_000 + item_index,
                item,
                &empty_provenance,
                now,
            )?;
            record.matched_subject_keys.clear();
            record.matched_subject_titles.clear();
            record.matched_subject_count = 0;
            record.library_provenance.clear();
            records.push(record);
        }
    }
    Ok(records)
}
