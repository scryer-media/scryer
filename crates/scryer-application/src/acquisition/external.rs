use super::*;

#[path = "arr_compat_bridge.rs"]
mod arr_compat_bridge;

pub(super) fn pending_explanation(candidate: &IndexerSearchResult) -> Option<String> {
    let explanation = serialize_decision_explanation(candidate);
    let Some(announcement) = candidate.extra.get("external_announcement") else {
        return explanation;
    };
    let mut value: serde_json::Value = explanation
        .as_deref()
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    value["external_announcement"] = announcement.clone();
    Some(value.to_string())
}

pub(super) fn restore_announcement(
    pending: &PendingRelease,
    extra: &mut HashMap<String, serde_json::Value>,
) -> IndexerResponseAttributes {
    let announcement = pending
        .scoring_log_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value.get("external_announcement").cloned());
    let Some(announcement) = announcement else {
        return IndexerResponseAttributes::default();
    };
    if let Some(flags) = announcement
        .get("flags")
        .and_then(serde_json::Value::as_array)
    {
        for flag in flags.iter().filter_map(serde_json::Value::as_str) {
            extra.insert(flag.to_owned(), serde_json::Value::Bool(true));
        }
    }
    let id = |key| {
        announcement
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let attributes = IndexerResponseAttributes {
        tvdb_id: id("tvdb_id"),
        tmdb_id: id("tmdb_id"),
        imdb_id: id("imdb_id"),
        categories: vec![],
    };
    extra.insert("external_announcement".into(), announcement);
    attributes
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalReleaseProtocol {
    Usenet,
    Torrent,
}

#[derive(Clone, Debug)]
pub struct ExternalReleaseInput {
    pub name: String,
    pub download_url: String,
    pub protocol: ExternalReleaseProtocol,
    pub source: String,
    pub size_bytes: Option<i64>,
    pub published_at: Option<DateTime<Utc>>,
    pub external_ids: IndexerResponseAttributes,
    pub flags: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalReleaseStatus {
    Queued,
    Held,
    Rejected,
    Duplicate,
}

#[derive(Clone, Debug)]
pub struct ExternalReleaseOutcome {
    pub status: ExternalReleaseStatus,
    pub reasons: Vec<String>,
    pub title_id: Option<String>,
    pub download_id: Option<String>,
    pub pending_id: Option<String>,
}

impl ExternalReleaseOutcome {
    fn rejected(reason: impl Into<String>) -> Self {
        Self {
            status: ExternalReleaseStatus::Rejected,
            reasons: vec![reason.into()],
            title_id: None,
            download_id: None,
            pending_id: None,
        }
    }
}

impl ExternalReleaseInput {
    fn into_candidate(self) -> AppResult<IndexerSearchResult> {
        let name = self.name.trim();
        let source = self.source.trim();
        let download_url = self.download_url.trim();
        if download_url.len() > 16_384 || self.flags.len() > 32 {
            return Err(AppError::Validation(
                "release URL or flag list exceeds the input limit".into(),
            ));
        }
        if name.is_empty() || name.len() > 2048 || source.is_empty() || source.len() > 256 {
            return Err(AppError::Validation(
                "release name and source are required and must be bounded".into(),
            ));
        }
        if self.size_bytes.is_some_and(|size| size < 0) {
            return Err(AppError::Validation(
                "release size must not be negative".into(),
            ));
        }
        let magnet_hash = crate::extract_magnet_info_hash(download_url);
        let source_kind = if magnet_hash.is_some() {
            if self.protocol != ExternalReleaseProtocol::Torrent {
                return Err(AppError::Validation(
                    "a magnet requires the torrent protocol".into(),
                ));
            }
            DownloadSourceKind::MagnetUri
        } else {
            let url = url::Url::parse(download_url)
                .map_err(|_| AppError::Validation("release download URL is invalid".into()))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(AppError::Validation(
                    "release URL must use HTTP, HTTPS, or a valid magnet".into(),
                ));
            }
            match self.protocol {
                ExternalReleaseProtocol::Usenet => DownloadSourceKind::NzbUrl,
                ExternalReleaseProtocol::Torrent => DownloadSourceKind::TorrentFile,
            }
        };
        let mut extra = HashMap::new();
        extra.insert(
            "external_announcement".into(),
            serde_json::json!({
                "flags": self.flags,
                "tvdb_id": self.external_ids.tvdb_id,
                "tmdb_id": self.external_ids.tmdb_id,
                "imdb_id": self.external_ids.imdb_id,
            }),
        );
        if let Some(hash) = magnet_hash {
            extra.insert("info_hash".into(), serde_json::Value::String(hash));
        }
        for flag in self.flags {
            if !matches!(
                flag.as_str(),
                "freeleech"
                    | "halfleech"
                    | "double_upload"
                    | "internal"
                    | "scene"
                    | "freeleech_75"
                    | "freeleech_25"
                    | "nuked"
                    | "subtitles"
                    | "golden"
                    | "approved"
            ) {
                return Err(AppError::Validation(format!(
                    "unsupported release flag: {flag}"
                )));
            }
            extra.insert(flag, serde_json::Value::Bool(true));
        }
        Ok(IndexerSearchResult {
            indexer_id: None,
            source: source.to_owned(),
            title: name.to_owned(),
            link: None,
            download_url: Some(download_url.to_owned()),
            source_kind: Some(source_kind),
            size_bytes: self.size_bytes,
            published_at: self.published_at.map(|date| date.to_rfc3339()),
            thumbs_up: None,
            thumbs_down: None,
            indexer_languages: None,
            indexer_subtitles: None,
            indexer_grabs: None,
            password_hint: None,
            parsed_release_metadata: None,
            quality_profile_decision: None,
            extra,
            response_attributes: self.external_ids,
            guid: None,
            info_url: None,
            provenance: None,
            candidate_token: None,
            queue_scope: None,
            coverage_scope: None,
            auto_eligible: None,
            auto_decision_code: None,
            auto_decision_summary: None,
        })
    }
}

impl AppUseCase {
    /// Evaluate an announcement against the existing catalog and automatic
    /// acquisition policy. No indexer configuration or search token is created.
    pub async fn submit_external_release(
        &self,
        actor: &User,
        input: ExternalReleaseInput,
        facets: &[MediaFacet],
    ) -> AppResult<ExternalReleaseOutcome> {
        let candidate = input.into_candidate()?;
        let libraries = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::ManageTitles)
            .await?;
        let facets = facets.to_vec();
        let bank = TitleContextBank::new(
            crate::import_title_resolution::MonitoredTitleMatcher::new(
                self.services.catalog.titles.clone(),
            ),
            move |title: &Title| {
                libraries.contains(&title.library_id)
                    && (facets.is_empty() || facets.contains(&title.facet))
            },
        );
        let Some(matched) =
            match_release_to_title_context(&candidate.title, &candidate.response_attributes, &bank)
                .await?
        else {
            return Ok(ExternalReleaseOutcome::rejected(
                "No unambiguous authorized monitored title matches this release",
            ));
        };
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&matched.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("matched title no longer exists".into()))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.evaluate_external_candidate(&title, candidate, Some(actor))
            .await
    }

    async fn external_submission_duplicate(
        &self,
        title: &Title,
        candidate: &IndexerSearchResult,
    ) -> AppResult<Option<ExternalReleaseOutcome>> {
        let source = candidate
            .canonical_download_source()
            .map(|(source, _)| source);
        let submissions = self
            .services
            .workflow
            .download_submissions
            .list_for_title(&title.id)
            .await?;
        for submission in submissions.into_iter().filter(|submission| {
            submission
                .source_hint
                .as_ref()
                .zip(source.as_ref())
                .is_some_and(|(left, right)| left == right)
                || submission
                    .info_hash
                    .as_deref()
                    .zip(candidate.info_hash())
                    .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
        }) {
            let locator = crate::ClientJobLocator::from_submission(&submission);
            let tracked = self
                .services
                .workflow
                .download_submissions
                .get_tracked_state(&locator)
                .await?;
            if matches!(
                tracked.as_deref(),
                Some("failed" | "ignored" | "imported" | "imported_seeding")
            ) {
                continue;
            }
            // A historical source identity is not a permanent acquisition veto.
            // Only positive evidence of the existing job can short-circuit the
            // ordinary policy and canonical retry/recovery checks below.
            let present = self
                .services
                .integrations
                .download_client
                .observe_download(&locator, 0)
                .await;
            if !matches!(present, Ok(crate::DownloadClientObservation::Present(ref item))
                if item.state != DownloadQueueState::Failed)
            {
                continue;
            }
            return Ok(Some(ExternalReleaseOutcome {
                status: ExternalReleaseStatus::Duplicate,
                reasons: vec!["Release already submitted".into()],
                title_id: Some(title.id.clone()),
                download_id: Some(submission.download_id.to_string()),
                pending_id: None,
            }));
        }
        Ok(None)
    }

    async fn evaluate_external_candidate(
        &self,
        title: &Title,
        candidate: IndexerSearchResult,
        actor: Option<&User>,
    ) -> AppResult<ExternalReleaseOutcome> {
        if let Some(duplicate) = self
            .external_submission_duplicate(title, &candidate)
            .await?
        {
            return Ok(duplicate);
        }
        let report = self
            .evaluate_external_title_releases(title, std::slice::from_ref(&candidate), actor)
            .await?;
        let outcome = report
            .external_outcome
            .expect("external acquisition retains its outcome");
        if outcome.status == ExternalReleaseStatus::Rejected
            && let Some(duplicate) = self
                .external_submission_duplicate(title, &candidate)
                .await?
        {
            return Ok(duplicate);
        }
        Ok(outcome)
    }

    async fn evaluate_external_title_releases(
        &self,
        title: &Title,
        releases: &[IndexerSearchResult],
        actor: Option<&User>,
    ) -> AppResult<RssSyncReport> {
        let mut outcome = ExternalReleaseOutcome::rejected(
            "Release does not cover an eligible monitored acquisition target",
        );
        outcome.title_id = Some(title.id.clone());
        if !title.monitored {
            return Ok(RssSyncReport {
                external_outcome: Some(outcome),
                ..Default::default()
            });
        }
        let now = Utc::now();
        let has_episodes = self
            .facet_registry
            .get(&title.facet)
            .is_some_and(|handler| handler.has_episodes());
        if !has_episodes
            && !crate::acquisition::targets::movie_is_available_for_acquisition(
                title.first_aired.as_deref(),
                title.digital_release_date.as_deref(),
                title.min_availability.as_deref().unwrap_or("announced"),
                &now,
            )
        {
            return Ok(RssSyncReport {
                external_outcome: Some(outcome),
                ..Default::default()
            });
        }
        let snapshot = crate::acquisition_workflow::DownloadClientSnapshot::fetch(self).await;
        let queue = self.title_queue_snapshot(&title.id, &snapshot).await;
        let profiles = self.delay_profiles().await?;
        let mut report = RssSyncReport {
            external_outcome: Some(outcome),
            external_actor: actor.cloned(),
            ..Default::default()
        };
        let mut grabbed_urls = HashSet::new();
        if has_episodes {
            self.process_rss_series_releases(
                title,
                releases,
                &snapshot,
                queue.as_ref(),
                &profiles,
                &mut grabbed_urls,
                &mut report,
                &now,
            )
            .await;
            self.process_rss_series_movie_releases(
                title,
                releases,
                &snapshot,
                queue.as_ref(),
                &profiles,
                &mut grabbed_urls,
                &mut report,
                &now,
            )
            .await;
        } else {
            self.process_rss_title_releases(
                title,
                releases,
                &snapshot,
                queue.as_ref(),
                &profiles,
                &mut grabbed_urls,
                &mut report,
                &now,
            )
            .await;
        }
        Ok(report)
    }

    pub(crate) async fn process_external_pending_releases(&self) -> AppResult<usize> {
        let pending = self
            .services
            .workflow
            .pending_releases
            .list_waiting_pending_releases()
            .await?;
        let mut grouped = HashMap::<String, Vec<(PendingRelease, IndexerSearchResult)>>::new();
        for pending in pending
            .into_iter()
            .filter(|pending| pending.indexer_id.is_none())
        {
            let candidate = pending_release_as_rss_result(&pending);
            if !candidate.extra.contains_key("external_announcement") {
                continue;
            }
            grouped
                .entry(pending.title_id.clone())
                .or_default()
                .push((pending, candidate));
        }
        let mut grabbed = 0;
        for (title_id, pending) in grouped {
            let Some(title) = self.services.catalog.titles.get_by_id(&title_id).await? else {
                continue;
            };
            let mut candidates = Vec::new();
            for (pending, candidate) in pending {
                if self
                    .external_submission_duplicate(&title, &candidate)
                    .await?
                    .is_some()
                {
                    self.services
                        .workflow
                        .pending_releases
                        .compare_and_set_pending_release_status(
                            &pending.id,
                            pending.status,
                            PendingReleaseStatus::Grabbed,
                            Some(&Utc::now().to_rfc3339()),
                        )
                        .await?;
                } else {
                    candidates.push(candidate);
                }
            }
            if !candidates.is_empty() {
                grabbed += self
                    .evaluate_external_title_releases(&title, &candidates, None)
                    .await?
                    .releases_grabbed;
            }
        }
        Ok(grabbed)
    }
}
