use super::*;

#[path = "arr_compat_bridge.rs"]
mod arr_compat_bridge;

/// The explanation a parked release stores. `explanation` is the text the
/// decision row records, `release_decision_explanation_for(candidate,
/// decision_code)`; a parked external release also carries its announcement.
pub(super) fn pending_explanation_for(
    candidate: &IndexerSearchResult,
    decision_code: ReleaseAutoDecisionCode,
    explanation: &str,
) -> Option<String> {
    let Some(announcement) = candidate.extra.get("external_announcement") else {
        return Some(explanation.to_owned());
    };
    if !explanation_reads_back_as_written(explanation) {
        let value = serde_json::from_str(explanation).ok();
        return Some(with_announcement(value, announcement));
    }
    // The serializer's own text reads back as the value it was written
    // from, so the value is extended directly instead of being read back.
    let value = decision_explanation_value_for(candidate, decision_code);
    Some(with_announcement(Some(value), announcement))
}

fn with_announcement(value: Option<serde_json::Value>, announcement: &serde_json::Value) -> String {
    let mut value = value.unwrap_or_else(|| serde_json::json!({}));
    value["external_announcement"] = announcement.clone();
    value.to_string()
}

/// Whether reading the serializer's `text` back gives the value it was
/// written from. It does unless an object's key is serde_json's raw-value
/// marker, which the reader expands, or the nesting is deep enough for the
/// reader's recursion limit; both are left to an actual read.
fn explanation_reads_back_as_written(text: &str) -> bool {
    const RAW_VALUE_TOKEN: &str = "$serde_json::private::RawValue";
    const SAFE_NESTING: usize = 100;
    if text.contains(RAW_VALUE_TOKEN) {
        return false;
    }
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for byte in text.bytes() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > SAFE_NESTING {
                    return false;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
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
        ..Default::default()
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
            return Err(AppError::validation_refused(
                crate::error_reasons::acquisition::RELEASE_URL_FLAG_LIST_EXCEEDS_INPUT_LIMIT,
                "release URL or flag list exceeds the input limit",
            ));
        }
        if name.is_empty() || name.len() > 2048 || source.is_empty() || source.len() > 256 {
            return Err(AppError::validation_refused(
                crate::error_reasons::acquisition::RELEASE_NAME_SOURCE_REQUIRED_INVALID,
                "release name and source are required and must be bounded",
            ));
        }
        if self.size_bytes.is_some_and(|size| size < 0) {
            return Err(AppError::validation_refused(
                crate::error_reasons::acquisition::RELEASE_SIZE_OUT_OF_RANGE,
                "release size must not be negative",
            ));
        }
        let magnet_hash = crate::extract_magnet_info_hash(download_url);
        let source_kind = if magnet_hash.is_some() {
            if self.protocol != ExternalReleaseProtocol::Torrent {
                return Err(AppError::validation_refused(
                    crate::error_reasons::acquisition::MAGNET_REQUIRES_TORRENT_PROTOCOL,
                    "a magnet requires the torrent protocol",
                ));
            }
            DownloadSourceKind::MagnetUri
        } else {
            let url = url::Url::parse(download_url).map_err(|_| {
                AppError::validation_refused(
                    crate::error_reasons::acquisition::RELEASE_DOWNLOAD_URL_INVALID,
                    "release download URL is invalid",
                )
            })?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(AppError::validation_refused(
                    crate::error_reasons::acquisition::RELEASE_URL_INVALID,
                    "release URL must use HTTP, HTTPS, or a valid magnet",
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
                return Err(AppError::validation_refused(
                    crate::error_reasons::acquisition::RELEASE_FLAG_UNSUPPORTED,
                    format!("unsupported release flag: {flag}"),
                ));
            }
            extra.insert(flag, serde_json::Value::Bool(true));
        }
        Ok(IndexerSearchResult {
            http: None,
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
            release_listing_json: None,
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
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::acquisition::MATCHED_TITLE_NO_LONGER_EXISTS_NOT_FOUND,
                    "matched title no longer exists",
                )
            })?;
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
        let availability_settings = self.movie_availability_policies().await?;
        let has_episodes = self
            .facet_registry
            .get(&title.facet)
            .is_some_and(|handler| handler.has_episodes());
        if !has_episodes
            && crate::acquisition::targets::movie_availability_decision_for_title(
                title,
                &availability_settings,
                &now,
            )
            .status
                != crate::types::MovieAvailabilityStatus::Available
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
        let mut pass = super::rss::RssPassState::default();
        let mut context = super::RssTitleContext::new(&title.id);
        if has_episodes {
            self.process_rss_series_releases(
                title,
                releases,
                &mut context,
                &snapshot,
                queue.as_ref(),
                &profiles,
                &mut pass,
                &mut report,
                &now,
            )
            .await;
            self.process_rss_series_movie_releases(
                title,
                releases,
                &mut context,
                &snapshot,
                queue.as_ref(),
                &profiles,
                &mut pass,
                &mut report,
                &now,
            )
            .await;
        } else {
            self.process_rss_title_releases(
                title,
                releases,
                &mut context,
                &snapshot,
                queue.as_ref(),
                &profiles,
                &mut pass,
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

#[cfg(test)]
mod pending_explanation_tests {
    use super::*;
    use crate::acquisition_release_search::{
        annotate_auto_decision, serialize_decision_explanation,
    };

    /// What a parked release stored before: the annotated copy's explanation,
    /// read back and extended when it carries an announcement.
    fn reparsed_pending_explanation(candidate: &IndexerSearchResult) -> Option<String> {
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

    fn synthetic_candidate(title: &str) -> IndexerSearchResult {
        IndexerSearchResult {
            http: None,
            indexer_id: None,
            source: "synthetic-indexer".to_string(),
            title: title.to_string(),
            link: None,
            download_url: Some("https://synthetic.invalid/get/1".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            size_bytes: Some(1_000),
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            indexer_languages: None,
            indexer_subtitles: None,
            indexer_grabs: None,
            password_hint: None,
            parsed_release_metadata: Some(crate::parse_release_metadata(title)),
            quality_profile_decision: None,
            extra: Default::default(),
            response_attributes: Default::default(),
            guid: Some("synthetic-guid".to_string()),
            info_url: None,
            provenance: None,
            candidate_token: None,
            queue_scope: None,
            coverage_scope: None,
            auto_eligible: Some(true),
            auto_decision_code: Some("synthetic_stale_code".to_string()),
            auto_decision_summary: None,
            release_listing_json: None,
        }
    }

    fn nested(depth: usize) -> serde_json::Value {
        (0..depth).fold(serde_json::json!(1), |inner, _| serde_json::json!([inner]))
    }

    #[test]
    fn parked_text_matches_reading_the_explanation_back() {
        let announcement = serde_json::json!({
            "flags": ["freeleech", "internal"],
            "tvdb_id": 101,
            "tmdb_id": null,
            "imdb_id": "tt0000001",
        });
        let conflicts = [
            None,
            Some(serde_json::json!([{"kind": "tvdb", "expected": "1", "actual": "2", "f": 0.1}])),
            Some(serde_json::json!({"$serde_json::private::RawValue": "[1,2]"})),
            Some(nested(110)),
            Some(nested(140)),
        ];
        let codes = [
            ReleaseAutoDecisionCode::PendingDelay,
            ReleaseAutoDecisionCode::MinimumAge,
            ReleaseAutoDecisionCode::ReleaseAgeUnknown,
            ReleaseAutoDecisionCode::DownloadClientUnavailable,
        ];
        for title in [
            "Synthetic.Show.S01E02.1080p.WEB-DL-SYNTH",
            "Synthetic \"Film\" é 2026",
        ] {
            for conflict in &conflicts {
                for with_announcement in [false, true] {
                    let mut candidate = synthetic_candidate(title);
                    if let Some(conflict) = conflict {
                        candidate
                            .extra
                            .insert("scryer_external_id_conflicts".into(), conflict.clone());
                    }
                    if with_announcement {
                        candidate
                            .extra
                            .insert("external_announcement".into(), announcement.clone());
                    }
                    for code in codes {
                        let mut annotated = candidate.clone();
                        annotate_auto_decision(&mut annotated, code);
                        let expected = reparsed_pending_explanation(&annotated);
                        let explanation = release_decision_explanation_for(&candidate, code);
                        let parked =
                            pending_explanation_for(&candidate, code, explanation.as_str());
                        assert_eq!(parked, expected, "code {}", code.as_str());
                    }
                }
            }
        }
    }
}
