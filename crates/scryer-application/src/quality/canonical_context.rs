//! Resolving the inputs canonical scoring needs.
//!
//! [`crate::canonical_scoring::score_release`] is pure and synchronous so its
//! invariants can be property-tested. Everything it needs that lives in the
//! database — the quality profile, the resolved persona and weights, the
//! required-language set, the library name, the active rule engine — is
//! gathered here once and then handed to it by reference.
//!
//! Resolving once per title also means the grab path and the import path score
//! against *identical* context, which is the other half of making their verdicts
//! agree.

use crate::canonical_scoring::ScoringContext;
use crate::quality_profile::CoverageSizeBasis;
use crate::scoring_weights::{ScoringPersona, ScoringWeights};
use crate::{AppUseCase, QualityProfile};
use scryer_domain::Title;

/// Owned scoring inputs for one title. Borrow a [`ScoringContext`] from it with
/// [`ResolvedScoringContext::view`].
pub(crate) struct ResolvedScoringContext {
    profile: QualityProfile,
    weights: ScoringWeights,
    required_audio_languages: Vec<String>,
    category: String,
    title_id: String,
    library_name: Option<String>,
    original_language: Option<String>,
    original_country: Option<String>,
    title_tags: Vec<String>,
    rules: scryer_rules::UserRulesEngine,
    default_runtime_minutes: Option<i32>,
}

impl ResolvedScoringContext {
    /// Borrow scoring inputs.
    ///
    /// `size_basis` overrides the title's own runtime for scopes that know
    /// better (an episode's length rather than the series average, a pack's
    /// total and per-member runtimes); pass
    /// [`CoverageSizeBasis::default`] to keep the title default as a
    /// single-member basis. Size scoring is runtime-derived, so getting this
    /// wrong moves the size bucket.
    pub(crate) fn view(
        &self,
        size_basis: CoverageSizeBasis,
        is_filler: bool,
    ) -> ScoringContext<'_> {
        ScoringContext {
            profile: &self.profile,
            weights: &self.weights,
            required_audio_languages: &self.required_audio_languages,
            category: &self.category,
            size_basis: size_basis.or_runtime(self.default_runtime_minutes),
            rules: (!self.rules.is_empty()).then_some(&self.rules),
            title_id: Some(&self.title_id),
            library_name: self.library_name.as_deref(),
            original_language: self.original_language.as_deref(),
            original_country: self.original_country.as_deref(),
            title_tags: &self.title_tags,
            is_filler,
        }
    }

    pub(crate) fn profile(&self) -> &QualityProfile {
        &self.profile
    }

    pub(crate) fn required_audio_languages(&self) -> &[String] {
        &self.required_audio_languages
    }

    /// The title's own runtime, used as the per-episode fallback when the
    /// catalog has no duration for an episode.
    pub(crate) fn default_runtime_minutes(&self) -> Option<i32> {
        self.default_runtime_minutes
    }
}

/// The announced-evidence parse for a release against one title.
///
/// The one place `languages_audio` is enriched, so grab, pending and import
/// cannot disagree about what a release claims to carry. Most release names say
/// nothing about audio; `release_audio_language_hints_for_title` infers the
/// title's original language when a profile requires one, and a lane that skips
/// the enrichment sees an empty language list and raises
/// `required_audio_language_missing` against a release that is perfectly fine.
/// That was the asymmetry between the grab side (enriched at
/// `discovery.rs`) and the import side (not enriched at all).
///
/// The audio context is keyed on the **title's** facet, not the search category:
/// a search collapses anime movies and series-movie links to `"movie"`, which
/// hides their anime origin and breaks dual-audio inference (eng+jpn).
pub(crate) fn announced_metadata_for_title(
    title: &Title,
    parsed: &crate::ParsedReleaseMetadata,
    required_audio_languages: &[String],
    indexer_languages: Option<&[String]>,
) -> crate::ParsedReleaseMetadata {
    let language_context = crate::title_audio_language_context(
        title.language.as_deref(),
        title.country.as_deref(),
        Some(title.facet.as_str()),
        &title.tags,
    );
    let mut enriched = parsed.clone();
    enriched.languages_audio = crate::release_audio_language_hints_for_title(
        parsed,
        indexer_languages,
        Some(&language_context),
        !required_audio_languages.is_empty(),
    );
    enriched
}

/// Everything a lane needs about a release it only knows by name.
///
/// The three lanes that judge a release from a stored title rather than from a
/// live search result — a parked pending release (D13/D20), a pending group
/// being ordered (BL3), and an in-flight submission read as a pseudo-incumbent
/// (D18) — all need the same derivation, and it is the derivation that has to
/// agree with the search lane or the whole design is undone.
pub(crate) struct ParkedReleaseFacts {
    /// The D4 runtime basis for what the release covers: total runtime, one
    /// member's, and how many members.
    pub size_basis: CoverageSizeBasis,
    /// The release's **block-free** score ([`crate::canonical_scoring::ScoredRelease::total`]):
    /// the number an incumbent's bar is built from, so a queued release the
    /// profile now vetoes still compares on honest terms instead of carrying
    /// `BLOCK_SCORE` into the ladder (I5). Whether the profile allows it at all
    /// is `allowed`, and the lanes that must refuse a vetoed release read that
    /// first.
    pub score: i32,
    pub tier_index: Option<usize>,
    pub revision: i32,
    /// Whether the *current* profile still allows it. A profile edit while a
    /// release waited can veto it (D20).
    pub allowed: bool,
    pub block_codes: Vec<String>,
}

/// Score a release Scryer knows only as a title and a size.
///
/// Pure and synchronous: the caller supplies the catalog rows and the resolved
/// scoring context, so a lane that already has them (the pending processor, the
/// RSS batch) pays for one parse and one term pipeline per release and nothing
/// else. Passing an empty catalog is legitimate for a caller that only needs the
/// tier, the revision and the score — the parse degrades to numbering-only,
/// which none of those depend on.
pub(crate) fn score_parked_release_title(
    title: &Title,
    release_title: &str,
    size_bytes: Option<i64>,
    catalog_episodes: &[scryer_domain::Episode],
    catalog_collections: &[scryer_domain::Collection],
    context: &ResolvedScoringContext,
) -> ParkedReleaseFacts {
    let parse_context = crate::release_parser::build_release_parse_context_for_title(
        title,
        catalog_episodes,
        Some(title.facet.as_str()),
    );
    let raw_parsed =
        crate::release_parser::parse_release_metadata_for_target(release_title, &parse_context);
    let coverage = crate::acquisition_coverage::resolve_release_coverage(
        &raw_parsed,
        catalog_episodes,
        catalog_collections,
        None,
    );
    let parsed =
        announced_metadata_for_title(title, &raw_parsed, context.required_audio_languages(), None);
    let size_basis = crate::acquisition_coverage::coverage_size_basis(
        &coverage,
        &parsed,
        catalog_episodes,
        context.default_runtime_minutes(),
    );
    let tier_index = crate::quality_profile::quality_tier_index(
        &context.profile().criteria,
        parsed.quality.as_deref(),
    );
    let scored = crate::canonical_scoring::score_release(
        &crate::canonical_scoring::ReleaseEvidence::announced(parsed, size_bytes),
        &context.view(size_basis, false),
    );

    ParkedReleaseFacts {
        size_basis,
        // Block-free, like an incumbent's bar: a queued release the profile
        // now vetoes must not carry −10 000 into `queued_rejection`, where it
        // would lose to every candidate and quietly switch the queue gate off.
        score: scored.total,
        tier_index,
        revision: scored.revision,
        allowed: scored.announced_decision.allowed,
        block_codes: scored.announced_decision.block_codes.clone(),
    }
}

/// Who is asking for a subject, and therefore what a multi-episode scope means.
///
/// Exactly one scope shape reads differently on the two sides, and it is the
/// one that caused the defect: [`crate::SubmissionScope::EpisodeSet`].
///
/// - At **grab** an `EpisodeSet` is a *candidate's coverage* — a batch release
///   that will arrive as one file per episode, each gated again on its own at
///   import. So it is judged per member: worth fetching when any monitored
///   member it covers is missing or improvable, refused when a member it covers
///   has not aired. That is D8's "one pack gate", and it applies to both pack
///   shapes (a full season is a `Collection`, a batch is an `EpisodeSet`) — a
///   batch that fills four missing episodes must not be refused because the
///   fifth already holds a better file.
/// - At **import** an `EpisodeSet` is *one landed file* spanning several
///   episodes. It has to beat everything it displaces, or the episodes it does
///   not improve are silently downgraded. Per-member semantics there would be
///   data loss.
///
/// **The bounded I4 exception.** Grab is normally at least as strict as import,
/// and here it is not: a double-episode release admitted per member can be
/// refused as a span at import. That is deliberate and it terminates — the
/// import records a `Skip` (D17: no blocklist, no reopen, the download stays
/// ImportBlocked for the operator) and D18's queue-aware admission stops the
/// scope re-grabbing the same release while it sits there. The alternative,
/// span semantics at grab, is the defect this exception exists to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubjectIntent {
    Grab,
    Import,
}

/// The monitored members of a pack scope, and how many of them cannot exist yet.
///
/// **Monitored only.** An unmonitored episode is not wanted: counting it as a
/// missing member made every partially-monitored season admit any pack, and
/// counting it as an occupied one would let an episode nobody asked for veto a
/// pack that fills the ones they did. It is simply not part of the scope.
///
/// One clock reading for the whole scope, so two members cannot be judged
/// against different "now"s. An id the catalog does not know is kept as a
/// monitored, aired member — dropping it would silently shrink the scope.
fn monitored_pack_members(
    episode_ids: &[String],
    catalog: &[scryer_domain::Episode],
) -> (Vec<String>, usize) {
    let now = chrono::Utc::now();
    let mut members = Vec::with_capacity(episode_ids.len());
    let mut unaired = 0usize;
    for episode_id in episode_ids {
        let Some(episode) = catalog.iter().find(|episode| &episode.id == episode_id) else {
            members.push(episode_id.clone());
            continue;
        };
        if !episode.monitored {
            continue;
        }
        if crate::acquisition_policy::episode_is_unaired(episode.air_date.as_deref(), &now) {
            unaired += 1;
        }
        members.push(episode.id.clone());
    }
    (members, unaired)
}

/// The episodes a file is the **primary** occupant of.
///
/// Not its whole `episode_ids` span. A file can be primary for E02 and merely
/// additional for E03 — a second copy of E03 that some other file already holds
/// — and counting E03 as covered makes a season whose E03 has no primary file
/// read as full, so the pack that would fill it is refused. `covers` feeds
/// exactly two questions, "which members are still empty" and "would replacing
/// this file drop coverage", and the honest answer to both is the primary span.
fn primary_span(file: &crate::EpisodeScopedMediaFile) -> Vec<String> {
    file.primary_episode_ids.clone()
}

impl AppUseCase {
    /// Gather everything canonical scoring needs for one title.
    ///
    /// The caller supplies the profile because profile resolution is strict and
    /// its failure modes (a dangling reference needs operator action; a
    /// transient store error should be retried) belong to the caller's flow
    /// rather than to scoring.
    pub(crate) async fn resolve_canonical_scoring_context(
        &self,
        title: &Title,
        profile: &QualityProfile,
    ) -> ResolvedScoringContext {
        let category = crate::post_download_gate::facet_to_category_hint(&title.facet).to_string();

        let required_audio_languages = self
            .resolve_required_audio_languages_for_title(title)
            .await
            .unwrap_or_default();

        let persona: ScoringPersona = self
            .resolve_scoring_persona(Some(title.library_id.as_str()), Some(category.as_str()))
            .await
            .unwrap_or_default();

        let weights = crate::scoring_weights::build_weights_for_category(
            &persona,
            &profile.criteria.scoring_overrides,
            Some(category.as_str()),
        );

        let library_name = match self
            .services
            .catalog
            .libraries
            .get_by_id(&title.library_id)
            .await
        {
            Ok(Some(library)) => Some(library.name),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    library_id = %title.library_id,
                    "canonical scoring: library name unresolved; rules see no library"
                );
                None
            }
        };

        let rules = self
            .services
            .customization
            .user_rules
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());

        ResolvedScoringContext {
            profile: profile.clone(),
            weights,
            required_audio_languages,
            category,
            title_id: title.id.clone(),
            library_name,
            original_language: title.language.clone(),
            original_country: title.country.clone(),
            title_tags: title.tags.clone(),
            rules,
            default_runtime_minutes: title.runtime_minutes,
        }
    }

    /// The primary files occupying a submission scope, each carrying a canonical
    /// bar, ready for [`crate::admission::evaluate_admission`].
    ///
    /// Episode scopes resolve through the file-episode link table — the same
    /// lookup the import gate uses. The grab path used to read a scalar
    /// `episode_id` off a title-wide listing instead, which is part of how the
    /// two gates ended up disagreeing about what was even in the way.
    pub(crate) async fn admission_subject_for_scope(
        &self,
        title: &Title,
        scope: &crate::SubmissionScope,
        context: &ResolvedScoringContext,
        runtime_minutes: Option<i32>,
        intent: SubjectIntent,
    ) -> crate::admission::AdmissionSubject {
        use crate::SubmissionScope;
        use crate::admission::{AdmissionScope, AdmissionSubject, Incumbent};

        // **One runtime basis per scope** (D4). Size scoring is runtime-derived,
        // so an incumbent's bar has to be computed against the length of what
        // *that file* holds — a double-length premiere, a 7-minute special, a
        // two-episode file — not the series average. Fetched once for the whole
        // subject; title and link scopes have no episodes and keep the caller's
        // runtime (the movie's).
        let episodes: Vec<scryer_domain::Episode> = match scope {
            SubmissionScope::Episode { .. }
            | SubmissionScope::EpisodeSet { .. }
            | SubmissionScope::Collection { .. } => self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await
                .unwrap_or_default(),
            SubmissionScope::Title
            | SubmissionScope::SeriesMovie { .. }
            | SubmissionScope::Orphan => Vec::new(),
        };

        let to_incumbent = |file: &crate::TitleMediaFile, covers: Vec<String>| {
            // The per-episode default is the **title's** runtime, never the
            // candidate's. `runtime_minutes` is the span the candidate covers —
            // a two-episode release passes ~90 minutes — and using it as the
            // fallback for an incumbent whose episodes carry no duration would
            // score a single-episode file as if it were twice as long, moving
            // its size term and therefore its bar. The candidate's span is only
            // a last resort, for a scope with no episodes at all (title and
            // link scopes, where it *is* the movie's runtime).
            let incumbent_basis = crate::acquisition_coverage::episode_span_size_basis(
                &episodes,
                &covers,
                context.default_runtime_minutes(),
            )
            .or_runtime(runtime_minutes);
            let bar = self.incumbent_bar(file, context, incumbent_basis);
            (
                Incumbent {
                    tier_index: bar.tier_index,
                    revision: bar.revision,
                    file_id: file.id.clone(),
                    file_path: file.file_path.clone(),
                    release_group: file
                        .release_group
                        .as_deref()
                        .map(str::trim)
                        .filter(|group| !group.is_empty())
                        .map(str::to_string),
                    score: bar.score,
                    covers,
                    created_at: file.created_at.clone(),
                },
                file.role.is_primary(),
            )
        };

        match scope {
            SubmissionScope::Episode { .. } | SubmissionScope::EpisodeSet { .. } => {
                let episode_ids = match scope {
                    SubmissionScope::Episode { episode_id } => vec![episode_id.clone()],
                    SubmissionScope::EpisodeSet { episode_ids } => episode_ids.clone(),
                    _ => unreachable!("outer match restricts this arm to episode scopes"),
                };
                // A batch being *grabbed* is a pack: one file per episode, each
                // gated again at import. See [`SubjectIntent`] for why the same
                // scope shape is span-scoped at import.
                let is_pack_grab = intent == SubjectIntent::Grab
                    && matches!(scope, SubmissionScope::EpisodeSet { .. });
                let (episode_ids, unaired_members) = if is_pack_grab {
                    monitored_pack_members(&episode_ids, &episodes)
                } else {
                    (episode_ids, 0)
                };
                let incumbents = self
                    .services
                    .library
                    .media_files
                    .list_live_media_files_for_episode_ids(&title.id, &episode_ids)
                    .await
                    .unwrap_or_default();

                let subject = AdmissionSubject::new(
                    AdmissionScope::Episodes(episode_ids),
                    incumbents.iter().map(|incumbent| {
                        to_incumbent(&incumbent.media_file, primary_span(incumbent))
                    }),
                );
                if is_pack_grab {
                    subject.per_member().with_unaired_members(unaired_members)
                } else {
                    subject
                }
            }
            SubmissionScope::SeriesMovie {
                series_movie_link_id,
            } => {
                let files = self
                    .services
                    .library
                    .media_files
                    .list_media_files_for_title(&title.id)
                    .await
                    .unwrap_or_default();
                AdmissionSubject::new(
                    AdmissionScope::SeriesMovieLink(series_movie_link_id.clone()),
                    files
                        .iter()
                        .filter(|file| {
                            file.series_movie_link_ids
                                .iter()
                                .any(|link_id| link_id == series_movie_link_id)
                        })
                        .map(|file| to_incumbent(file, Vec::new())),
                )
            }
            SubmissionScope::Title => {
                let files = self
                    .services
                    .library
                    .media_files
                    .list_media_files_for_title(&title.id)
                    .await
                    .unwrap_or_default();
                AdmissionSubject::new(
                    AdmissionScope::Title,
                    files
                        .iter()
                        // A title-scoped file is one bound to neither an episode
                        // nor a series-movie link.
                        .filter(|file| {
                            file.episode_id.is_none() && file.series_movie_link_ids.is_empty()
                        })
                        .map(|file| to_incumbent(file, Vec::new())),
                )
            }
            // A season pack is its member episodes. Returning an empty subject
            // here made every pack look like a first grab, so a season whose
            // episodes were all occupied by equal-or-better files was still
            // queued — then refused member by member at import, at pack
            // bandwidth. Sonarr resolves the pack to its episodes and checks the
            // file on disk for each; so do we.
            //
            // Members and the unaired count come from `monitored_pack_members`,
            // shared with the batch shape so the two pack gates cannot drift.
            SubmissionScope::Collection { collection_id } => {
                let members: Vec<scryer_domain::Episode> = self
                    .services
                    .catalog
                    .shows
                    .list_episodes_for_collection(collection_id)
                    .await
                    .unwrap_or_default();
                let all_member_ids: Vec<String> =
                    members.iter().map(|episode| episode.id.clone()).collect();
                let (episode_ids, unaired_members) =
                    monitored_pack_members(&all_member_ids, &members);

                if episode_ids.is_empty() {
                    // Nothing resolved, or nothing monitored. A plain empty
                    // subject would be neither per-member nor occupied, which
                    // `evaluate_admission` reads as "unoccupied, admit" — so a
                    // season nobody monitors would accept any pack. There is
                    // nothing here to fill, so it is a refusal. The pack-lane
                    // entry points normally return before this (they anchor on a
                    // monitored member), but a stale wanted row surviving an
                    // unmonitor reaches it.
                    return AdmissionSubject::new(AdmissionScope::Episodes(Vec::new()), [])
                        .per_member();
                }

                let incumbents = self
                    .services
                    .library
                    .media_files
                    .list_live_media_files_for_episode_ids(&title.id, &episode_ids)
                    .await
                    .unwrap_or_default();

                AdmissionSubject::new(
                    AdmissionScope::Episodes(episode_ids),
                    incumbents.iter().map(|incumbent| {
                        to_incumbent(&incumbent.media_file, primary_span(incumbent))
                    }),
                )
                .per_member()
                .with_unaired_members(unaired_members)
            }
            // An orphan grab has no scope to occupy.
            SubmissionScope::Orphan => AdmissionSubject::new(AdmissionScope::Title, []),
        }
    }

    /// Resolve a submission scope to the members scope-intersection needs.
    ///
    /// One catalog read at most, and only for scopes that have episodes. The
    /// collection ids matter because a season pack downloading for the season an
    /// episode belongs to is in flight *for that episode*, and the reverse: a
    /// single episode downloading blocks nothing else in its season.
    pub(crate) async fn scope_membership_for(
        &self,
        title: &Title,
        scope: &crate::SubmissionScope,
    ) -> crate::acquisition::acquisition::OwnedScopeMembership {
        use crate::SubmissionScope;
        use crate::acquisition::acquisition::OwnedScopeMembership;

        let episode_ids: Vec<String> = match scope {
            SubmissionScope::Episode { episode_id } => vec![episode_id.clone()],
            SubmissionScope::EpisodeSet { episode_ids } => episode_ids.clone(),
            SubmissionScope::Collection { .. }
            | SubmissionScope::Title
            | SubmissionScope::SeriesMovie { .. }
            | SubmissionScope::Orphan => Vec::new(),
        };

        match scope {
            SubmissionScope::Title | SubmissionScope::Orphan => OwnedScopeMembership::default(),
            SubmissionScope::SeriesMovie {
                series_movie_link_id,
            } => OwnedScopeMembership {
                series_movie_link_id: Some(series_movie_link_id.clone()),
                ..OwnedScopeMembership::default()
            },
            SubmissionScope::Collection { collection_id } => {
                let episode_ids = self
                    .services
                    .catalog
                    .shows
                    .list_episodes_for_collection(collection_id)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|episode| episode.id)
                    .collect();
                OwnedScopeMembership {
                    episode_ids,
                    collection_ids: vec![collection_id.clone()],
                    series_movie_link_id: None,
                }
            }
            SubmissionScope::Episode { .. } | SubmissionScope::EpisodeSet { .. } => {
                let catalog = self
                    .services
                    .catalog
                    .shows
                    .list_episodes_for_title(&title.id)
                    .await
                    .unwrap_or_default();
                let mut collection_ids: Vec<String> = catalog
                    .iter()
                    .filter(|episode| episode_ids.contains(&episode.id))
                    .filter_map(|episode| episode.collection_id.clone())
                    .collect();
                collection_ids.sort();
                collection_ids.dedup();
                OwnedScopeMembership {
                    episode_ids,
                    collection_ids,
                    series_movie_link_id: None,
                }
            }
        }
    }

    /// The scope's in-flight submissions, as pseudo-incumbents (D18).
    ///
    /// Sonarr's `QueueSpecification`. Liveness is deliberate, not incidental:
    ///
    /// - `Downloading | ImportPending | Importing`, or active in the client
    ///   snapshot: in flight, so it counts.
    /// - `ImportBlocked` **counts too**. A held import is a real claim on the
    ///   scope — the bytes exist — so an equal or worse release must not be
    ///   fetched beside it. A *better* one still may, which is the difference
    ///   from the old hard skip: a stuck import used to make its scope
    ///   permanently unsearchable.
    /// - `FailedPending` is excluded, exactly as Sonarr excludes it, so a
    ///   replacement can be grabbed while the failure handler runs. The lane
    ///   keeps its own hard skip for that state until the handler has run.
    /// - `Imported | ImportedSeeding | Failed | Ignored` are over.
    ///
    /// Every fact is re-derived from `source_title` and the size the submission
    /// recorded; see [`crate::admission::QueuedRelease`]. Pass the title's
    /// catalog rows where the lane already has them, so the queued release gets
    /// the same D4 runtime basis the candidate does — the size term is
    /// runtime-derived, and comparing a pack against a per-episode runtime would
    /// undo the parity this exists for.
    #[expect(
        clippy::too_many_arguments,
        reason = "the queued comparison needs the scope, the scoring context, the submissions, \
                  their tracked states and the client snapshot, and every one of them is \
                  already in the caller's hand"
    )]
    pub(crate) async fn queued_releases_for_scope(
        &self,
        title: &Title,
        membership: &crate::acquisition::acquisition::ScopeMembership<'_>,
        context: &ResolvedScoringContext,
        submissions: &[crate::DownloadSubmission],
        tracked_states: &std::collections::HashMap<
            crate::contracts::ClientJobLocator,
            scryer_domain::TrackedDownloadState,
        >,
        dl_snapshot: &crate::acquisition_workflow::DownloadClientSnapshot,
        catalog_episodes: &[scryer_domain::Episode],
        catalog_collections: &[scryer_domain::Collection],
    ) -> Vec<crate::admission::QueuedRelease> {
        submissions
            .iter()
            .filter(|submission| {
                crate::acquisition::acquisition::submission_scope_intersects(
                    &submission.scope,
                    membership,
                )
            })
            .filter(|submission| {
                let identity = crate::contracts::ClientJobLocator::from_submission(submission);
                crate::acquisition_workflow::submission_is_live_claim(
                    submission,
                    tracked_states.get(&identity).copied(),
                    dl_snapshot,
                )
            })
            .filter_map(|submission| {
                let release_title = submission
                    .source_title
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)?;
                let facts = score_parked_release_title(
                    title,
                    &release_title,
                    // The size the release announced at grab time. A queued
                    // release scored without it carries no size term while the
                    // candidate beside it does, so any candidate in a larger
                    // band reads as an upgrade over an identical download
                    // already in flight. `None` on pre-0.18 rows, which then
                    // compare size-less on both sides of the term.
                    submission.release_size_bytes,
                    catalog_episodes,
                    catalog_collections,
                    context,
                );
                Some(crate::admission::QueuedRelease {
                    title: release_title,
                    covers: match &submission.scope {
                        crate::SubmissionScope::Episode { episode_id } => vec![episode_id.clone()],
                        crate::SubmissionScope::EpisodeSet { episode_ids } => episode_ids.clone(),
                        crate::SubmissionScope::Collection { .. } => {
                            membership.episode_ids.to_vec()
                        }
                        crate::SubmissionScope::Title
                        | crate::SubmissionScope::Orphan
                        | crate::SubmissionScope::SeriesMovie { .. } => Vec::new(),
                    },
                    tier_index: facts.tier_index,
                    revision: facts.revision,
                    score: facts.score,
                })
            })
            .collect()
    }

    /// The bar a candidate must clear to displace `file`.
    ///
    /// Always re-derived from the row, never read back from
    /// `media_files.acquisition_score`. A stored number is only valid while the
    /// profile, persona, rule packs and algorithm that produced it are all
    /// unchanged, and re-deriving is one parse plus the term pipeline over
    /// columns already in memory — cheaper than proving the stored one is still
    /// true. The persisted score stays for display and history; it is not a bar.
    ///
    /// All three facts come out of the same re-derivation because admission
    /// compares them in one ladder (tier → revision → score, I3/D9); splitting
    /// them across two derivations is how the sides drift.
    pub(crate) fn incumbent_bar(
        &self,
        file: &crate::TitleMediaFile,
        context: &ResolvedScoringContext,
        size_basis: CoverageSizeBasis,
    ) -> IncumbentFacts {
        let view = context.view(size_basis, false);
        let scored = crate::canonical_scoring::score_media_file(file, &view);
        let tier_index = crate::quality_profile::quality_tier_index(
            &context.profile().criteria,
            scored
                .parsed_quality
                .as_deref()
                .or(file.resolution.as_deref())
                .or(file.quality_label.as_deref()),
        );
        IncumbentFacts {
            score: scored.total,
            tier_index,
            revision: scored.revision,
        }
    }
}

/// One stored file, judged the way the gate judges it.
///
/// A struct rather than a tuple because it is exactly the incumbent half of
/// [`crate::admission::CandidateFacts`], and a three-element tuple of
/// `(i32, Option<usize>, i32)` is two swappable `i32`s waiting to happen.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IncumbentFacts {
    /// The file's canonical landed score — its bar.
    pub score: i32,
    /// Position in the profile's quality ordering; lower is better.
    pub tier_index: Option<usize>,
    /// PROPER/REPACK rank of the release the row remembers.
    pub revision: i32,
}
