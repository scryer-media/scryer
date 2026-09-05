//! The request input document and the reads that fill it (spec 0003 FR-015,
//! plan §3.2, §4.2).
//!
//! Two halves, deliberately separated:
//!
//! - [`build_request_input`] is **pure**. Context in, document out, no clock, no
//!   repository. Every rule about how a fact turns into an [`Observation`] —
//!   what makes it `unknown` rather than `absent`, which certification is the
//!   one that ranks, how an age in days is derived — lives here and is unit
//!   testable without a harness.
//! - [`AppUseCase::assemble_request_input_context`] is the async half: it
//!   performs each read exactly once and hands the result over.
//!
//! # Unknown is not absent
//!
//! The distinction is the entire safety story. `absent` means a source answered
//! and there is no value ("this film carries no US certification"); a rule reads
//! that as a real answer. `unknown` means Scryer could not find out, and the
//! policy core holds any rule that reads it, which arbitrates to manual review.
//! Everything derived from a snapshot group the enrichment could not establish
//! is `unknown`; everything a source answered "nothing" to is `absent`, with the
//! reason recorded so an approver sees *why*.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, Utc};
use scryer_domain::{
    AppPermission, ExternalAccountStatus, ExternalId, Library, LibraryPermission, MediaFacet,
    MonitorSelection, User, UserAccountKind,
};
use scryer_rules::request::{
    Observation, REQUEST_INPUT_SCHEMA_VERSION, RequestCertificationDoc, RequestClockDoc,
    RequestDoc, RequestFactsDoc, RequestInput, RequestLibraryDoc, RequestRequesterDoc,
    certification_rank_for_label, max_resolution_for_quality_tiers,
};

use crate::media_requests::snapshot::{MediaRequestMetadataSnapshot, SNAPSHOT_GROUP_ALL};
use crate::{AppResult, AppUseCase};

/// Snapshot group names the fact builder asks [`MediaRequestMetadataSnapshot::is_missing`]
/// about. They match the group vocabulary WP3 writes into `missing`.
const GROUP_CONTENT_RATINGS: &str = "content_ratings";
const GROUP_GENRES: &str = "genres";
const GROUP_MDBLIST: &str = "mdblist";
const GROUP_RATINGS: &str = "ratings";
const GROUP_AWARDS: &str = "awards";

/// Reason code used for every fact derived from a snapshot group enrichment
/// could not establish. It is the code a held rule reports, so an approver sees
/// "Scryer could not read the metadata", not "the film is not rated".
pub const REASON_METADATA_UNAVAILABLE: &str = "metadata_unavailable";

/// Country codes that count as the United States certification. The rank ladder
/// [`certification_rank_for_label`] implements is the US one, so ranking a
/// German FSK label against it would be a category error; a title with only
/// foreign certifications leaves the label absent and the rank with it.
const US_COUNTRY_CODES: [&str; 4] = ["us", "usa", "united states", "united states of america"];

/// The draft a rule is evaluated against — the same shape whether it came from
/// a pre-flight preview, a submit, or an edit of a pending request.
#[derive(Clone, Debug)]
pub struct RequestDraft {
    pub facet: MediaFacet,
    pub title: String,
    pub year: Option<i32>,
    pub external_ids: Vec<ExternalId>,
    pub identity_fingerprint: String,
    pub quality_profile_id: Option<String>,
    pub quality_profile_name: Option<String>,
    pub monitor_type: Option<String>,
    pub monitor_selection: Option<MonitorSelection>,
    /// `None` means forever (spec 0003 FR-040).
    pub requested_lease_days: Option<i64>,
}

/// The quality profile the draft names, resolved once.
#[derive(Clone, Debug)]
pub struct RequestQualityContext {
    pub tiers: Vec<String>,
    pub allow_upgrades: bool,
}

/// What the catalog knows about this identity.
#[derive(Clone, Debug, Default)]
pub struct RequestCatalogContext {
    /// Libraries of the same facet that already hold the identity, excluding
    /// the request's own target library.
    pub exists_in_library_ids: Vec<String>,
    pub previous_request_count: i64,
    pub previously_denied: bool,
    pub previously_approved: bool,
    /// False when a read failed; every catalog fact then reads unknown.
    pub readable: bool,
}

/// What the instance knows about this requester's own history.
#[derive(Clone, Debug, Default)]
pub struct RequestRequesterHistoryContext {
    pub pending_request_count: i64,
    pub approved_last_30d: i64,
    pub denied_last_30d: i64,
    pub total_approved: i64,
    /// `None` when the claim store could not be read — distinct from zero.
    pub active_lease_count: Option<i64>,
    /// `None` is a real answer: this person has never submitted anything.
    pub last_request_at: Option<DateTime<Utc>>,
    /// False when a read failed; the four counters then read unknown.
    pub readable: bool,
}

/// Everything one evaluation needs, already read.
#[derive(Clone, Debug)]
pub struct RequestInputContext {
    pub evaluation_time: DateTime<Utc>,
    pub requester: RequestRequesterDoc,
    pub library: RequestLibraryDoc,
    pub request: RequestDoc,
    pub snapshot: MediaRequestMetadataSnapshot,
    /// `None` when the draft's profile could not be resolved; the three quality
    /// facts then read unknown rather than defaulting to a permissive tier list.
    pub quality: Option<RequestQualityContext>,
    pub catalog: RequestCatalogContext,
    pub history: RequestRequesterHistoryContext,
    /// `None` when no count is available; the fact reads unknown.
    pub library_title_count: Option<i64>,
}

// ── The pure builder ────────────────────────────────────────────────────────

/// Turn a fully-read context into the document the engine evaluates.
pub fn build_request_input(context: RequestInputContext) -> RequestInput {
    let facts = build_facts(&context);
    RequestInput {
        schema_version: REQUEST_INPUT_SCHEMA_VERSION,
        evaluation_time: context.evaluation_time,
        now: RequestClockDoc::at(context.evaluation_time),
        requester: context.requester,
        library: context.library,
        request: context.request,
        facts,
    }
}

fn build_facts(context: &RequestInputContext) -> RequestFactsDoc {
    let snapshot = &context.snapshot;
    let ratings_known = !snapshot.is_missing(GROUP_RATINGS);

    RequestFactsDoc {
        // ── content rating ──
        age_rating: snapshot_fact(snapshot, GROUP_CONTENT_RATINGS, || {
            snapshot_age_rating(snapshot).ok_or("no_age_rating")
        }),
        certifications: snapshot_fact(snapshot, GROUP_CONTENT_RATINGS, || {
            let certifications: Vec<RequestCertificationDoc> = snapshot
                .content_ratings
                .iter()
                .flat_map(|rating| {
                    rating
                        .certifications
                        .iter()
                        .map(|certification| RequestCertificationDoc {
                            country: rating.country.clone(),
                            value: certification.value.clone(),
                            source: certification.source.clone(),
                        })
                })
                .collect();
            if certifications.is_empty() {
                Err("no_certifications")
            } else {
                Ok(certifications)
            }
        }),
        certification_label: snapshot_fact(snapshot, GROUP_CONTENT_RATINGS, || {
            us_certification_label(snapshot).ok_or("no_us_certification")
        }),
        certification_rank: snapshot_fact(snapshot, GROUP_CONTENT_RATINGS, || {
            let Some(label) = us_certification_label(snapshot) else {
                return Err("no_us_certification");
            };
            // A label off the ladder is an *absence* of a rank, not an unknown:
            // the source answered, and Scryer's ladder simply does not place
            // that string. A rule comparing ranks then does not match, which is
            // the same thing it does for an unrated title.
            certification_rank_for_label(&label).ok_or("unrankable_certification")
        }),
        commonsense_recommended: snapshot_fact(snapshot, GROUP_MDBLIST, || {
            snapshot
                .mdblist
                .as_ref()
                .and_then(|mdblist| mdblist.commonsense)
                .ok_or("no_commonsense_rating")
        }),

        // ── title metadata ──
        genres: snapshot_fact(snapshot, GROUP_GENRES, || {
            let genres: Vec<String> = snapshot.genres.clone();
            if genres.is_empty() {
                Err("no_genres")
            } else {
                Ok(genres)
            }
        }),
        canonical_tag_keys: snapshot_fact(snapshot, GROUP_GENRES, || {
            let keys: Vec<String> = snapshot
                .canonical_tags
                .iter()
                .map(|tag| tag.key.clone())
                .collect();
            if keys.is_empty() {
                Err("no_canonical_tags")
            } else {
                Ok(keys)
            }
        }),
        themes: snapshot_fact(snapshot, GROUP_GENRES, || {
            let themes: Vec<String> = snapshot
                .canonical_tags
                .iter()
                .filter(|tag| tag.category.eq_ignore_ascii_case("theme"))
                .map(|tag| {
                    if tag.name.trim().is_empty() {
                        tag.key.clone()
                    } else {
                        tag.name.clone()
                    }
                })
                .collect();
            if themes.is_empty() {
                Err("no_themes")
            } else {
                Ok(themes)
            }
        }),
        // The adult flag is derived from the canonical tags, so it is known
        // exactly when they are — and `false` there is a real answer, not a
        // default: the tags were read and none of them is adult.
        is_adult: snapshot_fact(snapshot, GROUP_GENRES, || Ok(snapshot.is_adult)),
        rating: snapshot_fact(snapshot, GROUP_RATINGS, || {
            snapshot.ratings.rating.ok_or("no_rating")
        }),
        ratings_by_source: snapshot_fact(snapshot, GROUP_RATINGS, || {
            let by_source: BTreeMap<String, f64> = snapshot
                .ratings
                .external_ratings
                .iter()
                .filter(|rating| !rating.source.trim().is_empty())
                .map(|rating| (rating.source.trim().to_ascii_lowercase(), rating.normalized))
                .collect();
            if by_source.is_empty() {
                Err("no_external_ratings")
            } else {
                Ok(by_source)
            }
        }),
        tmdb_vote_average: optional_snapshot_fact(
            ratings_known,
            snapshot.tmdb_vote_average,
            "no_tmdb_vote_average",
        ),
        tmdb_vote_count: optional_snapshot_fact(
            ratings_known,
            snapshot.tmdb_vote_count,
            "no_tmdb_vote_count",
        ),
        popularity: optional_snapshot_fact(ratings_known, snapshot.popularity, "no_popularity"),
        runtime_minutes: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.runtime_minutes.map(i64::from),
            "no_runtime",
        ),
        original_language: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.original_language.clone(),
            "no_original_language",
        ),
        country: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.country.clone(),
            "no_country",
        ),
        network: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.network.clone(),
            "no_network",
        ),
        studio: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.studio.clone(),
            "no_studio",
        ),
        content_status: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.content_status.clone(),
            "no_content_status",
        ),
        release_date: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.release_date.clone(),
            "no_release_date",
        ),
        first_aired: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.first_aired.clone(),
            "no_first_aired",
        ),
        release_age_days: if snapshot.is_missing(SNAPSHOT_GROUP_ALL) {
            Observation::unknown(REASON_METADATA_UNAVAILABLE)
        } else {
            // A series' first air date is its release date; a movie has only
            // the one. Whichever is present is what the age is measured from.
            match snapshot
                .release_date
                .as_deref()
                .or(snapshot.first_aired.as_deref())
            {
                None => Observation::absent_because("no_release_date"),
                Some(value) => match parse_release_date(value) {
                    None => Observation::absent_because("unparseable_release_date"),
                    Some(date) => {
                        Observation::known((context.evaluation_time.date_naive() - date).num_days())
                    }
                },
            }
        },
        award_count: snapshot_fact(snapshot, GROUP_AWARDS, || Ok(snapshot.awards.len() as i64)),

        // ── quality ──
        quality_profile_tiers: match &context.quality {
            None => Observation::unknown("quality_profile_unavailable"),
            Some(quality) if quality.tiers.is_empty() => {
                Observation::absent_because("no_quality_tiers")
            }
            Some(quality) => Observation::known(quality.tiers.clone()),
        },
        quality_profile_max_resolution: match &context.quality {
            None => Observation::unknown("quality_profile_unavailable"),
            Some(quality) => match max_resolution_for_quality_tiers(&quality.tiers) {
                None => Observation::absent_because("no_resolution_in_quality_tiers"),
                Some(resolution) => Observation::known(resolution),
            },
        },
        quality_profile_allows_upgrades: match &context.quality {
            None => Observation::unknown("quality_profile_unavailable"),
            Some(quality) => Observation::known(quality.allow_upgrades),
        },

        // ── catalog ──
        exists_in_library_ids: catalog_fact(&context.catalog, || {
            context.catalog.exists_in_library_ids.clone()
        }),
        previous_request_count: catalog_fact(&context.catalog, || {
            context.catalog.previous_request_count
        }),
        previously_denied: catalog_fact(&context.catalog, || context.catalog.previously_denied),
        previously_approved: catalog_fact(&context.catalog, || context.catalog.previously_approved),

        // ── requester history ──
        pending_request_count: history_fact(&context.history, || {
            context.history.pending_request_count
        }),
        approved_last_30d: history_fact(&context.history, || context.history.approved_last_30d),
        denied_last_30d: history_fact(&context.history, || context.history.denied_last_30d),
        total_approved: history_fact(&context.history, || context.history.total_approved),
        active_lease_count: match context.history.active_lease_count {
            None => Observation::unknown("lifecycle_claims_unavailable"),
            Some(count) => Observation::known(count),
        },
        days_since_last_request: if !context.history.readable {
            Observation::unknown("request_history_unavailable")
        } else {
            match context.history.last_request_at {
                // Never having asked for anything is an answer, not a gap: a
                // rule can legitimately say "first-time requesters go to a
                // human" by testing `not input.facts.days_since_last_request`.
                None => Observation::absent_because("never_requested"),
                Some(last) => Observation::known(
                    (context.evaluation_time.date_naive() - last.date_naive()).num_days(),
                ),
            }
        },

        // ── library ──
        library_title_count: match context.library_title_count {
            None => Observation::unknown("not_yet_collected"),
            Some(count) => Observation::known(count),
        },
    }
}

/// A fact derived from one snapshot group: unknown when the group is missing,
/// otherwise whatever the closure decided (`Ok` known, `Err(reason)` absent).
fn snapshot_fact<T, F>(
    snapshot: &MediaRequestMetadataSnapshot,
    group: &str,
    resolve: F,
) -> Observation<T>
where
    T: serde::Serialize,
    F: FnOnce() -> Result<T, &'static str>,
{
    if snapshot.is_missing(group) {
        return Observation::unknown(REASON_METADATA_UNAVAILABLE);
    }
    match resolve() {
        Ok(value) => Observation::known(value),
        Err(reason) => Observation::absent_because(reason),
    }
}

/// The scalar case of [`snapshot_fact`], where the group is already resolved to
/// a boolean and the value is an `Option`.
fn optional_snapshot_fact<T: serde::Serialize>(
    group_known: bool,
    value: Option<T>,
    absent_reason: &'static str,
) -> Observation<T> {
    if !group_known {
        return Observation::unknown(REASON_METADATA_UNAVAILABLE);
    }
    match value {
        Some(value) => Observation::known(value),
        None => Observation::absent_because(absent_reason),
    }
}

fn catalog_fact<T: serde::Serialize, F: FnOnce() -> T>(
    catalog: &RequestCatalogContext,
    resolve: F,
) -> Observation<T> {
    if catalog.readable {
        Observation::known(resolve())
    } else {
        Observation::unknown("catalog_unavailable")
    }
}

fn history_fact<T: serde::Serialize, F: FnOnce() -> T>(
    history: &RequestRequesterHistoryContext,
    resolve: F,
) -> Observation<T> {
    if history.readable {
        Observation::known(resolve())
    } else {
        Observation::unknown("request_history_unavailable")
    }
}

/// The US content-rating row, if the snapshot has one.
fn us_content_rating(
    snapshot: &MediaRequestMetadataSnapshot,
) -> Option<&crate::types::ContentRating> {
    snapshot.content_ratings.iter().find(|rating| {
        US_COUNTRY_CODES
            .iter()
            .any(|code| rating.country.trim().eq_ignore_ascii_case(code))
    })
}

/// The minimum age a snapshot's content ratings imply.
///
/// Content ratings first, MDBList second: the certification body's own minimum
/// age is the authoritative one, and MDBList's is a derived aggregate that only
/// helps when nobody published a rating.
///
/// Public because the GraphQL read model shows an approver the same number the
/// `age_rating` fact reads, and deriving it twice would let the two drift.
pub fn snapshot_age_rating(snapshot: &MediaRequestMetadataSnapshot) -> Option<i64> {
    us_content_rating(snapshot)
        .and_then(|rating| rating.age_rating)
        .or_else(|| {
            snapshot
                .content_ratings
                .iter()
                .find_map(|rating| rating.age_rating)
        })
        .or_else(|| {
            if snapshot.is_missing(GROUP_MDBLIST) {
                None
            } else {
                snapshot
                    .mdblist
                    .as_ref()
                    .and_then(|mdblist| mdblist.age_rating)
            }
        })
        .map(i64::from)
}

/// The US certification value, if there is one. Public for the same reason
/// [`snapshot_age_rating`] is.
pub fn us_certification_label(snapshot: &MediaRequestMetadataSnapshot) -> Option<String> {
    us_content_rating(snapshot).and_then(|rating| {
        rating
            .certifications
            .iter()
            .map(|certification| certification.value.trim())
            .find(|value| !value.is_empty())
            .map(str::to_string)
    })
}

/// Accepts what the metadata sources actually publish: an RFC3339 timestamp or
/// a bare `YYYY-MM-DD`.
fn parse_release_date(value: &str) -> Option<NaiveDate> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date);
    }
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc).date_naive())
}

// ── The async half ──────────────────────────────────────────────────────────

impl AppUseCase {
    /// Perform every read one evaluation needs, exactly once each.
    ///
    /// A failure of any *fact* source is not a failure of the assembly: it
    /// degrades that group to unknown, which holds the rules that read it and
    /// lands the request in manual review. Only a read that would make the
    /// document meaningless — none, today — would be allowed to error.
    pub(crate) async fn assemble_request_input_context(
        &self,
        actor: &User,
        library: &Library,
        draft: &crate::request_rules::facts::RequestDraft,
        snapshot: MediaRequestMetadataSnapshot,
        evaluation_time: DateTime<Utc>,
    ) -> AppResult<RequestInputContext> {
        let requester = self.request_requester_doc(actor, &library.id).await?;
        let request = request_doc(draft);
        let quality = self.request_quality_context(draft).await;
        let catalog = self.request_catalog_context(library, draft).await;
        let history = self.request_history_context(actor, evaluation_time).await;
        let library_title_count = self.request_library_title_count(&library.id).await;

        Ok(RequestInputContext {
            evaluation_time,
            requester,
            library: RequestLibraryDoc {
                id: library.id.clone(),
                name: library.name.clone(),
                facet: library.facet.as_str().to_string(),
                is_default: library.is_default,
            },
            request,
            snapshot,
            quality,
            catalog,
            history,
            library_title_count,
        })
    }

    /// The requester document: identity, the permissions they actually hold,
    /// and their verified external links.
    ///
    /// Permissions are emitted as the same `as_str()` names the rest of Scryer
    /// uses, so a rule saying `"manage_titles" in input.requester.library_permissions`
    /// reads the vocabulary an operator already knows from the permissions
    /// screen.
    async fn request_requester_doc(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<RequestRequesterDoc> {
        let mut app_permissions = Vec::new();
        for permission in [
            AppPermission::ManageUsers,
            AppPermission::ManagePermissions,
            AppPermission::ManageSystemSettings,
            AppPermission::ManageCatalogSettings,
        ] {
            if self.has_app_permission(actor, permission).await? {
                app_permissions.push(permission.as_str().to_string());
            }
        }

        let mut library_permissions = Vec::new();
        for permission in [
            LibraryPermission::View,
            LibraryPermission::ManageTitles,
            LibraryPermission::ResolveImports,
            LibraryPermission::ManageLibrary,
            LibraryPermission::Request,
            LibraryPermission::AutoApproveRequests,
        ] {
            if self
                .has_library_permission(actor, library_id, permission)
                .await?
            {
                library_permissions.push(permission.as_str().to_string());
            }
        }

        // Verified links only: a pending claim is an assertion nobody has
        // confirmed, and a rule that trusted it would let anyone grant
        // themselves a provider identity by typing a username.
        let linked_providers = match self
            .services
            .identity
            .external_accounts
            .list_by_user_id(&actor.id)
            .await
        {
            Ok(accounts) => accounts
                .into_iter()
                .filter(|account| {
                    account.status == ExternalAccountStatus::Active && account.verified_at.is_some()
                })
                .map(|account| account.provider.as_str().to_string())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            Err(error) => {
                tracing::warn!(
                    user_id = actor.id.as_str(),
                    error = %error,
                    "could not read the requester's external accounts; rules see no linked providers"
                );
                Vec::new()
            }
        };

        Ok(RequestRequesterDoc {
            user_id: actor.id.clone(),
            username: actor.username.clone(),
            account_kind: match actor.account_kind {
                UserAccountKind::Local => "local".to_string(),
                UserAccountKind::ExternalAutoProvisioned => "external_auto_provisioned".to_string(),
            },
            app_permissions,
            library_permissions,
            linked_providers,
            // `User` carries no creation timestamp today, so the contract's
            // optional field is always absent rather than invented. Wiring it
            // means widening the user row's projection, which is not this
            // change.
            created_at: None,
        })
    }

    async fn request_quality_context(
        &self,
        draft: &crate::request_rules::facts::RequestDraft,
    ) -> Option<RequestQualityContext> {
        let profile_id = draft.quality_profile_id.as_deref()?;
        let settings = match self.load_quality_profile_settings().await {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read quality profiles; request rules see unknown quality facts"
                );
                return None;
            }
        };
        let profile =
            crate::settings::runtime::quality_profile_by_id(&settings.profiles, profile_id)
                .ok()
                .flatten()?;
        Some(RequestQualityContext {
            tiers: profile.criteria.quality_tiers.clone(),
            allow_upgrades: profile.criteria.allow_upgrades,
        })
    }

    async fn request_catalog_context(
        &self,
        library: &Library,
        draft: &crate::request_rules::facts::RequestDraft,
    ) -> RequestCatalogContext {
        let mut context = RequestCatalogContext {
            readable: true,
            ..RequestCatalogContext::default()
        };

        let mut library_ids: Vec<String> = Vec::new();
        for external_id in &draft.external_ids {
            match self
                .services
                .catalog
                .titles
                .find_by_external_id_in_facet(
                    draft.facet.clone(),
                    &external_id.source,
                    &external_id.value,
                )
                .await
            {
                // The request's own library is excluded: submit already refuses
                // a title that is there, so including it would make the fact
                // read as "already elsewhere" for a request that is not.
                Ok(Some(title)) if title.library_id != library.id => {
                    if !library_ids.contains(&title.library_id) {
                        library_ids.push(title.library_id);
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "could not read the catalog for a request rule fact"
                    );
                    context.readable = false;
                    return context;
                }
            }
        }
        context.exists_in_library_ids = library_ids;

        match self
            .services
            .catalog
            .media_requests
            .history_for_fingerprint(&draft.identity_fingerprint)
            .await
        {
            Ok(history) => {
                context.previous_request_count = history.len() as i64;
                context.previously_denied = history
                    .iter()
                    .any(|request| request.status == scryer_domain::MediaRequestStatus::Rejected);
                context.previously_approved = history
                    .iter()
                    .any(|request| request.status == scryer_domain::MediaRequestStatus::Approved);
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read request history for a request rule fact"
                );
                context.readable = false;
            }
        }

        context
    }

    async fn request_history_context(
        &self,
        actor: &User,
        evaluation_time: DateTime<Utc>,
    ) -> RequestRequesterHistoryContext {
        use scryer_domain::MediaRequestStatus;

        let thirty_days_ago = evaluation_time - chrono::Duration::days(30);
        let requests = &self.services.catalog.media_requests;
        let mut context = RequestRequesterHistoryContext {
            readable: true,
            ..RequestRequesterHistoryContext::default()
        };

        let counters = async {
            Ok::<_, crate::AppError>((
                requests
                    .count_for_requester(&actor.id, Some(MediaRequestStatus::Pending), None)
                    .await?,
                requests
                    .count_for_requester(
                        &actor.id,
                        Some(MediaRequestStatus::Approved),
                        Some(thirty_days_ago),
                    )
                    .await?,
                requests
                    .count_for_requester(
                        &actor.id,
                        Some(MediaRequestStatus::Rejected),
                        Some(thirty_days_ago),
                    )
                    .await?,
                requests
                    .count_for_requester(&actor.id, Some(MediaRequestStatus::Approved), None)
                    .await?,
                requests.latest_request_at_for_user(&actor.id).await?,
            ))
        }
        .await;

        match counters {
            Ok((pending, approved_30d, denied_30d, total_approved, last_request_at)) => {
                context.pending_request_count = pending as i64;
                context.approved_last_30d = approved_30d as i64;
                context.denied_last_30d = denied_30d as i64;
                context.total_approved = total_approved as i64;
                context.last_request_at = last_request_at;
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read requester history for request rule facts"
                );
                context.readable = false;
            }
        }

        // Counted separately because it comes from a different store: an
        // unreadable claim store must leave *this* fact unknown without also
        // blanking the request counters, which were read successfully.
        context.active_lease_count = match self
            .services
            .catalog
            .lifecycle_claims
            .count_live_for_user(&actor.id)
            .await
        {
            Ok(count) => Some(count as i64),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not count live lifecycle claims; the lease fact is unknown"
                );
                None
            }
        };

        context
    }

    async fn request_library_title_count(&self, library_id: &str) -> Option<i64> {
        match self
            .services
            .catalog
            .titles
            .count_titles_in_library(library_id)
            .await
        {
            Ok(count) => Some(count as i64),
            Err(error) => {
                tracing::warn!(
                    library_id,
                    error = %error,
                    "could not count titles in the library; the fact is unknown"
                );
                None
            }
        }
    }
}

/// The draft half of the input document. Always known: it is what the requester
/// is looking at.
fn request_doc(draft: &RequestDraft) -> RequestDoc {
    RequestDoc {
        // Fixed until a non-manual origin exists; the field is in the contract
        // so a watchlist-sourced request needs no schema bump.
        origin: "manual".to_string(),
        title: draft.title.clone(),
        year: draft.year,
        external_ids: draft
            .external_ids
            .iter()
            .map(|external_id| {
                (
                    external_id.source.trim().to_ascii_lowercase(),
                    external_id.value.trim().to_string(),
                )
            })
            .collect(),
        quality_profile_id: draft.quality_profile_id.clone(),
        quality_profile_name: draft.quality_profile_name.clone(),
        monitor_type: draft.monitor_type.clone(),
        monitor_selection_season_count: draft
            .monitor_selection
            .as_ref()
            .map(|selection| selection.seasons.len() as i64),
        lease_forever: draft.requested_lease_days.is_none(),
        lease_days: draft.requested_lease_days,
    }
}
