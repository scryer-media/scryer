fn base_completed_import_result(
    import_id: &str,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    started_at: DateTime<Utc>,
) -> ImportResult {
    ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: None,
        title_id: None,
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: release_evidence.release_title(None),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        quality: None,
        episode_ids: Vec::new(),
        file_size_bytes: None,
        link_type: None,
        error_message: None,
        release_burned: false,
        started_at,
        completed_at: Utc::now(),
    }
}
fn facet_for_completed_download(completed: &CompletedDownload) -> Option<MediaFacet> {
    match extract_parameter(&completed.parameters, "*scryer_facet")
        .as_deref()
        .map(str::trim)
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("movie") => Some(MediaFacet::Movie),
        Some("series") => Some(MediaFacet::Series),
        Some("anime") => Some(MediaFacet::Anime),
        _ => None,
    }
}
pub(crate) fn facet_from_tracked_label(value: Option<&str>) -> Option<MediaFacet> {
    match value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("movie") => Some(MediaFacet::Movie),
        Some("series") => Some(MediaFacet::Series),
        Some("anime") => Some(MediaFacet::Anime),
        _ => None,
    }
}
// ---------------------------------------------------------------------------
// Series import: process ALL video files, link each to its episode
// ---------------------------------------------------------------------------

#[expect(
    clippy::too_many_arguments,
    reason = "series import keeps operational completion and release evidence as separate inputs"
)]
async fn import_series_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    source_root: &Path,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template,
        specials_folder_template,
    } = resolve_import_paths(app, title).await?;
    let full_folder_path = effective_title_folder_path(&media_root, title, &folder_template, None);
    ensure_import_title_folder_available(app, title, &full_folder_path).await?;

    let quality_profile = resolve_import_quality_profile(app, title).await?;

    let nfo_enabled = app
        .resolve_nfo_write_on_import(Some(&title.library_id), &title.facet)
        .await?;
    let import_mode = crate::seeding_gate::resolve_seeding_safe_import_mode(
        app,
        Some(&title.library_id),
        &title.facet,
        Some(completed),
    )
    .await?;

    let mut imported_count: usize = 0;
    let mut skipped_count: usize = 0;
    let mut ignored_count: usize = 0;
    let mut rejected_count: usize = 0;
    let mut release_burned = false;
    let mut failed_count: usize = 0;
    let mut last_error: Option<String> = None;
    let mut rejection_reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut last_rejection_skip_reason: Option<ImportSkipReason> = None;
    let mut last_skipped_message: Option<String> = None;
    let mut last_skipped_skip_reason: Option<ImportSkipReason> = None;
    let mut imported_updates: Vec<NotificationMediaUpdate> = Vec::new();
    // Total bytes across every file this import brought in. Stays `None` until
    // at least one file reports a size, so a legacy-shaped import that knows no
    // sizes reports null rather than a misleading zero.
    let mut imported_size_bytes: Option<i64> = None;
    let mut imported_episode_ids: Vec<String> = Vec::new();
    let mut attributed_episode_ids: Vec<String> = Vec::new();
    let mut imported_link_type: Option<scryer_domain::ImportStrategy> = None;
    let expected_episode_ids =
        expected_episode_ids_for_completed_download(app, title, release_evidence).await;
    let pack_plan = build_episode_pack_import_plan(
        app,
        title,
        release_evidence,
        source_root,
        video_files,
        expected_episode_ids.as_ref(),
    )
    .await?;
    // `video_files` came from `find_video_files(dir, true)`: samples are already
    // excluded, so this is the count Sonarr's `OtherVideoFiles` rule wants.
    let video_file_count = video_files.len();
    // One release, one blocklist row — accumulated across the members and
    // written once after the loop. See [`DownloadBlocklistLedger`].
    let mut blocklist_ledger = DownloadBlocklistLedger::for_download(release_evidence);

    for source_video in video_files {
        match import_single_episode_file(
            app,
            actor,
            title,
            import_id,
            rename_enabled,
            &rename_template,
            &season_folder_template,
            &specials_folder_template,
            &full_folder_path,
            completed,
            release_evidence,
            source_video,
            &quality_profile,
            nfo_enabled,
            expected_episode_ids.as_ref(),
            pack_plan
                .as_ref()
                .and_then(|plan| plan.disposition_for(source_video)),
            video_file_count,
            &mut blocklist_ledger,
        )
        .await
        {
            Ok(EpisodeImportOutcome::Imported {
                dest_path,
                episode_ids,
                link_type,
                size_bytes,
                ..
            }) => {
                imported_count += 1;
                if let Some(size_bytes) = size_bytes {
                    imported_size_bytes =
                        Some(imported_size_bytes.unwrap_or(0).saturating_add(size_bytes));
                }
                imported_updates.push(NotificationMediaUpdate::created(dest_path));
                append_unique_episode_ids(&mut imported_episode_ids, &episode_ids);
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                if link_type == Some(scryer_domain::ImportStrategy::Move) {
                    imported_link_type = link_type;
                }
            }
            Ok(EpisodeImportOutcome::Skipped {
                message,
                skip_reason,
                episode_ids,
                ..
            }) => {
                skipped_count += 1;
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                last_skipped_message = Some(message);
                last_skipped_skip_reason = skip_reason;
            }
            Ok(EpisodeImportOutcome::Ignored {
                message,
                episode_ids,
                ..
            }) => {
                ignored_count += 1;
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                last_skipped_message = Some(message);
                last_skipped_skip_reason = None;
            }
            Ok(EpisodeImportOutcome::Rejected {
                rejection,
                disposition,
                episode_ids,
                ..
            }) => {
                rejected_count += 1;
                release_burned |= matches!(
                    disposition,
                    crate::import_decide::RejectionDisposition::Blocklist
                );
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                *rejection_reasons
                    .entry(rejection.message.clone())
                    .or_insert(0) += 1;
                last_error = Some(rejection.message.clone());
                last_rejection_skip_reason = rejection.skip_reason.clone();
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    file = %source_video.display(),
                    title = %title.name,
                    "failed to import episode file"
                );
                last_error = Some(err.to_string());
                failed_count += 1;
            }
        }
    }

    blocklist_ledger.finalize(app, actor, title).await;

    if imported_count > 0 {
        persist_title_folder_path_if_missing(app, title, &full_folder_path).await?;
        write_series_sidecars(app, title, &full_folder_path, nfo_enabled).await;
    }

    let move_import_has_failure =
        import_mode == scryer_domain::ImportMode::Move && failed_count > 0;
    let (decision, status, skip_reason) = aggregate_episode_import_outcome(
        import_mode,
        imported_count,
        rejected_count,
        failed_count,
        last_rejection_skip_reason,
        last_skipped_skip_reason,
    );
    let release_burned = matches!(&decision, ImportDecision::Rejected) && release_burned;

    let error_message = episode_import_summary_message(
        EpisodeImportSummaryCounts {
            imported: imported_count,
            ignored: ignored_count,
            skipped: skipped_count,
            rejected: rejected_count,
            failed: failed_count,
        },
        &rejection_reasons,
        last_error.as_deref(),
        last_skipped_message,
    );

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision,
        skip_reason,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: release_evidence.release_title(None),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        quality: None,
        episode_ids: attributed_episode_ids,
        file_size_bytes: None,
        link_type: imported_link_type,
        error_message,
        release_burned,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    let status = completed_import_status_for_result(&result, status);
    app.update_import_status_and_notify(import_id, status, result_json)
        .await?;

    if imported_count > 0 && !move_import_has_failure {
        app.append_domain_event(new_title_domain_event(
            actor,
            title,
            DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: title_context_snapshot(title),
                media_updates: imported_updates
                    .into_iter()
                    .map(|update| created_media_update(update.path))
                    .collect(),
                imported_count: imported_count as i32,
                import_id: Some(import_id.to_string()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: release_evidence.release_title(None),
                source_path: Some(completed.dest_dir.clone()),
                dest_path: None,
                quality: None,
                episode_ids: imported_episode_ids,
                size_bytes: imported_size_bytes,
            }),
        ))
        .await?;
    }

    Ok(result)
}
enum EpisodeImportOutcome {
    Imported {
        dest_path: String,
        episode_ids: Vec<String>,
        imported_media_file_id: Option<String>,
        reason_code: Option<String>,
        link_type: Option<scryer_domain::ImportStrategy>,
        source_cleanup: Option<Box<scryer_domain::ImportSourceCleanupGuard>>,
        destination_permit: ImportDestinationPermit,
        /// Bytes written for this file, so multi-file imports can report a
        /// total without re-stating the destination paths.
        size_bytes: Option<i64>,
        /// The file was imported *and* its release must be burned (D2: an
        /// honest 720p fills an empty scope, but must never come back as an
        /// "upgrade" to the 1080p it advertised).
        ///
        /// Reported rather than carried out, because the write is deduplicated
        /// per download: twelve members of a pack that all trip this must not
        /// write twelve identical blocklist rows. See [`DownloadBlocklistLedger`].
        blocklist_after_import: Option<crate::import_decide::BlocklistDirective>,
    },
    Skipped {
        message: String,
        reason_code: Option<String>,
        skip_reason: Option<ImportSkipReason>,
        episode_ids: Vec<String>,
    },
    Ignored {
        message: String,
        reason_code: String,
        episode_ids: Vec<String>,
    },
    Rejected {
        rejection: crate::post_download_gate::ImportedFileRejection,
        /// What the refusal costs the release (D17). Replaces a
        /// `finalize_before_import: bool` that could only say "blocklist and
        /// reopen" or "do nothing", and so had no way to express a hold — the
        /// third case the import gate genuinely produces.
        disposition: crate::import_decide::RejectionDisposition,
        reason_code: Option<String>,
        episode_ids: Vec<String>,
    },
}

/// One blocklist row per completed download, not one per member file.
///
/// A twelve-file season pack that trips a truth verdict used to write twelve
/// identical blocklist entries, each attributed to the one episode its member
/// happened to cover — so the operator saw the same release burned a dozen
/// times and no single row said which season it was. The release is the unit
/// being blocklisted, and a download carries exactly one, so the write is
/// deferred to the end of the file loop and attributed to the *union* of the
/// members' episode ids plus the download's collection scope (review m9).
///
/// The release attempt and the `ImportRejected` domain event ride along with the
/// blocklist for the same reason: one download, one recorded failure.
#[derive(Default)]
pub(super) struct DownloadBlocklistLedger {
    release_title: Option<String>,
    source_path: Option<PathBuf>,
    /// Set by the first member that was *refused*. A refusal outranks an
    /// imported-but-mis-advertised member: it carries the recycle reason and it
    /// is what reopens the scopes.
    rejection: Option<crate::post_download_gate::ImportedFileRejection>,
    /// Reason text from an imported-and-blocklisted member, used only when no
    /// member was refused outright.
    import_reason: Option<String>,
    episode_ids: Vec<String>,
    /// The members whose import was *refused* — the only scopes a reopen may
    /// touch. `episode_ids` above is the union the blocklist row is attributed
    /// to; a member that imported mis-advertised is in that union but has already
    /// been marked completed and must not be flipped back to `wanted`.
    rejected_episode_ids: Vec<String>,
    collection_id: Option<String>,
}

impl DownloadBlocklistLedger {
    fn for_download(release_evidence: &ReleaseEvidence) -> Self {
        let collection_id = match release_evidence.scope() {
            Some(SubmissionScope::Collection { collection_id }) => Some(collection_id.clone()),
            _ => None,
        };
        Self {
            collection_id,
            ..Self::default()
        }
    }

    fn note_release(&mut self, release_title: &str, source_path: &Path, episode_ids: &[String]) {
        if self.release_title.is_none() {
            self.release_title = Some(release_title.to_string());
            self.source_path = Some(source_path.to_path_buf());
        }
        append_unique_episode_ids(&mut self.episode_ids, episode_ids);
    }

    fn record_rejection(
        &mut self,
        release_title: &str,
        source_path: &Path,
        episode_ids: &[String],
        rejection: &crate::post_download_gate::ImportedFileRejection,
    ) {
        self.note_release(release_title, source_path, episode_ids);
        append_unique_episode_ids(&mut self.rejected_episode_ids, episode_ids);
        if self.rejection.is_none() {
            self.rejection = Some(crate::post_download_gate::ImportedFileRejection {
                message: rejection.message.clone(),
                recycle_reason: rejection.recycle_reason,
                skip_reason: rejection.skip_reason.clone(),
                blocking_rule_codes: rejection.blocking_rule_codes.clone(),
            });
        }
    }

    fn record_import_blocklist(
        &mut self,
        release_title: &str,
        source_path: &Path,
        episode_ids: &[String],
        reason: String,
    ) {
        self.note_release(release_title, source_path, episode_ids);
        if self.import_reason.is_none() {
            self.import_reason = Some(reason);
        }
    }

    /// The one write this download earned, or `None` if it earned none.
    ///
    /// Separated from carrying it out so the accumulation — one release, one
    /// row, the union of the members' episodes — is testable without an app.
    fn planned_write(&self) -> Option<PlannedBlocklistWrite<'_>> {
        let release_title = self.release_title.as_deref()?;
        let source_path = self.source_path.as_deref()?;
        Some(PlannedBlocklistWrite {
            release_title,
            source_path,
            rejection: self.rejection.as_ref(),
            import_reason: self.import_reason.as_deref(),
            attribution: crate::post_download_gate::BlocklistAttribution {
                episode_ids: &self.episode_ids,
                collection_id: self.collection_id.as_deref(),
                series_movie_link_id: None,
            },
            reopen_episode_ids: &self.rejected_episode_ids,
        })
    }

    /// Write the single entry this download earned, if any.
    async fn finalize(self, app: &AppUseCase, actor: &User, title: &scryer_domain::Title) {
        let Some(write) = self.planned_write() else {
            return;
        };
        if let Some(rejection) = write.rejection {
            crate::post_download_gate::reject_source_file_before_import(
                app,
                crate::domain_events::DomainEventActor::from(actor),
                title,
                write.release_title,
                write.source_path,
                write.attribution,
                // Reopen only the refused members; the row is attributed to
                // the whole union. Empty cannot happen for a recorded
                // rejection, but `None` keeps the default path if it did.
                (!write.reopen_episode_ids.is_empty()).then_some(write.reopen_episode_ids),
                rejection,
            )
            .await;
            return;
        }
        if let Some(reason) = write.import_reason {
            crate::post_download_gate::blocklist_release_for_title(
                app,
                title,
                write.release_title,
                Some(reason.to_string()),
            )
            .await;
        }
    }
}

/// The single blocklist write a download earned, resolved but not yet performed.
pub(super) struct PlannedBlocklistWrite<'a> {
    pub release_title: &'a str,
    pub source_path: &'a Path,
    /// `Some` when a member was refused: the write recycles, reopens and
    /// blocklists. `None` with an `import_reason` means every member imported
    /// and one of them was mis-advertised — blocklist only.
    pub rejection: Option<&'a crate::post_download_gate::ImportedFileRejection>,
    pub import_reason: Option<&'a str>,
    pub attribution: crate::post_download_gate::BlocklistAttribution<'a>,
    /// The refused members — the only scopes the write may reopen. The
    /// attribution above is the union of every member the download covered, so
    /// the blocklist row names the whole release, but a member that imported
    /// mis-advertised has already been marked completed and must not be
    /// flipped back to `wanted` with a file on disk.
    pub reopen_episode_ids: &'a [String],
}

/// A skipped episode file whose destination already holds the identical file
/// (`check_not_already_imported`) is not a rejection: the unit is in place, so
/// the automatic and manual paths both record it as `already_present` and let
/// the download finalize as imported instead of retrying forever.
pub(super) fn episode_skip_is_already_present(skip_reason: Option<&ImportSkipReason>) -> bool {
    matches!(
        skip_reason,
        Some(ImportSkipReason::AlreadyImported | ImportSkipReason::DuplicateFile)
    )
}

fn append_unique_episode_ids(target: &mut Vec<String>, source: &[String]) {
    for episode_id in source {
        if !target.contains(episode_id) {
            target.push(episode_id.clone());
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EpisodeImportSummaryCounts {
    imported: usize,
    ignored: usize,
    skipped: usize,
    rejected: usize,
    failed: usize,
}

/// The one-line summary a mixed-outcome series import records.
///
/// Every rejection reason is listed with how many members it held, most
/// frequent first, so a pack that imported eight members and held eight more
/// says *why* each group was held — not just whatever the last file happened
/// to hit. `Last error` is reserved for members whose import actually failed
/// to execute.
fn episode_import_summary_message(
    counts: EpisodeImportSummaryCounts,
    rejection_reasons: &BTreeMap<String, usize>,
    last_error: Option<&str>,
    last_skipped_message: Option<String>,
) -> Option<String> {
    let EpisodeImportSummaryCounts {
        imported,
        ignored,
        skipped,
        rejected,
        failed,
    } = counts;
    if imported == 0 && failed == 0 && rejected == 0 && skipped > 0 {
        return last_skipped_message;
    }
    if failed == 0 && skipped == 0 && ignored == 0 && rejected == 0 {
        return None;
    }

    let mut summary = format!(
        "{imported} imported, {ignored} ignored, {skipped} skipped, {rejected} rejected, {failed} failed"
    );
    if rejected > 0 {
        let mut reasons: Vec<(&String, &usize)> = rejection_reasons.iter().collect();
        reasons.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
        let listed = reasons
            .iter()
            .map(|(message, count)| format!("{count} × {message}"))
            .collect::<Vec<_>>()
            .join(" | ");
        if !listed.is_empty() {
            summary.push_str(". Rejected: ");
            summary.push_str(&listed);
        }
    }
    if let Some(error) = last_error
        && (failed > 0 || rejection_reasons.is_empty())
    {
        summary.push_str(". Last error: ");
        summary.push_str(error);
    }
    Some(summary)
}

#[cfg(test)]
mod episode_import_summary_message_tests {
    use super::*;

    fn counts(
        imported: usize,
        rejected: usize,
        failed: usize,
        skipped: usize,
    ) -> EpisodeImportSummaryCounts {
        EpisodeImportSummaryCounts {
            imported,
            ignored: 0,
            skipped,
            rejected,
            failed,
        }
    }

    #[test]
    fn every_rejection_reason_is_counted_most_frequent_first() {
        let reasons = BTreeMap::from([
            ("could not identify this pack member".to_string(), 1),
            ("outside the seasons declared by this pack".to_string(), 7),
        ]);
        assert_eq!(
            episode_import_summary_message(
                counts(8, 8, 0, 0),
                &reasons,
                Some("could not identify this pack member"),
                None,
            )
            .as_deref(),
            Some(
                "8 imported, 0 ignored, 0 skipped, 8 rejected, 0 failed. Rejected: 7 × outside the seasons declared by this pack | 1 × could not identify this pack member"
            )
        );
    }

    #[test]
    fn execution_failure_keeps_its_last_error() {
        assert_eq!(
            episode_import_summary_message(
                counts(0, 0, 1, 0),
                &BTreeMap::new(),
                Some("unexpected hardlink failure"),
                None,
            )
            .as_deref(),
            Some("0 imported, 0 ignored, 0 skipped, 0 rejected, 1 failed. Last error: unexpected hardlink failure")
        );
    }

    #[test]
    fn rejections_alongside_a_failure_report_both() {
        let reasons = BTreeMap::from([("held for manual import".to_string(), 1)]);
        assert_eq!(
            episode_import_summary_message(
                counts(1, 1, 1, 0),
                &reasons,
                Some("disk full"),
                None,
            )
            .as_deref(),
            Some(
                "1 imported, 0 ignored, 0 skipped, 1 rejected, 1 failed. Rejected: 1 × held for manual import. Last error: disk full"
            )
        );
    }

    #[test]
    fn all_skipped_reports_the_skip_message_alone() {
        assert_eq!(
            episode_import_summary_message(
                counts(0, 0, 0, 2),
                &BTreeMap::new(),
                None,
                Some("nothing wanted".to_string()),
            )
            .as_deref(),
            Some("nothing wanted")
        );
    }

    #[test]
    fn clean_import_has_no_summary() {
        assert_eq!(
            episode_import_summary_message(counts(3, 0, 0, 0), &BTreeMap::new(), None, None),
            None
        );
    }
}

fn aggregate_episode_import_outcome(
    import_mode: scryer_domain::ImportMode,
    imported_count: usize,
    rejected_count: usize,
    failed_count: usize,
    last_rejection_skip_reason: Option<ImportSkipReason>,
    last_skipped_skip_reason: Option<ImportSkipReason>,
) -> (ImportDecision, ImportStatus, Option<ImportSkipReason>) {
    if import_mode == scryer_domain::ImportMode::Move && failed_count > 0 {
        (ImportDecision::Failed, ImportStatus::Failed, None)
    } else if rejected_count > 0 {
        (
            ImportDecision::Rejected,
            ImportStatus::Failed,
            last_rejection_skip_reason,
        )
    } else if imported_count > 0 {
        (ImportDecision::Imported, ImportStatus::Completed, None)
    } else if failed_count > 0 {
        (ImportDecision::Failed, ImportStatus::Failed, None)
    } else {
        (
            ImportDecision::Skipped,
            ImportStatus::Skipped,
            last_skipped_skip_reason,
        )
    }
}

#[cfg(test)]
mod aggregate_episode_import_outcome_tests {
    use super::*;

    #[test]
    fn imported_member_wins_over_another_non_move_failure() {
        assert_eq!(
            aggregate_episode_import_outcome(
                scryer_domain::ImportMode::HardlinkOrCopy,
                1,
                0,
                1,
                None,
                None,
            ),
            (ImportDecision::Imported, ImportStatus::Completed, None)
        );
    }

    #[test]
    fn rejected_member_wins_over_another_import() {
        assert_eq!(
            aggregate_episode_import_outcome(
                scryer_domain::ImportMode::HardlinkOrCopy,
                1,
                1,
                0,
                Some(ImportSkipReason::PolicyMismatch),
                None,
            ),
            (
                ImportDecision::Rejected,
                ImportStatus::Failed,
                Some(ImportSkipReason::PolicyMismatch),
            )
        );
    }

    #[test]
    fn move_failure_wins_over_another_import() {
        assert_eq!(
            aggregate_episode_import_outcome(scryer_domain::ImportMode::Move, 1, 0, 1, None, None,),
            (ImportDecision::Failed, ImportStatus::Failed, None)
        );
    }

    #[test]
    fn all_ignored_members_are_skipped() {
        assert_eq!(
            aggregate_episode_import_outcome(
                scryer_domain::ImportMode::HardlinkOrCopy,
                0,
                0,
                0,
                None,
                None,
            ),
            (ImportDecision::Skipped, ImportStatus::Skipped, None)
        );
    }

    #[test]
    fn all_failed_members_fail() {
        assert_eq!(
            aggregate_episode_import_outcome(
                scryer_domain::ImportMode::HardlinkOrCopy,
                0,
                0,
                1,
                None,
                None,
            ),
            (ImportDecision::Failed, ImportStatus::Failed, None)
        );
    }
}

async fn expected_episode_ids_for_completed_download(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    release_evidence: &ReleaseEvidence,
) -> Option<HashSet<String>> {
    if let Some(scope) = release_evidence.scope()
        && let Some(ids) =
            expected_episode_ids_from_submission_scope(app, title, scope, false).await
        && !ids.is_empty()
    {
        return Some(ids);
    }
    let release_title = release_evidence.release_title(None)?;
    expected_episode_ids_from_release_title(app, title, &release_title).await
}
/// The episodes a grab's submission scope names outright — the one derivation
/// both the import's grabbed-release gate and the post-import verification use.
/// A collection (season) scope names every episode of the season: monitoring
/// decides what Scryer searches for, never which downloaded file belongs to
/// the grab. Verification alone passes `monitored_collection_preference` so a
/// season pack that lacks a file for an unmonitored episode still counts as
/// complete; title/series-movie/orphan scopes name none.
pub(crate) async fn expected_episode_ids_from_submission_scope(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    scope: &SubmissionScope,
    monitored_collection_preference: bool,
) -> Option<HashSet<String>> {
    match scope {
        SubmissionScope::Episode { episode_id } => Some(HashSet::from([episode_id.clone()])),
        SubmissionScope::EpisodeSet { episode_ids } => Some(episode_ids.iter().cloned().collect()),
        SubmissionScope::Collection { collection_id } => {
            if monitored_collection_preference
                && let Some(monitored) =
                    episode_ids_for_collection(app, title, collection_id, true).await
            {
                return Some(monitored);
            }
            episode_ids_for_collection(app, title, collection_id, false).await
        }
        SubmissionScope::Title | SubmissionScope::SeriesMovie { .. } | SubmissionScope::Orphan => {
            None
        }
    }
}
async fn expected_episode_ids_from_release_title(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    release_title: &str,
) -> Option<HashSet<String>> {
    let parsed = normalize_release_title_signal(parse_release_metadata(release_title));
    let ep_meta = parsed.episode.as_ref()?;
    let season = ep_meta.season.unwrap_or(1).to_string();
    let mut episodes = resolve_target_episodes(app, title, ep_meta, &season).await;

    if episodes.is_empty() {
        None
    } else {
        Some(episodes.drain(..).map(|episode| episode.id).collect())
    }
}
fn resolved_episode_ids_are_within_expected(
    target_episode_ids: &[String],
    expected_episode_ids: &HashSet<String>,
) -> bool {
    // An unresolved file binds to nothing, so it is never "within" the grabbed
    // release; the caller rejects that case first with a more precise reason.
    !target_episode_ids.is_empty()
        && target_episode_ids
            .iter()
            .all(|episode_id| expected_episode_ids.contains(episode_id))
}
async fn episode_ids_for_collection(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    collection_id: &str,
    monitored_only: bool,
) -> Option<HashSet<String>> {
    match app
        .services
        .catalog
        .shows
        .list_episodes_for_collection(collection_id)
        .await
    {
        Ok(episodes) => {
            let ids: HashSet<String> = episodes
                .into_iter()
                .filter(|episode| episode.title_id == title.id)
                .filter(|episode| !monitored_only || episode.monitored)
                .map(|episode| episode.id)
                .collect();
            (!ids.is_empty()).then_some(ids)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                collection_id,
                title_id = %title.id,
                "failed to resolve expected grabbed-release episode set"
            );
            None
        }
    }
}
async fn cleanup_superseded_episode_incumbents(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    superseded: &[crate::EpisodeScopedMediaFile],
    replacement_file_id: &str,
    replacement_path: &Path,
) {
    for incumbent in superseded {
        let mut recycle_result = None;
        let old_path =
            crate::stored_paths::stored_path_to_path_buf(&incumbent.media_file.file_path);
        if old_path.exists() {
            let old_file_recycle_context = match crate::upgrade::resolve_old_file_recycle_context(
                app,
                title,
                &incumbent.media_file,
            )
            .await
            {
                Ok(context) => context,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        path = %old_path.display(),
                        file_id = %incumbent.media_file.id,
                        "failed to resolve recycle context for superseded episode incumbent; keeping its database record to avoid orphaning the on-disk file"
                    );
                    continue;
                }
            };
            let metadata = crate::recycle_bin::ReplacedMediaRecycleMetadata {
                original_path: &incumbent.media_file.file_path,
                original_file_id: &incumbent.media_file.id,
                size_bytes: incumbent.media_file.size_bytes as u64,
                title_id: &title.id,
                media_root: Some(old_file_recycle_context.media_root.as_str()),
            };

            match crate::recycle_bin::recycle_replaced_media_file(
                &old_file_recycle_context.recycle_config,
                &old_path,
                metadata,
                true,
            )
            .await
            {
                Ok(result) => recycle_result = result,
                Err(error) => {
                    // Physical cleanup failed or was refused for safety. The file is
                    // still on disk, so keep its database record rather than orphaning
                    // the file; a later upgrade can retry cleanup.
                    tracing::warn!(
                        error = %error,
                        path = %old_path.display(),
                        file_id = %incumbent.media_file.id,
                        "failed to recycle superseded episode incumbent; keeping its database record to avoid orphaning the on-disk file"
                    );
                    continue;
                }
            }
        }

        if let Err(error) = app
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::MediaFileDeleted(scryer_domain::MediaFileDeletedEventData {
                    title: title_context_snapshot(title),
                    media_updates: vec![deleted_media_update(
                        incumbent.media_file.file_path.clone(),
                    )],
                    file_id: Some(incumbent.media_file.id.clone()),
                    reason: scryer_domain::MediaFileDeletedReason::UpgradeCleanup,
                    episode_ids: incumbent.episode_ids.clone(),
                }),
            ))
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %incumbent.media_file.id,
                "failed to emit superseded episode cleanup event"
            );
        }

        let deleted_record = match app
            .delete_media_file_record_with_dependents(&incumbent.media_file.id)
            .await
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    file_id = %incumbent.media_file.id,
                    "failed to delete superseded episode media file record"
                );
                false
            }
        };

        if deleted_record
            && let Err(error) = crate::recycle_bin::commit_recycle_entry(
                &recycle_result,
                replacement_file_id,
                replacement_path,
            )
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %incumbent.media_file.id,
                "superseded recycle entry could not be committed; it will not auto-purge"
            );
        }
    }
}
/// Why an obfuscated video file's episode identity could not be trusted, as an
/// actionable operator message; `None` when the file name carries usable
/// release signal of its own (the generic message then applies).
fn ambiguous_obfuscated_episode_message(
    source_video: &Path,
    release_evidence: &ReleaseEvidence,
    video_file_count: usize,
) -> Option<String> {
    let file_info = parsed_release_from_file_stem(source_video);
    if has_usable_release_title_signal(&file_info) {
        return None;
    }

    if video_file_count > 1 {
        // With other video files in the download each member must identify
        // itself (`build_augmented_episode_import_metadata_for_title`); the
        // release name's numbering was never applied to this file.
        return Some(format!(
            "Automatic import could not identify the episode for this file: this download contains {video_file_count} video files and this file's name is obfuscated. Open Manual Import and assign the correct season and episode."
        ));
    }

    let release_title = release_evidence.release_title(Some(source_video))?;
    let release_info = normalize_release_title_signal(parse_release_metadata(&release_title));
    let episode = release_info.episode.as_ref()?;
    if episode.season.is_some() {
        return None;
    }
    let episode_number = episode
        .episode_numbers
        .first()
        .copied()
        .or(episode.absolute_episode)
        .or_else(|| episode.absolute_episode_numbers.first().copied())?;

    Some(format!(
        "Automatic import could not choose a season for episode {episode_number}: the release name does not include a season and the downloaded filename is obfuscated. Open Manual Import and assign the correct season and episode."
    ))
}

fn sole_submission_episode_id(
    release_evidence: &ReleaseEvidence,
    other_video_files: bool,
) -> Option<&str> {
    if other_video_files {
        return None;
    }

    match release_evidence.scope()? {
        SubmissionScope::Episode { episode_id } => Some(episode_id.as_str()),
        SubmissionScope::EpisodeSet { episode_ids } if episode_ids.len() == 1 => {
            episode_ids.first().map(String::as_str)
        }
        _ => None,
    }
}

async fn grabbed_episode_fallback(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    release_evidence: &ReleaseEvidence,
    other_video_files: bool,
) -> AppResult<Option<scryer_domain::Episode>> {
    let Some(episode_id) = sole_submission_episode_id(release_evidence, other_video_files) else {
        return Ok(None);
    };
    let Some(episode) = app
        .services
        .catalog
        .shows
        .get_episode_by_id(episode_id)
        .await?
    else {
        tracing::warn!(
            episode_id,
            title_id = %title.id,
            "import: acquisition episode no longer exists; falling back to artifact parsing"
        );
        return Ok(None);
    };
    if episode.title_id != title.id {
        tracing::warn!(
            episode_id,
            scoped_title_id = %episode.title_id,
            import_title_id = %title.id,
            "import: acquisition episode belongs to another title; falling back to artifact parsing"
        );
        return Ok(None);
    }
    Ok(Some(episode))
}

/// Reconcile alternate numbering only when the file and the original grab
/// independently corroborate the one catalog episode Scryer submitted.
///
/// Scene releases can carry a collection's local numbering while the catalog
/// has that content in a different season. A parseable filename normally wins,
/// but once that filename resolves to no catalog episode, this narrowly admits
/// the scoped episode when the series title matches exactly, its episode title
/// is a near match, and both episode numbers agree.
async fn reconcile_unresolved_scene_episode_from_scoped_release(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    release_evidence: &ReleaseEvidence,
    source_video: &Path,
    other_video_files: bool,
) -> AppResult<Option<scryer_domain::Episode>> {
    let Some(scoped_episode) =
        grabbed_episode_fallback(app, title, release_evidence, other_video_files).await?
    else {
        return Ok(None);
    };

    let file_metadata = parsed_release_from_file_stem(source_video);
    let Some(file_episode) = file_metadata.episode.as_ref() else {
        return Ok(None);
    };
    let [file_episode_number] = file_episode.episode_numbers.as_slice() else {
        return Ok(None);
    };
    let Some(scoped_episode_number) = scoped_episode
        .episode_number
        .as_deref()
        .and_then(|number| number.parse::<u32>().ok())
    else {
        return Ok(None);
    };
    if *file_episode_number != scoped_episode_number
        || !parsed_title_matches_catalog_title(&file_metadata, title)
        || !source_fuzzily_matches_catalog_episode_title(title, source_video, &scoped_episode)
    {
        return Ok(None);
    }

    let Some(release_title) = release_evidence.release_title(None) else {
        return Ok(None);
    };
    let release_metadata = normalize_release_title_signal(parse_import_release_for_title(
        &release_title,
        title,
    ));
    let Some(release_episode) = release_metadata.episode.as_ref() else {
        return Ok(None);
    };
    let release_season = release_episode.season.unwrap_or(1).to_string();
    let release_targets =
        resolve_target_episodes(app, title, release_episode, &release_season).await;
    if release_targets.len() != 1 || release_targets[0].id != scoped_episode.id {
        return Ok(None);
    }

    tracing::debug!(
        file = %source_video.display(),
        title_id = %title.id,
        episode_id = %scoped_episode.id,
        "import: reconciled alternate scene numbering from scoped release evidence"
    );
    Ok(Some(scoped_episode))
}

fn parsed_title_matches_catalog_title(
    parsed: &crate::ParsedReleaseMetadata,
    title: &scryer_domain::Title,
) -> bool {
    let mut expected = Vec::with_capacity(1 + title.aliases.len() + title.tagged_aliases.len());
    expected.push(crate::app_usecase_rss::normalize_for_matching(&title.name));
    expected.extend(
        title
            .aliases
            .iter()
            .map(|alias| crate::app_usecase_rss::normalize_for_matching(alias)),
    );
    expected.extend(
        title
            .tagged_aliases
            .iter()
            .map(|alias| crate::app_usecase_rss::normalize_for_matching(&alias.name)),
    );

    let candidates = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.as_str()]
    } else {
        parsed
            .normalized_title_variants
            .iter()
            .map(String::as_str)
            .collect()
    };
    candidates.into_iter().any(|candidate| {
        let normalized = crate::app_usecase_rss::normalize_for_matching(candidate);
        !normalized.is_empty() && expected.iter().any(|value| value == &normalized)
    })
}

/// Match only a member's episode-title text with typo tolerance. Series title
/// identity is checked separately and always stays exact (canonical or alias).
fn source_fuzzily_matches_catalog_episode_title(
    title: &scryer_domain::Title,
    source_video: &Path,
    episode: &scryer_domain::Episode,
) -> bool {
    let Some(expected_title) = episode.title.as_deref().filter(|value| !value.trim().is_empty())
    else {
        return false;
    };
    let Some(stem) = source_video_stem(Some(source_video)) else {
        return false;
    };
    let expected = crate::app_usecase_rss::normalize_for_matching(expected_title);
    if expected.is_empty() {
        return false;
    }

    let context = crate::build_release_parse_context(title, Some(episode), None, None);
    let analysis = crate::analyze_release_for_target(&stem, &context);
    let Some(candidate) = analysis.best_candidate() else {
        return false;
    };
    if candidate.context_title_matches.iter().any(|context_match| {
        context_match.kind == crate::release_parser::ContextTitleMatchKind::EpisodeTitle
            && !candidate.zones.title_zones.iter().any(|title_zone| {
                context_match.token_range.start_token < title_zone.end_token
                    && title_zone.start_token < context_match.token_range.end_token
            })
    }) {
        return true;
    }

    let unmatched_tokens: Vec<_> = candidate
        .unconsumed_tokens
        .iter()
        .filter_map(|span| stem.get(span.start..span.end))
        .filter(|token| token.chars().any(|character| character.is_alphanumeric()))
        .collect();
    let max_window_tokens = expected_title
        .split_whitespace()
        .count()
        .saturating_add(2)
        .max(1);
    for start in 0..unmatched_tokens.len() {
        let mut phrase = String::new();
        for token in unmatched_tokens
            .iter()
            .skip(start)
            .take(max_window_tokens)
        {
            if !phrase.is_empty() {
                phrase.push(' ');
            }
            phrase.push_str(token);
            let normalized = crate::app_usecase_rss::normalize_for_matching(&phrase);
            if !normalized.is_empty()
                && normalized_episode_title_matches_or_is_near_match(&normalized, &expected)
            {
                return true;
            }
        }
    }
    false
}

fn normalized_episode_title_matches_or_is_near_match(candidate: &str, expected: &str) -> bool {
    if candidate == expected {
        return true;
    }

    let candidate_len = candidate.chars().count();
    let expected_len = expected.chars().count();
    let max_distance = match candidate_len.max(expected_len) {
        0..=5 => 0,
        6..=12 => 1,
        13..=24 => 2,
        _ => 3,
    };
    bounded_levenshtein_distance(candidate, expected, max_distance).is_some()
}

fn bounded_levenshtein_distance(left: &str, right: &str, max_distance: usize) -> Option<usize> {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.len().abs_diff(right.len()) > max_distance {
        return None;
    }

    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (left_index, left_char) in left.iter().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(left_index + 1);
        let mut row_min = left_index + 1;
        for (right_index, right_char) in right.iter().enumerate() {
            let cost = usize::from(left_char != right_char);
            let distance = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + cost);
            row_min = row_min.min(distance);
            current.push(distance);
        }
        if row_min > max_distance {
            return None;
        }
        previous = current;
    }

    previous
        .last()
        .copied()
        .filter(|distance| *distance <= max_distance)
}

#[cfg(test)]
mod alternate_scene_numbering_tests {
    use super::*;

    #[test]
    fn episode_title_match_allows_a_small_typo_but_not_an_unrelated_title() {
        assert!(normalized_episode_title_matches_or_is_near_match(
            "sonofdarkness",
            "sonofdarknes",
        ));
        assert!(!normalized_episode_title_matches_or_is_near_match(
            "sonofdarkness",
            "thenightmareofyou",
        ));
    }
}

fn unresolved_episode_import_message(
    parsed: &crate::ParsedReleaseMetadata,
    source_video: &Path,
    release_evidence: &ReleaseEvidence,
    video_file_count: usize,
) -> String {
    if let Some(absolute_number) = parsed.episode.as_ref().and_then(|episode| {
        episode
            .absolute_episode
            .or_else(|| episode.absolute_episode_numbers.first().copied())
    }) {
        return format!(
            "Automatic import found absolute episode {absolute_number}, but could not map it to a season and episode for this title. Open Manual Import and assign the correct episode."
        );
    }

    ambiguous_obfuscated_episode_message(source_video, release_evidence, video_file_count)
        .unwrap_or_else(|| {
            "Automatic import could not determine a season and episode from the downloaded file. Open Manual Import and assign the correct season and episode."
                .to_string()
        })
}
/// Import a single episode video file: parse, gate, import, and link.
#[expect(
    clippy::too_many_arguments,
    reason = "single-episode imports need the full source, rename, and persistence context together"
)]
async fn import_single_episode_file(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    rename_enabled: bool,
    rename_template: &str,
    season_folder_template: &str,
    specials_folder_template: &str,
    title_folder_path: &Path,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    source_video: &Path,
    quality_profile: &crate::QualityProfile,
    nfo_enabled: bool,
    expected_episode_ids: Option<&HashSet<String>>,
    planned_disposition: Option<&PlannedEpisodeMemberDisposition>,
    video_file_count: usize,
    blocklist_ledger: &mut DownloadBlocklistLedger,
) -> AppResult<EpisodeImportOutcome> {
    // Sonarr's `OtherVideoFiles`: with more than one (non-sample) video in the
    // download, each file must identify itself.
    let other_video_files = video_file_count > 1;
    let parsed = build_augmented_episode_import_metadata_for_title(
        source_video,
        release_evidence,
        title,
        other_video_files,
    );

    let planned_pack_import = matches!(
        planned_disposition,
        Some(PlannedEpisodeMemberDisposition::Import { .. })
    );
    let planned_episodes = match planned_disposition {
        Some(PlannedEpisodeMemberDisposition::Import { episodes }) => Some(episodes.clone()),
        Some(PlannedEpisodeMemberDisposition::Ignore {
            episodes,
            reason_code,
            message,
        }) => {
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "ignored",
                Some(reason_code),
                None,
                episodes,
            )
            .await?;
            return Ok(EpisodeImportOutcome::Ignored {
                message: message.clone(),
                reason_code: (*reason_code).to_string(),
                episode_ids: episodes.iter().map(|episode| episode.id.clone()).collect(),
            });
        }
        Some(PlannedEpisodeMemberDisposition::Hold {
            episodes,
            reason_code,
            message,
        }) => {
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "rejected",
                Some(reason_code),
                None,
                episodes,
            )
            .await?;
            return Ok(EpisodeImportOutcome::Rejected {
                rejection: crate::post_download_gate::ImportedFileRejection {
                    message: message.clone(),
                    recycle_reason: reason_code,
                    skip_reason: Some(ImportSkipReason::PolicyMismatch),
                    blocking_rule_codes: vec![(*reason_code).to_string()],
                },
                disposition: crate::import_decide::RejectionDisposition::Hold,
                reason_code: Some((*reason_code).to_string()),
                episode_ids: episodes.iter().map(|episode| episode.id.clone()).collect(),
            });
        }
        None => None,
    };

    let episode_is_resolvable = |episode: &crate::ParsedEpisodeMetadata| {
        !episode.episode_numbers.is_empty()
            || (episode.absolute_episode.is_some()
                && title.facet == scryer_domain::MediaFacet::Anime)
            || episode.air_date.is_some()
            || episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
    };
    let file_episode = file_episode_identity_for_title(source_video, title);
    let identity_episode = file_episode
        .as_ref()
        .filter(|episode| episode_is_resolvable(episode))
        .or_else(|| {
            parsed
                .episode
                .as_ref()
                .filter(|episode| episode_is_resolvable(episode))
        });
    let (target_episodes, uses_catalog_identity) = if let Some(episodes) = planned_episodes {
        // The verified-pack preflight resolves member identity from catalog
        // episodes. Parsed metadata remains release evidence only.
        (episodes, true)
    } else if let Some(ep_meta) = identity_episode {
        // A file that positively identifies itself wins identity resolution.
        // The expected-scope gate below then holds it when that identity is not
        // part of the grabbed release. Acquisition is only an ambiguity
        // fallback; it never overwrites contradictory file evidence.
        let season = ep_meta.season.unwrap_or(1);
        let season_str = season.to_string();
        let (resolved_episodes, numbering) = resolve_target_episodes_with_numbering(
            app,
            title,
            ep_meta,
            &season_str,
            &parsed.normalized_title_variants,
            file_reference_date(source_video),
        )
        .await;
        // Several readings of an anime release resolve equally well: picking
        // one would file the episode under a numbering the user never asked
        // for, so the file is held for a human instead.
        if let Some(summary) = numbering.ambiguity_summary() {
            let message = format!(
                "release numbering has several equally-good readings for this anime: {summary}"
            );
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "rejected",
                Some(ANIME_NUMBERING_AMBIGUOUS_REASON),
                None,
                &resolved_episodes,
            )
            .await?;
            return Ok(EpisodeImportOutcome::Rejected {
                rejection: crate::post_download_gate::ImportedFileRejection {
                    message,
                    recycle_reason: ANIME_NUMBERING_AMBIGUOUS_REASON,
                    skip_reason: Some(ImportSkipReason::PolicyMismatch),
                    blocking_rule_codes: vec![ANIME_NUMBERING_AMBIGUOUS_REASON.to_string()],
                },
                disposition: crate::import_decide::RejectionDisposition::Hold,
                reason_code: Some(ANIME_NUMBERING_AMBIGUOUS_REASON.to_string()),
                episode_ids: resolved_episodes
                    .iter()
                    .map(|episode| episode.id.clone())
                    .collect(),
            });
        }
        if resolved_episodes.is_empty()
            && let Some(episode) = reconcile_unresolved_scene_episode_from_scoped_release(
                app,
                title,
                release_evidence,
                source_video,
                other_video_files,
            )
            .await?
        {
            (vec![episode], true)
        } else {
            (resolved_episodes, false)
        }
    } else if let Some(episode) =
        grabbed_episode_fallback(app, title, release_evidence, other_video_files).await?
    {
        // Only an ambiguous sole video inherits the exact catalog episode that
        // acquisition admitted. Multi-video downloads never use this fallback.
        (vec![episode], true)
    } else {
        tracing::debug!(
            file = %source_video.display(),
            other_video_files,
            "skipping file with no parseable episode info"
        );
        return Ok(EpisodeImportOutcome::Skipped {
            message: unresolved_episode_import_message(
                &parsed,
                source_video,
                release_evidence,
                video_file_count,
            ),
            reason_code: None,
            skip_reason: Some(ImportSkipReason::UnparseableEpisode),
            episode_ids: Vec::new(),
        });
    };
    let target_episode_ids: Vec<String> = target_episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect();
    // Fail closed: a parseable episodic file that binds to no episode of this
    // title is not part of what was grabbed and must never reach the library.
    // Ordered ahead of the grabbed-release scope check so the reported reason
    // names the missing episode instead of the broader scope violation, and
    // returned before any destination rendering, scoring, media-file insertion,
    // or source cleanup can run.
    if target_episodes.is_empty() {
        // The early return skips the shared outcome handling below, so record
        // the rejected artifact here: the file must still be visible in the
        // import results even though nothing was transferred.
        persist_file_import_artifact(
            app,
            import_id,
            completed,
            title.id.as_str(),
            source_video,
            "episode",
            "rejected",
            Some("episode_not_found_for_title"),
            None,
            &target_episodes,
        )
        .await?;
        return Ok(EpisodeImportOutcome::Rejected {
            rejection: crate::post_download_gate::ImportedFileRejection {
                message: "file resolves to no episode of this title".to_string(),
                recycle_reason: "episode_not_found_for_title",
                skip_reason: Some(ImportSkipReason::PolicyMismatch),
                blocking_rule_codes: vec!["episode_not_found_for_title".to_string()],
            },
            // The file stays in the completed-download directory: leaving the
            // rest of the pack importable is Sonarr-compatible, and burning the
            // release for one stray file would be wrong. An operator decides
            // through Manual Import, so this is a hold rather than a skip.
            disposition: crate::import_decide::RejectionDisposition::Hold,
            reason_code: Some("episode_not_found_for_title".to_string()),
            episode_ids: Vec::new(),
        });
    }
    if !planned_pack_import
        && let Some(expected_episode_ids) = expected_episode_ids
        && !resolved_episode_ids_are_within_expected(&target_episode_ids, expected_episode_ids)
    {
        // The obfuscation explainer describes a season guessed from the
        // release name; a file in a multi-video download identified itself,
        // so it simply resolved outside the grabbed release.
        let obfuscated_message = if other_video_files {
            None
        } else {
            ambiguous_obfuscated_episode_message(source_video, release_evidence, video_file_count)
        };
        persist_file_import_artifact(
            app,
            import_id,
            completed,
            title.id.as_str(),
            source_video,
            "episode",
            "rejected",
            Some("episode_outside_grabbed_release"),
            None,
            &target_episodes,
        )
        .await?;
        return Ok(EpisodeImportOutcome::Rejected {
            rejection: crate::post_download_gate::ImportedFileRejection {
                message: obfuscated_message.unwrap_or_else(|| {
                    "Automatic import resolved the downloaded file to episode(s) outside the grabbed release. Open Manual Import and assign the correct season and episode."
                        .to_string()
                }),
                recycle_reason: "episode_outside_grabbed_release",
                skip_reason: Some(ImportSkipReason::PolicyMismatch),
                blocking_rule_codes: vec!["episode_outside_grabbed_release".to_string()],
            },
            disposition: crate::import_decide::RejectionDisposition::Hold,
            reason_code: Some("episode_outside_grabbed_release".to_string()),
            episode_ids: target_episode_ids.clone(),
        });
    }
    let resolved_episode = target_episodes.first();
    let (season, ep_num_str, abs_str) = if uses_catalog_identity {
        let season = resolved_episode
            .and_then(|episode| episode.season_number.as_deref())
            .map(str::trim)
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1);
        let episode_number = resolved_episode
            .and_then(|episode| episode.episode_number.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_string();
        let absolute_number = resolved_episode.and_then(|episode| episode.absolute_number.clone());
        (season, episode_number, absolute_number)
    } else {
        let ep_meta = identity_episode.expect("the parse path resolved episode metadata");
        let season = ep_meta.season.unwrap_or(1);
        let episode_number = episode_number_token_for_import(
            &ep_meta.episode_numbers,
            resolved_episode.and_then(|episode| episode.episode_number.as_deref()),
        );
        let absolute_number = ep_meta
            .absolute_episode
            .map(|number| number.to_string())
            .or_else(|| resolved_episode.and_then(|episode| episode.absolute_number.clone()));
        (season, episode_number, absolute_number)
    };
    let post_processing_episode = if uses_catalog_identity {
        ep_num_str.parse::<u32>().ok()
    } else {
        identity_episode.and_then(|episode| episode.episode_numbers.first().copied())
    };
    let episode_title = target_episodes.first().and_then(|ep| ep.title.as_deref());
    let import_purpose = release_evidence.purpose();
    let origin = release_evidence.import_origin();
    let additional_import = import_purpose.is_additional_file();
    let runtime_sample_mode = if import_purpose.is_manual_replacement() {
        crate::post_download_gate::RuntimeSampleValidationMode::BypassRuntimeSampleCheck
    } else {
        crate::post_download_gate::RuntimeSampleValidationMode::EnforceAutomatic
    };
    let outcome = execute_resolved_episode_import(
        app,
        actor,
        title,
        import_id,
        Some(completed),
        rename_enabled,
        rename_template,
        season_folder_template,
        specials_folder_template,
        title_folder_path,
        source_video,
        &parsed,
        &target_episodes,
        &target_episodes,
        season,
        &ep_num_str,
        abs_str.as_deref(),
        episode_title,
        quality_profile,
        None,
        runtime_sample_mode,
        origin,
        release_evidence.announced_size_bytes(),
        additional_import,
    )
    .await?;

    match &outcome {
        EpisodeImportOutcome::Imported {
            dest_path,
            imported_media_file_id,
            reason_code,
            blocklist_after_import,
            source_cleanup,
            ..
        } => {
            // Imported, but the release lied about its quality: burn it so the
            // next upgrade search cannot re-grab the same lie. Recorded, not
            // written — one row per download, not one per member.
            if let Some(directive) = blocklist_after_import {
                tracing::info!(
                    title_id = %title.id,
                    code = directive.code,
                    "{}",
                    directive.reason
                );
                blocklist_ledger.record_import_blocklist(
                    &release_evidence
                        .release_title(Some(source_video))
                        .unwrap_or_default(),
                    source_video,
                    &target_episode_ids,
                    directive.reason.clone(),
                );
            }
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "imported",
                reason_code.as_deref(),
                imported_media_file_id.as_deref(),
                &target_episodes,
            )
            .await?;

            finalize_deferred_import_source_cleanup(
                app,
                source_cleanup.as_deref().cloned(),
                &crate::stored_paths::stored_path_to_path_buf(dest_path),
                Some(completed),
            )
            .await?;

            if imported_media_file_id.is_some() && reason_code.as_deref() != Some("additional_file")
            {
                if nfo_enabled {
                    let nfo_path = std::path::Path::new(dest_path).with_extension("nfo");
                    if let Some(episode) = target_episodes.first() {
                        let nfo_content = render_episode_nfo(title, episode);
                        if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await
                        {
                            tracing::warn!(
                                error = %err,
                                path = %nfo_path.display(),
                                "failed to write episode NFO sidecar"
                            );
                        }
                    }
                }

                spawn_post_processing(PostProcessingContext {
                    app: app.clone(),
                    actor: crate::domain_events::DomainEventActor::from(actor),
                    title_id: title.id.clone(),
                    title_name: title.name.clone(),
                    facet: title.facet.clone(),
                    dest_path: PathBuf::from(dest_path),
                    year: title.year,
                    imdb_id: title
                        .external_ids
                        .iter()
                        .find(|e| e.source == "imdb")
                        .map(|e| e.value.clone()),
                    tvdb_id: title
                        .external_ids
                        .iter()
                        .find(|e| e.source == "tvdb")
                        .map(|e| e.value.clone()),
                    season: Some(season),
                    episode: post_processing_episode,
                    quality: parsed.quality.clone(),
                });
            }
        }
        EpisodeImportOutcome::Skipped {
            reason_code,
            skip_reason,
            ..
        } => {
            let artifact_result = if episode_skip_is_already_present(skip_reason.as_ref()) {
                "already_present"
            } else {
                "rejected"
            };
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                artifact_result,
                reason_code.as_deref(),
                None,
                &target_episodes,
            )
            .await?;
        }
        EpisodeImportOutcome::Ignored { reason_code, .. } => {
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "ignored",
                Some(reason_code),
                None,
                &target_episodes,
            )
            .await?;
        }
        EpisodeImportOutcome::Rejected {
            rejection,
            disposition,
            reason_code,
            ..
        } => {
            // Only a release that provably lied is burned, and only once per
            // download. `Skip` and `Hold` record the decision and stop: the
            // download sits in `ImportBlocked` for the operator either way, and
            // reopening a scope whose refusal will repeat is pure churn (D17).
            if matches!(
                disposition,
                crate::import_decide::RejectionDisposition::Blocklist
            ) {
                let source_title = release_evidence
                    .release_title(Some(source_video))
                    .unwrap_or_default();
                blocklist_ledger.record_rejection(
                    &source_title,
                    source_video,
                    &target_episode_ids,
                    rejection,
                );
            }

            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "rejected",
                reason_code
                    .as_deref()
                    .or_else(|| rejection.skip_reason.as_ref().map(ImportSkipReason::as_str)),
                None,
                &target_episodes,
            )
            .await?;
        }
    }

    Ok(outcome)
}

fn episode_number_token_for_import(
    parsed_episode_numbers: &[u32],
    resolved_episode_number: Option<&str>,
) -> String {
    parsed_episode_numbers
        .first()
        .map(ToString::to_string)
        .or_else(|| {
            resolved_episode_number
                .map(str::trim)
                .filter(|number| !number.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod episode_number_token_for_import_tests {
    use super::*;

    #[test]
    fn parsed_regular_episode_number_takes_precedence() {
        assert_eq!(episode_number_token_for_import(&[7], Some("1")), "7");
    }

    #[test]
    fn resolved_episode_number_fills_an_absolute_only_parse() {
        assert_eq!(episode_number_token_for_import(&[], Some("1")), "1");
    }

    #[test]
    fn episode_number_token_stays_empty_without_a_regular_number() {
        assert_eq!(episode_number_token_for_import(&[], Some("  ")), "");
        assert_eq!(episode_number_token_for_import(&[], None), "");
    }

    #[test]
    fn resolved_episode_number_renders_a_padded_destination_token() {
        let episode = episode_number_token_for_import(&[], Some("1"));
        let tokens = BTreeMap::from([
            ("season".to_string(), "1".to_string()),
            ("episode".to_string(), episode),
        ]);

        assert_eq!(
            render_rename_template("S{season:2}E{episode:2}", &tokens),
            "S01E01"
        );
    }
}
/// Resolve media root path and rename template for a title's facet.
pub(crate) async fn resolve_import_paths(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> AppResult<ImportPathSettings> {
    let media_root = app.title_root_folder_path_override(title).await?;

    let rename_enabled = app.resolve_rename_enabled(&title.facet).await?;
    let rename_template = app.resolve_rename_template(&title.facet).await?;
    let folder_template = app
        .read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            super::FOLDER_TEMPLATE_KEY,
            Some(title.facet.as_str()),
        )
        .await?;
    let default_folder_template = match title.facet {
        MediaFacet::Movie => super::DEFAULT_FOLDER_TEMPLATE_MOVIE,
        MediaFacet::Series => super::DEFAULT_FOLDER_TEMPLATE_SERIES,
        MediaFacet::Anime => super::DEFAULT_FOLDER_TEMPLATE_ANIME,
    };
    let folder_template = crate::normalize_title_folder_template_or_default(
        folder_template,
        default_folder_template,
        title.facet.as_str(),
    );
    let season_folder_template = crate::normalize_season_folder_template_or_default(
        app.read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            super::SEASON_FOLDER_TEMPLATE_KEY,
            Some(title.facet.as_str()),
        )
        .await?,
    );
    let specials_folder_template = crate::normalize_specials_folder_template_or_default(
        app.read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            super::SPECIALS_FOLDER_TEMPLATE_KEY,
            Some(title.facet.as_str()),
        )
        .await?,
    );

    Ok(ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template,
        specials_folder_template,
    })
}

/// Compute the parent directory for an episode import: the season or specials
/// folder beneath the title folder, or the title folder itself when the library
/// is not configured to use season folders.
pub(crate) fn episodic_import_parent_path(
    title: &scryer_domain::Title,
    use_season_folders: bool,
    title_folder_path: &Path,
    season_folder_template: &str,
    specials_folder_template: &str,
    season_num: u32,
) -> PathBuf {
    if use_season_folders {
        let season_folder = crate::render_episode_folder_name(
            title,
            season_num,
            season_folder_template,
            specials_folder_template,
        );
        title_folder_path.join(season_folder)
    } else {
        title_folder_path.to_path_buf()
    }
}

/// Return the explicit season-folder title override encoded in legacy tags.
/// The application resolver combines this value with library and facet settings.
pub(crate) fn season_folder_tag_override(title: &scryer_domain::Title) -> Option<bool> {
    title
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix("scryer:season-folder:"))
        .map(|value| !value.trim().eq_ignore_ascii_case("disabled"))
}

/// Legacy title-tag interpretation retained for focused tag parsing tests.
/// Runtime import, scan, and rename paths use `AppUseCase::resolve_use_season_folders`.
#[cfg(test)]
pub(crate) fn use_season_folders(title: &scryer_domain::Title) -> bool {
    season_folder_tag_override(title).unwrap_or(true)
}

/// Compute the destination path for an episode import using the canonical
/// token set: base tokens from parsed release metadata, overridden by the
/// explicit episode values supplied by the caller.
///
/// `ep_num_str` may be empty to leave `{episode}` blank (anime absolute-only
/// files where no per-season episode number is known).
/// `quality_override` replaces the filename-parsed quality token when the
/// caller supplies an explicit label (e.g. manual import).
#[expect(
    clippy::too_many_arguments,
    reason = "episode rename rendering uses the full canonical token set explicitly"
)]
pub(crate) fn episode_import_dest_path(
    title: &scryer_domain::Title,
    use_season_folders: bool,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
    source_path: &Path,
    title_folder_path: &Path,
    rename_enabled: bool,
    rename_template: &str,
    season_folder_template: &str,
    specials_folder_template: &str,
    season_num: u32,
    ep_num_str: &str,
    absolute_number: Option<&str>,
    episode_title: Option<&str>,
    quality_override: Option<&str>,
) -> PathBuf {
    let mut tokens = build_rename_tokens(title, parsed, ext);
    tokens.insert("season".to_string(), season_num.to_string());
    tokens.insert("season_order".to_string(), season_num.to_string());
    tokens.insert("episode".to_string(), ep_num_str.to_string());
    tokens.insert(
        "absolute_episode".to_string(),
        absolute_number.unwrap_or("").to_string(),
    );
    tokens.insert(
        "episode_title".to_string(),
        episode_title.unwrap_or("").to_string(),
    );
    if let Some(q) = quality_override {
        tokens.insert("quality".to_string(), q.to_string());
    }
    let rendered = if rename_enabled {
        render_rename_template(rename_template, &tokens)
    } else {
        preserved_import_filename(source_path)
    };
    episodic_import_parent_path(
        title,
        use_season_folders,
        title_folder_path,
        season_folder_template,
        specials_folder_template,
        season_num,
    )
    .join(rendered)
}
/// Build the common rename token map from parsed release metadata.
pub(crate) fn build_rename_tokens(
    title: &scryer_domain::Title,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::new();
    let fallback_title_year = title.year;
    let resolved_year = parsed.year.or(fallback_title_year);
    tokens.insert("title".to_string(), title.name.clone());
    tokens.insert(
        "year".to_string(),
        resolved_year.map(|y| y.to_string()).unwrap_or_default(),
    );
    tokens.insert(
        "quality".to_string(),
        parsed
            .quality
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
    );
    tokens.insert(
        "source".to_string(),
        parsed
            .source
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    tokens.insert(
        "video_codec".to_string(),
        parsed
            .video_codec
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    tokens.insert(
        "audio".to_string(),
        parsed
            .audio
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    tokens.insert(
        "release_group".to_string(),
        parsed.release_group.clone().unwrap_or_default(),
    );
    tokens.insert(
        "season".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.season)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.episode_numbers.first().copied())
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "absolute_episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.absolute_episode)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert("episode_title".to_string(), String::new());
    tokens.insert("ext".to_string(), ext.to_string());
    tokens
}
/// Resolve a parsed episode block against the catalog, translating community
/// (per-cour) anime numbering into the catalog's own numbering first.
///
/// The translation is inert for a non-anime title and for an anime title with
/// no stored numbering bridge, so every other import keeps today's behaviour.
/// The returned resolution is `Ambiguous` when the release name has several
/// equally-good readings; the caller holds the file rather than picking one.
pub(crate) async fn resolve_target_episodes_with_numbering(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
    parsed_title_variants: &[String],
    reference_date: Option<chrono::NaiveDate>,
) -> (
    Vec<scryer_domain::Episode>,
    crate::anime_numbering::NumberingResolution,
) {
    use crate::anime_numbering::NumberingResolution;

    let literal = || resolve_target_episodes(app, title, ep_meta, season_str);

    if title.facet != scryer_domain::MediaFacet::Anime {
        return (literal().await, NumberingResolution::Unchanged);
    }
    let bridge = match app
        .services
        .catalog
        .shows
        .get_anime_numbering_bridge(&title.id)
        .await
    {
        Ok(bridge) => bridge,
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id = %title.id,
                "import: anime numbering bridge lookup failed; using literal numbering"
            );
            None
        }
    };
    let Some(bridge) = bridge else {
        return (literal().await, NumberingResolution::Unchanged);
    };
    let catalog_episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .unwrap_or_default();

    let mut translated = ep_meta.clone();
    if translated.season.is_none() {
        translated.season = season_str.trim().parse::<u32>().ok();
    }
    let resolution = crate::anime_numbering::translate_parsed_episode_numbering(
        &bridge,
        title,
        &catalog_episodes,
        &mut translated,
        parsed_title_variants,
        reference_date,
    );

    match &resolution {
        NumberingResolution::Resolved(candidate) => {
            // The winning candidate was validated against these very episodes,
            // so its ids need no second lookup.
            let episodes: Vec<_> = candidate
                .episode_ids
                .iter()
                .filter_map(|episode_id| {
                    catalog_episodes
                        .iter()
                        .find(|episode| &episode.id == episode_id)
                        .cloned()
                })
                .collect();
            if episodes.is_empty() {
                return (literal().await, NumberingResolution::Unchanged);
            }
            tracing::debug!(
                title_id = %title.id,
                season = candidate.season,
                kind = candidate.kind.as_str(),
                "import: translated community anime numbering"
            );
            (episodes, resolution)
        }
        // An ambiguous release still reports what the literal reading found so
        // the hold message can name it; the caller never imports it.
        NumberingResolution::Ambiguous(_) | NumberingResolution::Unchanged => {
            (literal().await, resolution)
        }
    }
}

pub(crate) async fn resolve_target_episodes(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
) -> Vec<scryer_domain::Episode> {
    let mut episodes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let target_season = crate::parsed_episode_lookup_season(ep_meta, season_str);

    if let Some(air_date) = ep_meta.air_date {
        let air_date_str = air_date.format("%Y-%m-%d").to_string();
        match app
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(collections) => {
                let mut matches = Vec::new();
                for collection in collections {
                    match app
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(&collection.id)
                        .await
                    {
                        Ok(collection_episodes) => {
                            matches.extend(collection_episodes.into_iter().filter(|episode| {
                                episode.title_id == title.id
                                    && episode.air_date.as_deref() == Some(air_date_str.as_str())
                            }));
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "daily episode lookup failed during import")
                        }
                    }
                }

                matches.sort_by_key(|episode| {
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                });

                if let Some(part) = ep_meta.daily_part {
                    let part_index = part.saturating_sub(1) as usize;
                    if let Some(episode) = matches.into_iter().nth(part_index)
                        && seen.insert(episode.id.clone())
                    {
                        episodes.push(episode);
                    }
                } else {
                    for episode in matches {
                        if seen.insert(episode.id.clone()) {
                            episodes.push(episode);
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "daily collection lookup failed during import")
            }
        }
    }

    for episode_number in &ep_meta.episode_numbers {
        let episode_str = episode_number.to_string();
        match app
            .services
            .catalog
            .shows
            .find_episode_by_title_and_numbers(&title.id, &target_season, &episode_str)
            .await
        {
            Ok(Some(episode)) => {
                if seen.insert(episode.id.clone()) {
                    episodes.push(episode);
                }
            }
            Ok(None) => {
                tracing::debug!(
                    title_id = %title.id,
                    season = %season_str,
                    episode = %episode_str,
                    "no matching episode found for imported file"
                );
            }
            Err(err) => tracing::warn!(error = %err, "episode lookup failed during import"),
        }
    }

    if episodes.is_empty()
        && ep_meta.season.is_some()
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
    {
        match app
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(collections) => {
                for collection in collections
                    .into_iter()
                    .filter(|collection| collection.collection_index == target_season)
                {
                    match app
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(&collection.id)
                        .await
                    {
                        Ok(collection_episodes) => {
                            let mut collection_episodes: Vec<_> = collection_episodes
                                .into_iter()
                                .filter(|episode| {
                                    episode.title_id == title.id
                                        && episode.season_number.as_deref()
                                            == Some(target_season.as_str())
                                })
                                .collect();
                            collection_episodes.sort_by_key(|episode| {
                                episode
                                    .episode_number
                                    .as_deref()
                                    .and_then(|value| value.parse::<u32>().ok())
                                    .unwrap_or(u32::MAX)
                            });
                            for episode in collection_episodes {
                                if seen.insert(episode.id.clone()) {
                                    episodes.push(episode);
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "season episode lookup failed during import")
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "season collection lookup failed during import")
            }
        }
    }

    if episodes.is_empty() && !ep_meta.special_absolute_episode_numbers.is_empty() {
        for special_number in &ep_meta.special_absolute_episode_numbers {
            let episode_str = special_number.to_string();
            match app
                .services
                .catalog
                .shows
                .find_episode_by_title_and_numbers(&title.id, "0", &episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        special = %episode_str,
                        "no matching special episode found during import"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "special episode lookup failed during import")
                }
            }
        }
    }

    if episodes.is_empty()
        && (ep_meta.absolute_episode.is_some() || !ep_meta.absolute_episode_numbers.is_empty())
    {
        let absolute_numbers: Vec<u32> = if !ep_meta.absolute_episode_numbers.is_empty() {
            ep_meta.absolute_episode_numbers.clone()
        } else if ep_meta.episode_numbers.is_empty() {
            vec![ep_meta.absolute_episode.unwrap_or_default()]
        } else {
            ep_meta.episode_numbers.clone()
        };

        for absolute_number in absolute_numbers {
            let absolute_episode_str = absolute_number.to_string();
            match app
                .services
                .catalog
                .shows
                .find_episode_by_title_and_absolute_number(&title.id, &absolute_episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        absolute = absolute_number,
                        "no matching episode found by absolute number"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "episode absolute lookup failed during import")
                }
            }
        }
    }

    episodes
}
async fn write_series_sidecars(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    title_folder_path: &Path,
    nfo_enabled: bool,
) {
    if nfo_enabled {
        let tvshow_nfo_path = title_folder_path.join("tvshow.nfo");
        if !tvshow_nfo_path.exists() {
            if let Some(parent) = tvshow_nfo_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let nfo_content = render_tvshow_nfo(title);
            if let Err(err) = tokio::fs::write(&tvshow_nfo_path, nfo_content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %tvshow_nfo_path.display(),
                    "failed to write tvshow NFO sidecar"
                );
            }
        }
    }

    let plexmatch_enabled = match app
        .resolve_plexmatch_write_on_import(Some(&title.library_id), &title.facet)
        .await
    {
        Ok(value) => value.unwrap_or(false),
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id = %title.id,
                "failed to resolve plexmatch sidecar setting"
            );
            false
        }
    };
    if plexmatch_enabled {
        let plexmatch_path = title_folder_path.join(".plexmatch");
        if !plexmatch_path.exists() {
            if let Some(parent) = plexmatch_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let content = render_plexmatch(title);
            if let Err(err) = tokio::fs::write(&plexmatch_path, content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %plexmatch_path.display(),
                    "failed to write .plexmatch hint file"
                );
            }
        }
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "import artifact persistence records the full import outcome for later inspection"
)]
async fn persist_file_import_artifact(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
    title_id: &str,
    source_path: &Path,
    media_kind: &str,
    result: &str,
    reason_code: Option<&str>,
    imported_media_file_id: Option<&str>,
    episodes: &[scryer_domain::Episode],
) -> AppResult<()> {
    let relative_path = source_path
        .strip_prefix(&completed.dest_dir)
        .ok()
        .map(path_to_stored_string)
        .filter(|path| !path.is_empty());
    let normalized_file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
        .unwrap_or_else(|| source_path.to_string_lossy().to_ascii_lowercase());
    let canonical_download_id = match app
        .services
        .workflow
        .imports
        .canonical_download_id_for_import(import_id)
        .await
    {
        Ok(canonical_download_id) => canonical_download_id,
        Err(error) => {
            tracing::warn!(
                error = %error,
                import_id,
                source_ref = %completed.download_client_item_id,
                "failed to resolve canonical identity for import artifact; retaining legacy artifact write"
            );
            None
        }
    };

    let episode_rows: Vec<(Option<String>, Option<i32>, Option<i32>)> = if episodes.is_empty() {
        vec![(None, None, None)]
    } else {
        episodes
            .iter()
            .map(|episode| {
                (
                    Some(episode.id.clone()),
                    episode
                        .season_number
                        .as_deref()
                        .and_then(|value| value.parse().ok()),
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse().ok()),
                )
            })
            .collect()
    };

    let source_identity = ClientJobLocator::for_import_artifact(
        Some(completed.client_id.as_str()),
        &completed.client_type,
        &completed.download_client_item_id,
    );
    let artifacts = episode_rows
        .into_iter()
        .map(
            |(episode_id, season_number, episode_number)| ImportArtifact {
                id: Id::new().0,
                source_client_id: source_identity.client_id.clone(),
                source_system: source_identity.client_type.clone(),
                source_ref: source_identity.item_id.clone(),
                import_id: Some(import_id.to_string()),
                relative_path: relative_path.clone(),
                normalized_file_name: normalized_file_name.clone(),
                media_kind: media_kind.to_string(),
                title_id: Some(title_id.to_string()),
                episode_id,
                season_number,
                episode_number,
                result: result.to_string(),
                reason_code: reason_code.map(str::to_string),
                imported_media_file_id: imported_media_file_id.map(str::to_string),
                created_at: Utc::now(),
            },
        )
        .collect();
    if let Err(error) = app
        .services
        .workflow
        .import_artifacts
        .insert_artifacts_for_download(artifacts, canonical_download_id.as_ref())
        .await
    {
        tracing::warn!(
            error = %error,
            import_id,
            source_ref = %completed.download_client_item_id,
            file = %source_path.display(),
            "failed to persist import artifacts"
        );
        return Err(AppError::ImportEvidenceUnavailable(error.to_string()));
    }
    Ok(())
}
// 50 MB

/// Name-only sample detection: the file stem contains "sample"
/// (case-insensitive). Unlike `is_sample_file` this carries no size heuristic,
/// so a legitimately small movie (short film, old cartoon, low-bitrate SD) is
/// never mistaken for a sample; the automatic movie path never size-filters,
/// and manual import must not be stricter than it.
pub(crate) fn is_sample_named_file(path: &Path) -> bool {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|stem| stem.contains("sample"))
}

pub(crate) fn is_sample_file(path: &Path) -> bool {
    if is_sample_named_file(path) {
        return true;
    }

    if scryer_domain::canonical_video_extension(path) == Some("strm") {
        return false;
    }

    // Small files in multi-episode directories are almost certainly samples/promos
    std::fs::metadata(path)
        .map(|m| m.len() < SAMPLE_SIZE_THRESHOLD)
        .unwrap_or(false)
}
fn resolve_title_from_release_candidate(
    titles: &[Title],
    candidate: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<Title> {
    if candidate.episode.is_some() {
        crate::import_title_resolution::resolve_monitored_episode_title_from_release(
            titles, candidate, facet_hint,
        )
        .map(|resolved| resolved.title.clone())
    } else {
        crate::import_title_resolution::resolve_monitored_movie_title_from_release(
            titles, candidate,
        )
        .map(|resolved| resolved.title.clone())
    }
}
/// Canonical import-time release metadata for an episode file: the release
/// evidence parsed with the title's canonical grab-time context (see
/// `parse_import_release_for_title`) supplies every score-bearing fact; the
/// episode identity follows Sonarr's `OtherVideoFiles` rule
/// (`AggregateEpisodes.GetBestEpisodeInfo`).
///
/// The release name's numbering is applied to a file only when that file is
/// the download's sole video and the release names concrete episodes. When
/// the download holds other video files, or the release is a season pack
/// (whole or partial — it has no episode numbers to hand out), every file must
/// identify itself from its own name; a file that cannot gets no episode at
/// all, so the caller parks it for manual import instead of guessing.
fn build_augmented_episode_import_metadata_for_title(
    source_video: &Path,
    release_evidence: &ReleaseEvidence,
    title: &scryer_domain::Title,
    other_video_files: bool,
) -> ParsedReleaseMetadata {
    let Some(release_title) = release_evidence.release_title(Some(source_video)) else {
        return ParsedReleaseMetadata::default();
    };

    let mut parsed =
        normalize_release_title_signal(parse_import_release_for_title(&release_title, title));
    // The title-anchored parse keeps score-bearing facts but drops the release
    // name's own numbering when that name does not match the title's canonical
    // identity (a user-assigned or parameter-matched download); the release
    // name's context-free numbering is still what the release claims.
    let release_episode = parsed
        .episode
        .take()
        .or_else(|| parse_release_metadata(&release_title).episode);
    let release_is_season_pack = release_episode.as_ref().is_some_and(|episode| {
        episode.full_season || episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
    });
    parsed.episode = if other_video_files || release_is_season_pack {
        file_episode_identity_for_title(source_video, title)
    } else if let Some(scene_episode) = scene_titled_file_episode(source_video) {
        // Sonarr's `!SceneChecker.IsSceneTitle(fileName)` guard: a sole video
        // that is itself a properly named scene release (dotted, grouped,
        // quality-tagged, episode-numbered) identifies itself; the release
        // name's numbering is not applied over it. A disagreement with the
        // grabbed release then surfaces through the grabbed-release gate
        // rather than being papered over.
        Some(scene_episode)
    } else {
        // Sole video of a non-pack release: the release name is the best
        // episode evidence, and only after it the file name — which may locate
        // an episode but cannot supplement score-bearing release metadata.
        release_episode.or_else(|| file_episode_identity_for_title(source_video, title))
    };
    parsed
}

/// The episode a sole video names when its stem is a scene-style release name
/// (Sonarr `SceneChecker.IsSceneTitle`): dotted, no spaces, and a context-free
/// parse yields a release group, a quality, a title, and episode numbering.
/// Anything less (obfuscated, renamed, "episode 2.mkv") is not scene-titled
/// and does not override the release name.
fn scene_titled_file_episode(source_video: &Path) -> Option<crate::ParsedEpisodeMetadata> {
    let stem = source_video_stem(Some(source_video))?;
    if !stem.contains('.') || stem.contains(' ') {
        return None;
    }
    let parsed = normalize_release_title_signal(parse_release_metadata(&stem));
    if parsed
        .release_group
        .as_deref()
        .is_none_or(|group| group.trim().is_empty())
        || parsed.quality.is_none()
        || parsed.normalized_title.trim().is_empty()
    {
        return None;
    }
    parsed.episode
}

/// Reason code for a file whose anime numbering has more than one equally-good
/// reading. It is a hold, never a discard: the file is fine, only Scryer's
/// reading of its numbering is undecided.
pub(crate) const ANIME_NUMBERING_AMBIGUOUS_REASON: &str = "anime_numbering_ambiguous";

/// The file's own modified date, used only to break an otherwise dead-even tie
/// between two anime numbering readings. Absent when the file system has no
/// usable timestamp, in which case the tie simply stands.
fn file_reference_date(source_video: &Path) -> Option<chrono::NaiveDate> {
    let modified = std::fs::metadata(source_video).and_then(|meta| meta.modified()).ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(modified).date_naive())
}

/// The episode a video file names on its own: its stem parsed with the
/// title's canonical context (so absolute/anime numbering resolves the way
/// the grab path resolves it), then the context-free stem parse the manual
/// preview and obfuscation checks use.
fn file_episode_identity_for_title(
    source_video: &Path,
    title: &scryer_domain::Title,
) -> Option<crate::ParsedEpisodeMetadata> {
    source_video_stem(Some(source_video))
        .and_then(|stem| parse_import_release_for_title(&stem, title).episode)
        .or_else(|| parsed_release_from_file_stem(source_video).episode)
}
