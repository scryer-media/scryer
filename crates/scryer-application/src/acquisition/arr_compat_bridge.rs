use super::*;
use scryer_domain::LibraryPermission;

impl AppUseCase {
    /// Compatibility-only intake: resolve one destination before delegating to
    /// the existing acquisition evaluator and canonical download submission.
    pub async fn submit_arr_compat_release(
        &self,
        actor: &User,
        input: ExternalReleaseInput,
        facets: &[MediaFacet],
        library_id: Option<&str>,
    ) -> AppResult<ExternalReleaseOutcome> {
        let libraries: HashSet<_> = self
            .list_libraries_for_permission(actor, None, LibraryPermission::ManageTitles)
            .await?
            .into_iter()
            .filter(|library| facets.contains(&library.facet))
            .filter(|library| library_id.is_none_or(|id| id == library.id))
            .map(|library| library.id)
            .collect();
        if library_id.is_some() && libraries.is_empty() {
            return Err(AppError::Validation(
                "The selected library is unavailable or incompatible with this endpoint".into(),
            ));
        }
        let candidate = input.into_candidate()?;
        let matching_libraries = libraries.clone();
        let matching_facets = facets.to_vec();
        let bank = TitleContextBank::new(
            crate::import_title_resolution::MonitoredTitleMatcher::new(
                self.services.catalog.titles.clone(),
            ),
            move |title| {
                matching_libraries.contains(&title.library_id)
                    && matching_facets.contains(&title.facet)
            },
        );
        let matches = compat_matching_titles(&candidate, &bank).await?;
        let title_id = match matches.as_slice() {
            [] => {
                return Ok(ExternalReleaseOutcome::rejected(
                    "No unambiguous authorized monitored title matches this release",
                ));
            }
            [title_id] => title_id,
            _ => {
                return Ok(ExternalReleaseOutcome::rejected(if library_id.is_some() {
                    "This release matches multiple catalog entries within the selected library. Resolve the duplicate or conflicting entries before retrying."
                } else {
                    "This release matches multiple catalog entries. Configure a library-specific endpoint."
                }));
            }
        };
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("matched title no longer exists".into()))?;
        if !libraries.contains(&title.library_id) || !facets.contains(&title.facet) {
            return Ok(ExternalReleaseOutcome::rejected(
                "The matched title moved outside the selected scope. Retry the announcement.",
            ));
        }
        self.require_library_permission(actor, &title.library_id, LibraryPermission::ManageTitles)
            .await?;
        self.evaluate_external_candidate(&title, candidate, Some(actor))
            .await
    }
}

/// Adapter admission uses existing indexed discovery and identity proofs, but
/// never chooses a destination by quality, library order, or title-ID ranking.
async fn compat_matching_titles(
    release: &IndexerSearchResult,
    bank: &TitleContextBank,
) -> AppResult<Vec<String>> {
    let mut probes = Vec::<String>::new();
    for key in context_free_identity_anchor_keys(&release.title) {
        let tokens: Vec<_> = key.split_whitespace().collect();
        for end in 1..=tokens.len() {
            let probe = tokens[..end].join(" ");
            if !probes.contains(&probe) {
                probes.push(probe);
            }
        }
    }
    let mut discovered = Vec::new();
    let mut seen = HashSet::new();
    let mut admit = |titles: Vec<Title>| {
        for title in titles {
            if title.monitored && (bank.in_scope)(&title) && seen.insert(title.id.clone()) {
                discovered.push(title);
            }
        }
    };
    admit(bank.matcher.titles_naming_key_shapes(&probes).await?);
    for (source, id) in [
        ("tvdb", release.response_attributes.tvdb_id.as_deref()),
        ("tmdb", release.response_attributes.tmdb_id.as_deref()),
        ("imdb", release.response_attributes.imdb_id.as_deref()),
    ] {
        if let Some(id) = id {
            admit(
                bank.matcher
                    .monitored_titles_by_external_id(source, id)
                    .await?,
            );
        }
    }
    let (anchors, _) = crate::title_matching::relaxed::neutral_spelling_forms(&release.title);
    let spelling = bank.matcher.spelling_candidates(&anchors, None).await?;
    let ids: Vec<_> = spelling.candidates(&anchors, None).into_iter().collect();
    admit(bank.matcher.titles_by_ids(&ids).await?);
    let mut matches = Vec::new();
    for title in discovered {
        let candidate = bank.candidate(&title, &spelling).await?;
        let parsed =
            parse_release_metadata_for_target(&release.title, &candidate.evidence.parse_context);
        if let (Some(actual), Some(expected)) = (parsed.year, title.year)
            && actual != expected
        {
            continue;
        }
        let asserted = crate::acquisition_release_search::strict_external_id_agreement(
            &release.response_attributes,
            candidate.info.tvdb_id.as_deref(),
            candidate.info.tmdb_id.as_deref(),
            candidate.info.imdb_id.as_deref(),
        );
        if asserted == Some(false) {
            continue;
        }
        let Some(evidence_match) =
            crate::acquisition_release_search::match_parsed_release_with_asserted_ids(
                &parsed,
                &candidate.evidence,
                asserted,
            )
        else {
            continue;
        };
        let agreement = external_id_agreement(
            &release.response_attributes,
            candidate.info.tvdb_id.as_deref(),
            candidate.info.tmdb_id.as_deref(),
            candidate.info.imdb_id.as_deref(),
        );
        if evidence_match.requires_external_id && agreement != Some(true) {
            continue;
        }
        if candidate.evidence.ambiguity.requires_disambiguator()
            && !candidate_presents_identity_disambiguator(
                &candidate.evidence,
                &CandidateTitleMatch {
                    evidence_match: Some(evidence_match),
                },
                agreement,
            )
        {
            continue;
        }
        matches.push(title.id);
        if matches.len() == 2 {
            break;
        }
    }
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(id: &str, library: &str, year: i32) -> Title {
        Title {
            id: id.into(),
            name: "Tide Chart".into(),
            facet: MediaFacet::Movie,
            library_id: library.into(),
            root_folder_id: "fixture-root".into(),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: Some(year),
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    async fn resolve(
        titles: Vec<Title>,
        library: Option<&str>,
        name: &str,
        tmdb: Option<&str>,
    ) -> Vec<String> {
        let library = library.map(str::to_owned);
        let bank = TitleContextBank::new(
            crate::import_title_resolution::MonitoredTitleMatcher::over_titles(titles),
            move |title| library.as_ref().is_none_or(|id| id == &title.library_id),
        );
        let release = ExternalReleaseInput {
            name: name.into(),
            download_url: "https://releases.invalid/fixture.nzb".into(),
            protocol: ExternalReleaseProtocol::Usenet,
            source: "Fixture".into(),
            size_bytes: None,
            published_at: None,
            external_ids: IndexerResponseAttributes {
                tmdb_id: tmdb.map(str::to_owned),
                ..Default::default()
            },
            flags: vec![],
        }
        .into_candidate()
        .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            compat_matching_titles(&release, &bank),
        )
        .await
        .expect("bounded adapter matching")
        .unwrap()
    }

    #[tokio::test]
    async fn arr_compat_copies_remain_ambiguous_regardless_of_order_or_shared_ids() {
        for same_library in [false, true] {
            for reverse in [false, true] {
                let mut first = title("z-copy", "primary", 2024);
                let mut second = title(
                    "a-copy",
                    if same_library { "primary" } else { "secondary" },
                    2024,
                );
                first.external_ids = vec![scryer_domain::ExternalId::new("tmdb", "777001")];
                second.external_ids = first.external_ids.clone();
                let titles = if reverse {
                    vec![second, first]
                } else {
                    vec![first, second]
                };
                for asserted in [None, Some("777001")] {
                    assert_eq!(
                        resolve(
                            titles.clone(),
                            None,
                            "Tide.Chart.2024.1080p.WEB-DL",
                            asserted
                        )
                        .await
                        .len(),
                        2
                    );
                    let scoped = resolve(
                        titles.clone(),
                        Some("primary"),
                        "Tide.Chart.2024.1080p.WEB-DL",
                        asserted,
                    )
                    .await;
                    assert_eq!(scoped.len(), if same_library { 2 } else { 1 });
                    if !same_library {
                        assert_eq!(scoped, ["z-copy"]);
                    }
                }
                assert!(
                    resolve(
                        titles,
                        Some("primary"),
                        "Tide.Chart.2024.1080p.WEB-DL",
                        Some("777002")
                    )
                    .await
                    .is_empty()
                );
            }
        }
    }

    #[tokio::test]
    async fn arr_compat_scope_does_not_make_vague_remakes_identifiable() {
        let titles = vec![
            title("modern", "primary", 2024),
            title("original", "secondary", 1999),
        ];
        assert!(
            resolve(
                titles.clone(),
                Some("primary"),
                "Tide.Chart.1080p.WEB-DL",
                None
            )
            .await
            .is_empty()
        );
        assert!(
            resolve(
                titles.clone(),
                Some("primary"),
                "Tide.Chart.1999.1080p.WEB-DL",
                None
            )
            .await
            .is_empty()
        );
        assert_eq!(
            resolve(
                titles,
                Some("primary"),
                "Tide.Chart.2024.1080p.WEB-DL",
                None
            )
            .await,
            ["modern"]
        );
    }

    #[tokio::test]
    async fn arr_compat_ignores_unmonitored_and_unrelated_entries() {
        let first = title("primary", "primary", 2024);
        let mut second = title("secondary", "secondary", 2024);
        second.monitored = false;
        let mut unrelated = title("unrelated", "third", 2024);
        unrelated.name = "Different Film".into();
        assert_eq!(
            resolve(
                vec![first, second, unrelated],
                None,
                "Tide.Chart.2024.1080p.WEB-DL",
                None
            )
            .await,
            ["primary"]
        );
    }
}
