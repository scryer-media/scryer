#[derive(Clone, Debug)]
enum PlannedEpisodeMemberDisposition {
    Import {
        episodes: Vec<scryer_domain::Episode>,
    },
    Ignore {
        episodes: Vec<scryer_domain::Episode>,
        reason_code: &'static str,
        message: String,
    },
    Hold {
        episodes: Vec<scryer_domain::Episode>,
        reason_code: &'static str,
        message: String,
    },
}

#[derive(Default)]
struct EpisodePackImportPlan {
    members: HashMap<PathBuf, PlannedEpisodeMemberDisposition>,
}

impl EpisodePackImportPlan {
    fn disposition_for(&self, source_path: &Path) -> Option<&PlannedEpisodeMemberDisposition> {
        self.members.get(source_path)
    }
}

struct VerifiedEpisodePack {
    declared_seasons: Option<HashSet<u32>>,
    is_extras_release: bool,
}

impl VerifiedEpisodePack {
    /// Whether the pack vouches for this catalog episode: a standard episode
    /// whose season the release name declares (a pack that declares no
    /// seasons — "Complete Series" — vouches for every standard episode).
    /// This is the one rule for which pack members import automatically and
    /// which Manual Import suggestions a multi-season pack keeps alive beyond
    /// the season or set the grab was scoped to.
    fn vouches_for(&self, episode: &scryer_domain::Episode) -> bool {
        episode.episode_type == scryer_domain::EpisodeType::Standard
            && self.declared_seasons.as_ref().is_none_or(|declared| {
                catalog_episode_season(episode).is_some_and(|season| declared.contains(&season))
            })
    }
}

enum PlannedMemberDraft {
    Resolved(Vec<scryer_domain::Episode>),
    AmbiguousNumbering {
        season: u32,
        season_local: Option<Vec<scryer_domain::Episode>>,
        absolute: Option<Vec<scryer_domain::Episode>>,
    },
    Ignore {
        reason_code: &'static str,
        message: String,
    },
    Hold {
        episodes: Vec<scryer_domain::Episode>,
        reason_code: &'static str,
        message: String,
    },
}

fn verified_episode_pack(
    release_evidence: &ReleaseEvidence,
    title: &scryer_domain::Title,
) -> Option<VerifiedEpisodePack> {
    let ReleaseEvidence::ScryerSubmission { scope, .. } = release_evidence else {
        return None;
    };
    let pack_scope = match scope {
        SubmissionScope::Collection { .. } => true,
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids.len() > 1,
        _ => false,
    };
    if !pack_scope {
        return None;
    }

    let release_title = release_evidence.release_title(None)?;
    let parsed =
        normalize_release_title_signal(parse_import_release_for_title(&release_title, title));
    let episode = parsed.episode.as_ref()?;
    let is_pack = episode.full_season
        || episode.is_series_pack
        || episode.is_multi_season
        || episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack;
    if !is_pack {
        return None;
    }

    let declared_seasons = if !episode.season_numbers.is_empty() {
        Some(episode.season_numbers.iter().copied().collect())
    } else {
        episode.season.map(|season| HashSet::from([season]))
    };

    Some(VerifiedEpisodePack {
        declared_seasons,
        is_extras_release: episode.is_season_extra,
    })
}

async fn build_episode_pack_import_plan(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    release_evidence: &ReleaseEvidence,
    source_root: &Path,
    video_files: &[PathBuf],
    expected_episode_ids: Option<&HashSet<String>>,
) -> AppResult<Option<EpisodePackImportPlan>> {
    let Some(pack) = verified_episode_pack(release_evidence, title) else {
        return Ok(None);
    };
    let catalog_episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await?;

    let mut drafts = Vec::with_capacity(video_files.len());
    for source_path in video_files {
        let draft = if pack.is_extras_release {
            PlannedMemberDraft::Hold {
                episodes: Vec::new(),
                reason_code: "season_extras_release",
                message: "Automatic import will not treat a season-range extras release as normal episodes. Open Manual Import and assign any wanted content."
                    .to_string(),
            }
        } else {
            plan_episode_pack_member(title, source_root, source_path, &catalog_episodes)
        };
        let draft = if matches!(
            &draft,
            PlannedMemberDraft::Hold {
                reason_code: "episode_not_found_for_title",
                ..
            }
        ) {
            match reconcile_unresolved_pack_member_from_expected_scope(
                title,
                &pack,
                source_path,
                &catalog_episodes,
                expected_episode_ids,
            ) {
                ScopedPackMemberReconciliation::Resolved(episode_id) => catalog_episodes
                    .iter()
                    .find(|episode| episode.id == episode_id)
                    .cloned()
                    .map(|episode| PlannedMemberDraft::Resolved(vec![episode]))
                    .unwrap_or(draft),
                ScopedPackMemberReconciliation::Ambiguous => PlannedMemberDraft::Hold {
                    episodes: Vec::new(),
                    reason_code: "ambiguous_pack_alternate_numbering",
                    message: "Automatic import found multiple scoped catalog episodes with this member's alternate episode number. Open Manual Import and assign the correct episode."
                        .to_string(),
                },
                ScopedPackMemberReconciliation::Unresolved => draft,
            }
        } else {
            draft
        };
        drafts.push(draft);
    }

    resolve_ambiguous_pack_numbering(&mut drafts, &catalog_episodes, expected_episode_ids);
    hold_duplicate_pack_episode_mappings(&mut drafts);

    let mut plan = EpisodePackImportPlan::default();
    for (source_path, draft) in video_files.iter().cloned().zip(drafts) {
        let disposition = finalize_pack_member_disposition(draft, &pack);
        plan.members.insert(source_path, disposition);
    }
    Ok(Some(plan))
}

enum ScopedPackMemberReconciliation {
    Resolved(String),
    Ambiguous,
    Unresolved,
}

/// Reconcile a pack member whose own numbering found nothing in the catalog,
/// corroborated in every lane by the pack's verified submission scope.
fn reconcile_unresolved_pack_member_from_expected_scope(
    title: &scryer_domain::Title,
    pack: &VerifiedEpisodePack,
    source_path: &Path,
    catalog: &[scryer_domain::Episode],
    expected_episode_ids: Option<&HashSet<String>>,
) -> ScopedPackMemberReconciliation {
    match reconcile_pack_member_from_scene_numbering(
        title,
        pack,
        source_path,
        catalog,
        expected_episode_ids,
    ) {
        ScopedPackMemberReconciliation::Unresolved => {
            reconcile_pack_member_from_absolute_numbering(
                pack,
                source_path,
                catalog,
                expected_episode_ids,
            )
        }
        reconciled => reconciled,
    }
}

/// Map a pack member that numbers itself absolutely onto the one catalog
/// episode carrying that absolute number.
///
/// Two shapes arrive here. A true absolute token (`- 231`) parses with no
/// season at all. A bare `E###` token parses as an inferred season 1, which is
/// a misparse — so that lane is taken only when the catalog holds no such
/// season-1 episode and the pack declares the season the absolute lands in.
/// Without that cross-check the remap would be a guess.
fn reconcile_pack_member_from_absolute_numbering(
    pack: &VerifiedEpisodePack,
    source_path: &Path,
    catalog: &[scryer_domain::Episode],
    expected_episode_ids: Option<&HashSet<String>>,
) -> ScopedPackMemberReconciliation {
    if pack.is_extras_release {
        return ScopedPackMemberReconciliation::Unresolved;
    }
    let parsed = parsed_release_from_file_stem(source_path);
    let Some(identity) = parsed.episode.as_ref() else {
        return ScopedPackMemberReconciliation::Unresolved;
    };
    if identity.air_date.is_some()
        || identity.special_kind.is_some()
        || !identity.special_absolute_episode_numbers.is_empty()
        || identity.full_season
        || identity.is_series_pack
        || identity.is_multi_season
        || identity.is_season_extra
        || identity.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
    {
        return ScopedPackMemberReconciliation::Unresolved;
    }
    let declared_seasons = pack
        .declared_seasons
        .as_ref()
        .filter(|seasons| !seasons.is_empty());

    let absolute_number = match identity.season {
        None => match parsed_absolute_numbers(identity).as_slice() {
            [number] => *number,
            _ => return ScopedPackMemberReconciliation::Unresolved,
        },
        Some(season) => {
            // An explicit absolute companion means the member is not a bare
            // `E###` misparse, and a declared season is the only thing that can
            // corroborate reading its episode number as an absolute.
            if identity.absolute_episode.is_some()
                || !identity.absolute_episode_numbers.is_empty()
                || declared_seasons.is_none()
            {
                return ScopedPackMemberReconciliation::Unresolved;
            }
            let [episode_number] = identity.episode_numbers.as_slice() else {
                return ScopedPackMemberReconciliation::Unresolved;
            };
            // A catalog episode at the parsed numbering means the parse was
            // right: an explicit S01E05 is never reread as absolute 5.
            if catalog.iter().any(|episode| {
                catalog_episode_season(episode) == Some(season)
                    && catalog_episode_number(episode) == Some(*episode_number)
            }) {
                return ScopedPackMemberReconciliation::Unresolved;
            }
            *episode_number
        }
    };

    let mut absolute_matches = catalog
        .iter()
        .filter(|episode| catalog_episode_absolute_number(episode) == Some(absolute_number));
    let Some(episode) = absolute_matches.next() else {
        return ScopedPackMemberReconciliation::Unresolved;
    };
    if absolute_matches.next().is_some() {
        return ScopedPackMemberReconciliation::Unresolved;
    }
    if episode.episode_type != scryer_domain::EpisodeType::Standard {
        return ScopedPackMemberReconciliation::Unresolved;
    }
    if declared_seasons.is_some_and(|seasons| {
        !catalog_episode_season(episode).is_some_and(|season| seasons.contains(&season))
    }) {
        return ScopedPackMemberReconciliation::Unresolved;
    }
    if expected_episode_ids.is_some_and(|expected| !expected.contains(&episode.id)) {
        return ScopedPackMemberReconciliation::Unresolved;
    }
    ScopedPackMemberReconciliation::Resolved(episode.id.clone())
}

/// Reconcile a pack member only when its local scene numbering is corroborated
/// by one catalog episode in the verified submission scope.
fn reconcile_pack_member_from_scene_numbering(
    title: &scryer_domain::Title,
    pack: &VerifiedEpisodePack,
    source_path: &Path,
    catalog: &[scryer_domain::Episode],
    expected_episode_ids: Option<&HashSet<String>>,
) -> ScopedPackMemberReconciliation {
    let Some(declared_seasons) = pack.declared_seasons.as_ref().filter(|seasons| !seasons.is_empty())
    else {
        return ScopedPackMemberReconciliation::Unresolved;
    };
    let Some(expected_episode_ids) = expected_episode_ids else {
        return ScopedPackMemberReconciliation::Unresolved;
    };
    if pack.is_extras_release {
        return ScopedPackMemberReconciliation::Unresolved;
    }

    let parsed = parsed_release_from_file_stem(source_path);
    let Some(identity) = parsed.episode.as_ref() else {
        return ScopedPackMemberReconciliation::Unresolved;
    };
    let [episode_number] = identity.episode_numbers.as_slice() else {
        return ScopedPackMemberReconciliation::Unresolved;
    };
    if identity.air_date.is_some()
        || identity.absolute_episode.is_some()
        || !identity.absolute_episode_numbers.is_empty()
        || !identity.special_absolute_episode_numbers.is_empty()
        || identity.special_kind.is_some()
        || identity.full_season
        || identity.is_series_pack
        || identity.is_multi_season
        || identity.is_season_extra
        || identity.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
        || !has_usable_release_title_signal(&parsed)
        || !parsed_title_matches_catalog_title(&parsed, title)
    {
        return ScopedPackMemberReconciliation::Unresolved;
    }

    let candidates: Vec<_> = catalog
        .iter()
        .filter(|episode| {
            expected_episode_ids.contains(&episode.id)
                && episode.episode_type == scryer_domain::EpisodeType::Standard
                && catalog_episode_number(episode) == Some(*episode_number)
                && catalog_episode_season(episode)
                    .is_some_and(|season| declared_seasons.contains(&season))
                && source_fuzzily_matches_catalog_episode_title(title, source_path, episode)
        })
        .cloned()
        .collect();
    match candidates.as_slice() {
        [episode] => ScopedPackMemberReconciliation::Resolved(episode.id.clone()),
        [] => ScopedPackMemberReconciliation::Unresolved,
        _ => ScopedPackMemberReconciliation::Ambiguous,
    }
}

fn plan_episode_pack_member(
    title: &scryer_domain::Title,
    source_root: &Path,
    source_path: &Path,
    catalog: &[scryer_domain::Episode],
) -> PlannedMemberDraft {
    let parsed_stem = parsed_pack_member_for_catalog(title, source_path, catalog);
    if has_disc_layout_ancestor(source_root, source_path) {
        return PlannedMemberDraft::Hold {
            episodes: Vec::new(),
            reason_code: "disc_layout_member",
            message: "Automatic import found a disc-layout pack member. Open Manual Import and assign the correct episode."
                .to_string(),
        };
    }

    if member_names_different_title(title, source_root, source_path, &parsed_stem) {
        return PlannedMemberDraft::Hold {
            episodes: Vec::new(),
            reason_code: "member_title_mismatch",
            message: "Automatic import found a pack member that names a different title. Open Manual Import and assign or discard it."
                .to_string(),
        };
    }

    if parsed_stem.episode.as_ref().is_some_and(|episode| {
        matches!(
            episode.special_kind,
            Some(
                crate::ParsedSpecialKind::Ncop
                    | crate::ParsedSpecialKind::Nced
                    | crate::ParsedSpecialKind::Extra
            )
        )
    }) {
        return PlannedMemberDraft::Ignore {
            reason_code: "auxiliary_video",
            message: "Recognized auxiliary pack video was intentionally ignored.".to_string(),
        };
    }

    let folder_season = match nearest_explicit_season_ancestor(source_root, source_path) {
        Ok(season) => season,
        Err(()) => {
            return PlannedMemberDraft::Hold {
                episodes: Vec::new(),
                reason_code: "conflicting_season_folders",
                message: "Automatic import found conflicting season folders for this pack member. Open Manual Import and assign the correct episode."
                    .to_string(),
            };
        }
    };

    let Some(identity) = parsed_stem
        .episode
        .clone()
        .or_else(|| parsed_release_from_file_stem(source_path).episode)
    else {
        return PlannedMemberDraft::Hold {
            episodes: Vec::new(),
            reason_code: "unparseable_pack_member",
            message: "Automatic import could not identify this pack member. Open Manual Import and assign the correct episode."
                .to_string(),
        };
    };

    plan_parsed_pack_identity(
        title.facet == scryer_domain::MediaFacet::Anime,
        &identity,
        folder_season,
        catalog,
    )
}

fn plan_parsed_pack_identity(
    is_anime: bool,
    identity: &crate::ParsedEpisodeMetadata,
    folder_season: Option<u32>,
    catalog: &[scryer_domain::Episode],
) -> PlannedMemberDraft {
    if let Some(air_date) = identity.air_date {
        let matches = catalog_episodes_for_air_date(catalog, air_date, identity.daily_part);
        return resolved_or_missing(matches, "daily episode");
    }

    if let Some(season) = identity.season {
        if folder_season.is_some_and(|folder| folder != season) {
            return PlannedMemberDraft::Hold {
                episodes: Vec::new(),
                reason_code: "member_season_conflict",
                message: "Automatic import found a filename season that conflicts with its season folder. Open Manual Import and assign the correct episode."
                    .to_string(),
            };
        }
        let standard =
            catalog_episodes_for_season_numbers(catalog, season, &identity.episode_numbers);
        let Some(standard) = standard else {
            return missing_catalog_member("season and episode");
        };
        let absolute_numbers = parsed_absolute_numbers(identity);
        if !absolute_numbers.is_empty() {
            let Some(absolute) = catalog_episodes_for_absolute_numbers(catalog, &absolute_numbers)
            else {
                return missing_catalog_member("absolute episode companion");
            };
            if episode_id_set(&standard) != episode_id_set(&absolute) {
                return PlannedMemberDraft::Hold {
                    episodes: standard,
                    reason_code: "member_identity_conflict",
                    message: "Automatic import found season/episode and absolute-number identities that resolve to different catalog episodes. Open Manual Import and choose the correct episode."
                        .to_string(),
                };
            }
        }
        return PlannedMemberDraft::Resolved(standard);
    }

    if !identity.special_absolute_episode_numbers.is_empty() {
        let matches = catalog_episodes_for_season_numbers(
            catalog,
            0,
            &identity.special_absolute_episode_numbers,
        );
        return matches
            .map(PlannedMemberDraft::Resolved)
            .unwrap_or_else(|| missing_catalog_member("special episode"));
    }

    let numbers = parsed_absolute_numbers(identity);
    if numbers.is_empty() {
        return PlannedMemberDraft::Hold {
            episodes: Vec::new(),
            reason_code: "unparseable_pack_member",
            message: "Automatic import could not identify this pack member. Open Manual Import and assign the correct episode."
                .to_string(),
        };
    }

    if let Some(season) = folder_season {
        let season_local = catalog_episodes_for_season_numbers(catalog, season, &numbers);
        let absolute = catalog_episodes_for_absolute_numbers(catalog, &numbers).and_then(|items| {
            items
                .iter()
                .all(|episode| catalog_episode_season(episode) == Some(season))
                .then_some(items)
        });
        if let (Some(local), Some(absolute)) = (&season_local, &absolute)
            && episode_id_set(local) == episode_id_set(absolute)
        {
            return PlannedMemberDraft::Resolved(local.clone());
        }
        return PlannedMemberDraft::AmbiguousNumbering {
            season,
            season_local,
            absolute,
        };
    }

    if is_anime {
        return catalog_episodes_for_absolute_numbers(catalog, &numbers)
            .map(PlannedMemberDraft::Resolved)
            .unwrap_or_else(|| missing_catalog_member("absolute episode"));
    }

    PlannedMemberDraft::Hold {
        episodes: Vec::new(),
        reason_code: "ambiguous_pack_numbering",
        message: "Automatic import found a bare episode number without a usable season folder. Open Manual Import and assign the correct episode."
            .to_string(),
    }
}

fn parsed_pack_member_for_catalog(
    title: &scryer_domain::Title,
    source_path: &Path,
    catalog: &[scryer_domain::Episode],
) -> crate::ParsedReleaseMetadata {
    let Some(stem) = source_video_stem(Some(source_path)) else {
        return parsed_release_from_file_stem(source_path);
    };
    let context = crate::build_release_parse_context_for_title(title, catalog, None);
    crate::parse_release_metadata_for_target(&stem, &context)
}

fn resolved_or_missing(
    episodes: Option<Vec<scryer_domain::Episode>>,
    label: &str,
) -> PlannedMemberDraft {
    episodes
        .map(PlannedMemberDraft::Resolved)
        .unwrap_or_else(|| missing_catalog_member(label))
}

fn missing_catalog_member(label: &str) -> PlannedMemberDraft {
    PlannedMemberDraft::Hold {
        episodes: Vec::new(),
        reason_code: "episode_not_found_for_title",
        message: format!(
            "Automatic import could not map the member's {label} identity to this title's catalog. Open Manual Import and assign the correct episode."
        ),
    }
}

fn parsed_absolute_numbers(identity: &crate::ParsedEpisodeMetadata) -> Vec<u32> {
    if !identity.absolute_episode_numbers.is_empty() {
        identity.absolute_episode_numbers.clone()
    } else if let Some(number) = identity.absolute_episode {
        vec![number]
    } else if identity.season.is_none() {
        identity.episode_numbers.clone()
    } else {
        Vec::new()
    }
}

fn catalog_episode_season(episode: &scryer_domain::Episode) -> Option<u32> {
    episode
        .season_number
        .as_deref()
        .map(str::trim)
        .and_then(|value| value.parse::<u32>().ok())
}

fn catalog_episode_number(episode: &scryer_domain::Episode) -> Option<u32> {
    episode
        .episode_number
        .as_deref()
        .map(str::trim)
        .and_then(|value| value.parse::<u32>().ok())
}

fn catalog_episode_absolute_number(episode: &scryer_domain::Episode) -> Option<u32> {
    episode
        .absolute_number
        .as_deref()
        .map(str::trim)
        .and_then(|value| value.parse::<u32>().ok())
}

fn catalog_episodes_for_season_numbers(
    catalog: &[scryer_domain::Episode],
    season: u32,
    numbers: &[u32],
) -> Option<Vec<scryer_domain::Episode>> {
    resolve_unique_catalog_numbers(numbers, |number| {
        catalog
            .iter()
            .filter(|episode| {
                catalog_episode_season(episode) == Some(season)
                    && catalog_episode_number(episode) == Some(number)
            })
            .cloned()
            .collect()
    })
}

fn catalog_episodes_for_absolute_numbers(
    catalog: &[scryer_domain::Episode],
    numbers: &[u32],
) -> Option<Vec<scryer_domain::Episode>> {
    resolve_unique_catalog_numbers(numbers, |number| {
        catalog
            .iter()
            .filter(|episode| catalog_episode_absolute_number(episode) == Some(number))
            .cloned()
            .collect()
    })
}

fn resolve_unique_catalog_numbers(
    numbers: &[u32],
    mut matches: impl FnMut(u32) -> Vec<scryer_domain::Episode>,
) -> Option<Vec<scryer_domain::Episode>> {
    if numbers.is_empty() {
        return None;
    }
    let mut resolved = Vec::with_capacity(numbers.len());
    let mut seen = HashSet::new();
    for number in numbers {
        let mut candidates = matches(*number);
        if candidates.len() != 1 {
            return None;
        }
        let episode = candidates.pop()?;
        if !seen.insert(episode.id.clone()) {
            return None;
        }
        resolved.push(episode);
    }
    Some(resolved)
}

fn catalog_episodes_for_air_date(
    catalog: &[scryer_domain::Episode],
    air_date: chrono::NaiveDate,
    daily_part: Option<u32>,
) -> Option<Vec<scryer_domain::Episode>> {
    let air_date = air_date.format("%Y-%m-%d").to_string();
    let mut matches: Vec<_> = catalog
        .iter()
        .filter(|episode| episode.air_date.as_deref() == Some(air_date.as_str()))
        .cloned()
        .collect();
    matches.sort_by_key(catalog_episode_number);
    if let Some(part) = daily_part {
        matches
            .into_iter()
            .nth(part.saturating_sub(1) as usize)
            .map(|episode| vec![episode])
    } else {
        (matches.len() == 1).then_some(matches)
    }
}

fn episode_id_set(episodes: &[scryer_domain::Episode]) -> HashSet<String> {
    episodes.iter().map(|episode| episode.id.clone()).collect()
}

fn resolve_ambiguous_pack_numbering(
    drafts: &mut [PlannedMemberDraft],
    catalog: &[scryer_domain::Episode],
    expected_episode_ids: Option<&HashSet<String>>,
) {
    let mut seasons = HashSet::new();
    for draft in drafts.iter() {
        if let PlannedMemberDraft::AmbiguousNumbering { season, .. } = draft {
            seasons.insert(*season);
        }
    }

    for season in seasons {
        let indexes: Vec<_> = drafts
            .iter()
            .enumerate()
            .filter_map(|(index, draft)| match draft {
                PlannedMemberDraft::AmbiguousNumbering {
                    season: member_season,
                    ..
                } if *member_season == season => Some(index),
                _ => None,
            })
            .collect();
        let fixed_ids = resolved_episode_ids_for_season(drafts, season);
        let local = numbering_candidate(drafts, &indexes, true, &fixed_ids);
        let absolute = numbering_candidate(drafts, &indexes, false, &fixed_ids);

        let selected_local = match (&local, &absolute) {
            (Some(_), None) => Some(true),
            (None, Some(_)) => Some(false),
            (Some(local), Some(absolute)) if local == absolute => Some(true),
            (Some(local), Some(absolute)) => {
                let local_proven = numbering_candidate_is_proven(
                    local,
                    &fixed_ids,
                    season,
                    catalog,
                    expected_episode_ids,
                );
                let absolute_proven = numbering_candidate_is_proven(
                    absolute,
                    &fixed_ids,
                    season,
                    catalog,
                    expected_episode_ids,
                );
                match (local_proven, absolute_proven) {
                    (true, false) => Some(true),
                    (false, true) => Some(false),
                    _ => None,
                }
            }
            (None, None) => None,
        };

        for index in indexes {
            let replacement = match (&drafts[index], selected_local) {
                (
                    PlannedMemberDraft::AmbiguousNumbering {
                        season_local,
                        ..
                    },
                    Some(true),
                ) => season_local.clone().map(PlannedMemberDraft::Resolved),
                (
                    PlannedMemberDraft::AmbiguousNumbering {
                        season_local: _,
                        absolute,
                        ..
                    },
                    Some(false),
                ) => absolute.clone().map(PlannedMemberDraft::Resolved),
                _ => None,
            }
            .unwrap_or_else(|| PlannedMemberDraft::Hold {
                episodes: Vec::new(),
                reason_code: "ambiguous_pack_numbering",
                message: "Automatic import could not prove whether this season folder uses absolute or season-local numbering. Open Manual Import and assign the correct episode."
                    .to_string(),
            });
            drafts[index] = replacement;
        }
    }
}

fn resolved_episode_ids_for_season(drafts: &[PlannedMemberDraft], season: u32) -> HashSet<String> {
    drafts
        .iter()
        .filter_map(|draft| match draft {
            PlannedMemberDraft::Resolved(episodes) => Some(episodes),
            _ => None,
        })
        .flat_map(|episodes| episodes.iter())
        .filter(|episode| catalog_episode_season(episode) == Some(season))
        .map(|episode| episode.id.clone())
        .collect()
}

fn numbering_candidate(
    drafts: &[PlannedMemberDraft],
    indexes: &[usize],
    season_local: bool,
    fixed_ids: &HashSet<String>,
) -> Option<HashSet<String>> {
    let mut ids = HashSet::new();
    for index in indexes {
        let PlannedMemberDraft::AmbiguousNumbering {
            season_local: local,
            absolute,
            ..
        } = &drafts[*index]
        else {
            return None;
        };
        let episodes = if season_local { local } else { absolute };
        let episodes = episodes.as_ref()?;
        for episode in episodes {
            if fixed_ids.contains(&episode.id) || !ids.insert(episode.id.clone()) {
                return None;
            }
        }
    }
    Some(ids)
}

fn numbering_candidate_is_proven(
    candidate_ids: &HashSet<String>,
    fixed_ids: &HashSet<String>,
    season: u32,
    catalog: &[scryer_domain::Episode],
    expected_episode_ids: Option<&HashSet<String>>,
) -> bool {
    let mut resolved = fixed_ids.clone();
    resolved.extend(candidate_ids.iter().cloned());

    let catalog_standard: HashSet<_> = catalog
        .iter()
        .filter(|episode| episode.episode_type == scryer_domain::EpisodeType::Standard)
        .filter(|episode| catalog_episode_season(episode) == Some(season))
        .map(|episode| episode.id.clone())
        .collect();
    if !catalog_standard.is_empty() && resolved == catalog_standard {
        return true;
    }

    let Some(expected) = expected_episode_ids else {
        return false;
    };
    let catalog_ids_for_season: HashSet<_> = catalog
        .iter()
        .filter(|episode| catalog_episode_season(episode) == Some(season))
        .map(|episode| episode.id.clone())
        .collect();
    let expected_for_season: HashSet<_> = expected
        .intersection(&catalog_ids_for_season)
        .cloned()
        .collect();
    !expected_for_season.is_empty()
        && resolved
            .intersection(expected)
            .cloned()
            .collect::<HashSet<_>>()
            == expected_for_season
}

fn hold_duplicate_pack_episode_mappings(drafts: &mut [PlannedMemberDraft]) {
    let mut owners = HashMap::<String, Vec<usize>>::new();
    for (index, draft) in drafts.iter().enumerate() {
        if let PlannedMemberDraft::Resolved(episodes) = draft {
            for episode in episodes {
                owners.entry(episode.id.clone()).or_default().push(index);
            }
        }
    }
    let collisions: HashSet<_> = owners
        .into_values()
        .filter(|indexes| indexes.len() > 1)
        .flatten()
        .collect();
    for index in collisions {
        let episodes = match &drafts[index] {
            PlannedMemberDraft::Resolved(episodes) => episodes.clone(),
            _ => continue,
        };
        drafts[index] = PlannedMemberDraft::Hold {
            episodes,
            reason_code: "duplicate_pack_episode_mapping",
            message: "Automatic import found multiple pack files that resolve to the same catalog episode. Open Manual Import and choose the correct file."
                .to_string(),
        };
    }
}

fn finalize_pack_member_disposition(
    draft: PlannedMemberDraft,
    pack: &VerifiedEpisodePack,
) -> PlannedEpisodeMemberDisposition {
    match draft {
        PlannedMemberDraft::Resolved(episodes) => {
            let standard_outside_declared = episodes.iter().any(|episode| {
                episode.episode_type == scryer_domain::EpisodeType::Standard
                    && !pack.vouches_for(episode)
            });
            if standard_outside_declared {
                return PlannedEpisodeMemberDisposition::Hold {
                    episodes,
                    reason_code: "episode_outside_declared_pack_seasons",
                    message: "Automatic import resolved an episode outside the seasons declared by this pack. Open Manual Import and verify the member."
                        .to_string(),
                };
            }
            // Monitoring decides what Scryer goes and gets, never what it
            // keeps: like Sonarr, a resolved pack member imports even when
            // its episodes are unmonitored — the bytes are already here.
            PlannedEpisodeMemberDisposition::Import { episodes }
        }
        PlannedMemberDraft::Ignore {
            reason_code,
            message,
        } => PlannedEpisodeMemberDisposition::Ignore {
            episodes: Vec::new(),
            reason_code,
            message,
        },
        PlannedMemberDraft::Hold {
            episodes,
            reason_code,
            message,
        } => PlannedEpisodeMemberDisposition::Hold {
            episodes,
            reason_code,
            message,
        },
        PlannedMemberDraft::AmbiguousNumbering { .. } => {
            PlannedEpisodeMemberDisposition::Hold {
                episodes: Vec::new(),
                reason_code: "ambiguous_pack_numbering",
                message: "Automatic import could not prove this pack member's numbering convention. Open Manual Import and assign the correct episode."
                    .to_string(),
            }
        }
    }
}

fn nearest_explicit_season_ancestor(
    source_root: &Path,
    source_path: &Path,
) -> Result<Option<u32>, ()> {
    let mut season = None;
    let mut current = source_path.parent();
    while let Some(parent) = current {
        if parent == source_root {
            break;
        }
        if !parent.starts_with(source_root) {
            break;
        }
        if let Some(candidate) = parent.file_name().and_then(|name| name.to_str())
            && let Some(candidate) = single_season_folder_number(candidate)
        {
            if season.is_some_and(|existing| existing != candidate) {
                return Err(());
            }
            season = Some(candidate);
        }
        current = parent.parent();
    }
    Ok(season)
}

fn single_season_folder_number(folder_name: &str) -> Option<u32> {
    let episode = parse_release_metadata(folder_name).episode?;
    let season = episode.season?;
    ((episode.season_numbers.is_empty() || episode.season_numbers == [season])
        && episode.episode_numbers.is_empty()
        && episode.absolute_episode.is_none()
        && episode.absolute_episode_numbers.is_empty()
        && episode.special_absolute_episode_numbers.is_empty()
        && !episode.is_multi_season
        && !episode.is_season_extra
        && episode.special_kind.is_none())
    .then_some(season)
}

fn member_names_different_title(
    title: &scryer_domain::Title,
    source_root: &Path,
    source_path: &Path,
    parsed_stem: &crate::ParsedReleaseMetadata,
) -> bool {
    if parsed_stem.episode.is_some()
        && has_usable_release_title_signal(parsed_stem)
        && !parsed_title_matches_catalog_title(parsed_stem, title)
    {
        return true;
    }

    let mut child = source_path;
    let mut current = source_path.parent();
    while let Some(parent) = current {
        if parent == source_root || !parent.starts_with(source_root) {
            break;
        }
        if let Some(name) = parent.file_name().and_then(|name| name.to_str())
            && folder_is_title_container(parent, child)
        {
            let parsed_parent = normalize_release_title_signal(parse_release_metadata(name));
            let word_count = parsed_parent
                .normalized_title
                .split_whitespace()
                .filter(|word| word.chars().any(|ch| ch.is_alphabetic()))
                .count();
            if word_count >= 2
                && has_usable_release_title_signal(&parsed_parent)
                && !parsed_title_matches_catalog_title(&parsed_parent, title)
            {
                return true;
            }
        }
        child = parent;
        current = parent.parent();
    }
    false
}

fn has_disc_layout_ancestor(source_root: &Path, source_path: &Path) -> bool {
    let mut current = source_path.parent();
    while let Some(parent) = current {
        if !parent.starts_with(source_root) {
            break;
        }
        if parent
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_disc_layout_directory)
        {
            return true;
        }
        if parent == source_root {
            break;
        }
        current = parent.parent();
    }
    false
}

fn folder_is_title_container(parent: &Path, child: &Path) -> bool {
    child.parent() == Some(parent)
        && child
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(single_season_folder_number)
            .is_some()
}

fn is_disc_layout_directory(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_uppercase().as_str(),
        "BDMV" | "CERTIFICATE" | "HVDVD_TS" | "VIDEO_TS"
    )
}

#[cfg(test)]
mod series_plan_tests {
    use super::*;
    use chrono::Utc;

    fn catalog_episode(id: &str, episode: u32, absolute: u32) -> scryer_domain::Episode {
        scryer_domain::Episode {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            collection_id: Some("season-1".to_string()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some(episode.to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some(format!("S01E{episode:02}")),
            title: Some(format!("Episode {episode}")),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some(absolute.to_string()),
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn hybrid_identity(absolute: u32) -> crate::ParsedEpisodeMetadata {
        crate::ParsedEpisodeMetadata {
            season: Some(1),
            season_numbers: vec![1],
            episode_numbers: vec![1],
            absolute_episode: Some(absolute),
            absolute_episode_numbers: vec![absolute],
            ..Default::default()
        }
    }

    #[test]
    fn agreeing_hybrid_identity_resolves_one_catalog_episode() {
        let catalog = vec![catalog_episode("ep-1", 1, 1), catalog_episode("ep-2", 2, 2)];
        assert!(matches!(
            plan_parsed_pack_identity(false, &hybrid_identity(1), None, &catalog),
            PlannedMemberDraft::Resolved(ref episodes) if episodes.len() == 1 && episodes[0].id == "ep-1"
        ));
    }

    #[test]
    fn conflicting_hybrid_identity_is_held() {
        let catalog = vec![catalog_episode("ep-1", 1, 1), catalog_episode("ep-2", 2, 2)];
        assert!(matches!(
            plan_parsed_pack_identity(false, &hybrid_identity(2), None, &catalog),
            PlannedMemberDraft::Hold {
                reason_code: "member_identity_conflict",
                ..
            }
        ));
    }

    #[test]
    fn nearest_season_ancestor_accepts_decorated_single_season_folder() {
        let root = Path::new("/census");
        for (folder, expected) in [
            ("Season 03 Archive", 3),
            ("S02 (BD)", 2),
            ("Season 4 - WEB", 4),
        ] {
            let member = root.join("Synthetic Title").join(folder).join("member.mkv");
            assert_eq!(
                nearest_explicit_season_ancestor(root, &member),
                Ok(Some(expected)),
                "{folder}"
            );
        }
    }

    #[test]
    fn season_folder_parser_rejects_episode_multi_season_and_extras_shapes() {
        for folder in ["S01E02", "S01-S02", "Season 01 Extras"] {
            assert_eq!(single_season_folder_number(folder), None, "{folder}");
        }
    }

    #[test]
    fn only_a_season_child_makes_a_folder_title_bearing() {
        let title_folder = Path::new("/census/Census Title");
        let season_folder = title_folder.join("Season 01 Archive");
        let commentary_folder = season_folder.join("Commentary Notes");

        assert!(folder_is_title_container(title_folder, &season_folder));
        assert!(!folder_is_title_container(
            &season_folder,
            &commentary_folder
        ));
    }

    #[test]
    fn date_only_pack_identity_requires_one_catalog_match_or_explicit_part() {
        let mut first = catalog_episode("ep-1", 1, 1);
        first.air_date = Some("2040-01-02".to_string());
        let mut second = catalog_episode("ep-2", 2, 2);
        second.air_date = Some("2040-01-02".to_string());
        let catalog = vec![first, second];
        let date = chrono::NaiveDate::from_ymd_opt(2040, 1, 2).expect("valid census date");
        let date_only = crate::ParsedEpisodeMetadata {
            air_date: Some(date),
            ..Default::default()
        };
        let part_two = crate::ParsedEpisodeMetadata {
            air_date: Some(date),
            daily_part: Some(2),
            ..Default::default()
        };

        assert!(matches!(
            plan_parsed_pack_identity(false, &date_only, None, &catalog),
            PlannedMemberDraft::Hold {
                reason_code: "episode_not_found_for_title",
                ..
            }
        ));
        assert!(matches!(
            plan_parsed_pack_identity(false, &part_two, None, &catalog),
            PlannedMemberDraft::Resolved(ref episodes) if episodes[0].id == "ep-2"
        ));
    }

    fn catalog_episode_in_season(
        id: &str,
        season: u32,
        episode: u32,
        absolute: u32,
    ) -> scryer_domain::Episode {
        let mut catalog = catalog_episode(id, episode, absolute);
        catalog.collection_id = Some(format!("season-{season}"));
        catalog.season_number = Some(season.to_string());
        catalog.episode_label = Some(format!("S{season:02}E{episode:02}"));
        catalog
    }

    fn declared_season_pack(season: Option<u32>) -> VerifiedEpisodePack {
        VerifiedEpisodePack {
            declared_seasons: season.map(|season| HashSet::from([season])),
            is_extras_release: false,
        }
    }

    fn pack_member(file_name: &str) -> PathBuf {
        Path::new("/downloads/Quiet Meridian Season 13").join(file_name)
    }

    fn reconciled_absolute_member(
        pack: &VerifiedEpisodePack,
        file_name: &str,
        catalog: &[scryer_domain::Episode],
        expected_episode_ids: Option<&HashSet<String>>,
    ) -> ScopedPackMemberReconciliation {
        reconcile_pack_member_from_absolute_numbering(
            pack,
            &pack_member(file_name),
            catalog,
            expected_episode_ids,
        )
    }

    /// `[GroupTag] Quiet Meridian - 231 ...` parses as a bare absolute with no
    /// season of its own; the declared season is what corroborates the map.
    #[test]
    fn absolute_numbered_pack_member_maps_through_the_catalog_absolute_number() {
        let catalog = vec![
            catalog_episode_in_season("ep-s13e01", 13, 1, 230),
            catalog_episode_in_season("ep-s13e02", 13, 2, 231),
        ];
        let expected = episode_id_set(&catalog);

        assert!(matches!(
            reconciled_absolute_member(
                &declared_season_pack(Some(13)),
                "[GroupTag] Quiet Meridian - 231 [BD][h.264][1080p][FLAC] [ABB6F939].mkv",
                &catalog,
                Some(&expected),
            ),
            ScopedPackMemberReconciliation::Resolved(ref episode_id) if episode_id == "ep-s13e02"
        ));
    }

    /// A bare `E###` token is only readable as an inferred season 1, which the
    /// catalog contradicts; the declared season resolves the misparse.
    #[test]
    fn bare_episode_token_pack_member_remaps_onto_the_declared_season() {
        let catalog = vec![
            catalog_episode_in_season("ep-s13e01", 13, 1, 230),
            catalog_episode_in_season("ep-s13e02", 13, 2, 231),
        ];
        let expected = episode_id_set(&catalog);

        assert!(matches!(
            reconciled_absolute_member(
                &declared_season_pack(Some(13)),
                "E230.mkv",
                &catalog,
                Some(&expected),
            ),
            ScopedPackMemberReconciliation::Resolved(ref episode_id) if episode_id == "ep-s13e01"
        ));
    }

    #[test]
    fn a_catalogued_season_episode_is_never_reread_as_an_absolute() {
        let catalog = vec![
            catalog_episode_in_season("ep-s01e05", 1, 5, 5),
            catalog_episode_in_season("ep-s13e01", 13, 1, 230),
        ];
        let expected = episode_id_set(&catalog);

        assert!(matches!(
            reconciled_absolute_member(
                &declared_season_pack(Some(1)),
                "Quiet.Meridian.S01E05.1080p.WEB-DL.x264-GroupTag.mkv",
                &catalog,
                Some(&expected),
            ),
            ScopedPackMemberReconciliation::Unresolved
        ));
    }

    #[test]
    fn an_absolute_outside_the_declared_season_is_left_unresolved() {
        let catalog = vec![catalog_episode_in_season("ep-s12e24", 12, 24, 231)];
        let expected = episode_id_set(&catalog);

        assert!(matches!(
            reconciled_absolute_member(
                &declared_season_pack(Some(13)),
                "[GroupTag] Quiet Meridian - 231 [BD][h.264][1080p][FLAC] [ABB6F939].mkv",
                &catalog,
                Some(&expected),
            ),
            ScopedPackMemberReconciliation::Unresolved
        ));
    }

    #[test]
    fn a_shared_absolute_number_is_left_unresolved() {
        let catalog = vec![
            catalog_episode_in_season("ep-s13e01", 13, 1, 230),
            catalog_episode_in_season("ep-s13e02", 13, 2, 230),
        ];
        let expected = episode_id_set(&catalog);

        assert!(matches!(
            reconciled_absolute_member(
                &declared_season_pack(Some(13)),
                "E230.mkv",
                &catalog,
                Some(&expected),
            ),
            ScopedPackMemberReconciliation::Unresolved
        ));
    }

    /// Without a declared season nothing corroborates rereading `E230` as an
    /// absolute, so the misparse lane stays shut.
    #[test]
    fn a_bare_episode_token_needs_a_declared_season() {
        let catalog = vec![catalog_episode_in_season("ep-s13e01", 13, 1, 230)];

        assert!(matches!(
            reconciled_absolute_member(&declared_season_pack(None), "E230.mkv", &catalog, None),
            ScopedPackMemberReconciliation::Unresolved
        ));
        assert!(matches!(
            reconciled_absolute_member(
                &declared_season_pack(None),
                "[GroupTag] Quiet Meridian - 230 [BD][h.264][1080p][FLAC] [ABB6F939].mkv",
                &catalog,
                None,
            ),
            ScopedPackMemberReconciliation::Resolved(ref episode_id) if episode_id == "ep-s13e01"
        ));
    }
}
