// Grab-time seeding-goal resolution.
//
// A single helper answers "what seeding goals does this grab get?" so the
// download-client choke point never has to know the precedence rules and no
// construction site has to duplicate them. Precedence mirrors the design doc
// (§4.2): the indexer that supplied the release wins, then the download-client
// routing entry the grab was routed through, then the global default. Nothing
// resolved means no goals at all — seeding profiles are opt-in, so an install
// with no profiles behaves exactly as it does today.

use std::sync::Arc;

use chrono::Utc;
use scryer_domain::{
    IndexerConfig, PostImportTracking, SeasonPackSeedMode, SeedGoalMetAction, SeedingProfile,
};

use crate::DownloadSourceKind;
use serde_json::Value;

use crate::{
    AppResult, DEFAULT_SEEDING_PROFILE_SETTING_KEY, IndexerConfigRepository,
    MINIMUM_SEEDERS_FLOOR_DEFAULT, MINIMUM_SEEDERS_FLOOR_SETTING_KEY, SETTINGS_SCOPE_SYSTEM,
    SeedingProfileRepository, SettingsRepository,
};

/// Which assignment level supplied the profile. Persisted with the resolution
/// so later packages (and operators reading history) can explain a goal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeedGoalResolutionSource {
    /// No profile applied; the client's own global limits stay in charge.
    #[default]
    None,
    /// The indexer that supplied the release carries a profile.
    Indexer,
    /// Seed criteria imported from Prowlarr for a managed child indexer. Used
    /// only when that child has no seeding profile of its own, so assigning
    /// one is how an operator overrides what Prowlarr holds.
    ProwlarrManaged,
    /// The download-client routing entry the grab was routed through.
    RoutingEntry,
    /// The global default seeding profile setting.
    GlobalDefault,
}

impl SeedGoalResolutionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Indexer => "indexer",
            Self::ProwlarrManaged => "prowlarr_managed",
            Self::RoutingEntry => "routing_entry",
            Self::GlobalDefault => "global_default",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "none" => Some(Self::None),
            "indexer" => Some(Self::Indexer),
            "prowlarr_managed" => Some(Self::ProwlarrManaged),
            "routing_entry" => Some(Self::RoutingEntry),
            "global_default" => Some(Self::GlobalDefault),
            _ => None,
        }
    }
}

/// Everything the resolver needs about one grab. Tracker minimums come off the
/// release `extra` map the indexer adapter populates
/// (`minimum_seed_ratio` / `minimum_seed_time_minutes` and the season-pack
/// twins); construction sites that have no release object pass `None` and the
/// resolver simply skips the clamp.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeedGoalRequest {
    /// Indexer that supplied the release, when known.
    pub indexer_id: Option<String>,
    /// `seedingProfileId` from the routing entry of the client this grab was
    /// actually routed to.
    pub routing_seeding_profile_id: Option<String>,
    /// Whether the release is a season pack (drives the profile's season-pack
    /// override and which tracker minimum applies).
    pub season_pack: bool,
    pub tracker_min_seed_ratio: Option<f64>,
    pub tracker_min_seed_time_minutes: Option<i64>,
    pub season_pack_min_seed_ratio: Option<f64>,
    pub season_pack_min_seed_time_minutes: Option<i64>,
}

impl SeedGoalRequest {
    /// Tracker minimum for the ratio axis, preferring the season-pack minimum
    /// on season-pack releases and falling back to the per-release minimum.
    fn effective_min_ratio(&self) -> Option<f64> {
        if self.season_pack {
            self.season_pack_min_seed_ratio
                .or(self.tracker_min_seed_ratio)
        } else {
            self.tracker_min_seed_ratio
        }
    }

    fn effective_min_seed_time_minutes(&self) -> Option<i64> {
        if self.season_pack {
            self.season_pack_min_seed_time_minutes
                .or(self.tracker_min_seed_time_minutes)
        } else {
            self.tracker_min_seed_time_minutes
        }
    }
}

/// The resolved policy for one grab. `seeding_profile_id` is `None` exactly
/// when `resolution_source` is `None`, and in that case every goal is `None`
/// too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedSeedGoals {
    pub seeding_profile_id: Option<String>,
    pub seed_goal_ratio: Option<f64>,
    pub seed_goal_seconds: Option<i64>,
    pub never_remove: bool,
    pub goal_met_action: Option<SeedGoalMetAction>,
    /// Whether Scryer keeps managing the torrent after import. Frozen with the
    /// goals, so a torrent keeps the tracking mode it was grabbed under.
    pub post_import_tracking: PostImportTracking,
    pub resolution_source: SeedGoalResolutionSource,
}

impl ResolvedSeedGoals {
    /// Whether anything was resolved at all. A profile with no goals on either
    /// axis still counts — `never_remove` and `goal_met_action` are policy even
    /// without a numeric goal.
    pub fn is_resolved(&self) -> bool {
        self.resolution_source != SeedGoalResolutionSource::None
    }

    /// Whether either numeric goal is present (i.e. there is something to push
    /// to a Tier-A client or evaluate in Tier B).
    pub fn has_goals(&self) -> bool {
        self.seed_goal_ratio.is_some() || self.seed_goal_seconds.is_some()
    }
}

/// Resolves seeding goals from the three assignment levels plus tracker
/// minimums. Cheap to clone; every lookup goes straight to the repositories so
/// a profile edit takes effect on the next grab.
#[derive(Clone)]
pub struct SeedGoalResolver {
    seeding_profiles: Arc<dyn SeedingProfileRepository>,
    indexer_configs: Option<Arc<dyn IndexerConfigRepository>>,
    settings: Arc<dyn SettingsRepository>,
}

impl SeedGoalResolver {
    pub fn new(
        seeding_profiles: Arc<dyn SeedingProfileRepository>,
        indexer_configs: Option<Arc<dyn IndexerConfigRepository>>,
        settings: Arc<dyn SettingsRepository>,
    ) -> Self {
        Self {
            seeding_profiles,
            indexer_configs,
            settings,
        }
    }

    /// Resolve the applicable profile and compute its goals for one grab.
    pub async fn resolve(&self, request: &SeedGoalRequest) -> AppResult<ResolvedSeedGoals> {
        let Some((profile, source)) = self.resolve_profile(request).await? else {
            return Ok(ResolvedSeedGoals::default());
        };
        Ok(apply_profile(&profile, request, source))
    }

    /// Walk the precedence chain. A level that names a profile which no longer
    /// exists falls through to the next level rather than failing the grab —
    /// a dangling assignment must never block a download.
    async fn resolve_profile(
        &self,
        request: &SeedGoalRequest,
    ) -> AppResult<Option<(SeedingProfile, SeedGoalResolutionSource)>> {
        if let Some(indexer_id) = trimmed(request.indexer_id.as_deref())
            && let Some(repository) = self.indexer_configs.as_ref()
            && let Some(indexer) = repository.get_by_id(&indexer_id).await?
        {
            // An assigned profile always wins: choosing one is exactly how an
            // operator overrides the criteria Prowlarr holds for this tracker.
            if let Some(profile_id) = trimmed(indexer.seeding_profile_id.as_deref())
                && let Some(profile) = self.load_profile(&profile_id).await?
            {
                return Ok(Some((profile, SeedGoalResolutionSource::Indexer)));
            }
            // Goals only: a child whose Prowlarr criteria are just an
            // `appMinimumSeeders` supplies nothing to this walk and falls
            // through to the tiers below.
            if let Some(profile) = prowlarr_managed_goal_profile(&indexer) {
                return Ok(Some((profile, SeedGoalResolutionSource::ProwlarrManaged)));
            }
        }

        if let Some(profile_id) = trimmed(request.routing_seeding_profile_id.as_deref())
            && let Some(profile) = self.load_profile(&profile_id).await?
        {
            return Ok(Some((profile, SeedGoalResolutionSource::RoutingEntry)));
        }

        if let Some(profile_id) = self.default_seeding_profile_id().await?
            && let Some(profile) = self.load_profile(&profile_id).await?
        {
            return Ok(Some((profile, SeedGoalResolutionSource::GlobalDefault)));
        }

        Ok(None)
    }

    async fn load_profile(&self, profile_id: &str) -> AppResult<Option<SeedingProfile>> {
        self.seeding_profiles.get_by_id(profile_id).await
    }

    /// Fewest seeders a candidate from this indexer may report and still be
    /// grabbed.
    ///
    /// Deliberately a sibling of `resolve_profile` rather than a call into it.
    /// That walk has a `RoutingEntry` tier, and admission runs *before* a
    /// download-client route is chosen, so the routing level cannot
    /// participate: an operator who has only assigned a profile to a routing
    /// entry gets the floor here, not that profile. Keeping the two walks
    /// adjacent is what makes that divergence readable as a decision.
    ///
    /// Precedence: indexer profile → Prowlarr-managed → global default
    /// profile → system floor. A resolved *profile* whose `minimum_seeders` is
    /// `None` inherits the floor rather than falling through to the next tier,
    /// matching "first profile wins" everywhere else in this module.
    pub async fn resolve_minimum_seeders(&self, indexer_id: Option<&str>) -> AppResult<i32> {
        if let Some(minimum) = self.admission_minimum_seeders(indexer_id).await? {
            return Ok(minimum.max(0));
        }
        self.minimum_seeders_floor().await
    }

    /// The routing-free half of the precedence walk. See
    /// [`Self::resolve_minimum_seeders`] for why routing is excluded.
    ///
    /// A tier answers with `Some` only when it has something to say about swarm
    /// health — the mirror of [`prowlarr_managed_goal_profile`] answering only
    /// about goals. The one asymmetry is deliberate: a *profile row* answers as
    /// a unit, so an assigned profile that leaves `minimum_seeders` blank ends
    /// the walk at the floor (assigning it is how an operator overrides what
    /// Prowlarr holds), while Prowlarr's imported criteria are a bag of
    /// independent fields and only speak for the field they carry.
    async fn admission_minimum_seeders(&self, indexer_id: Option<&str>) -> AppResult<Option<i32>> {
        if let Some(indexer_id) = trimmed(indexer_id)
            && let Some(repository) = self.indexer_configs.as_ref()
            && let Some(indexer) = repository.get_by_id(&indexer_id).await?
        {
            if let Some(profile_id) = trimmed(indexer.seeding_profile_id.as_deref())
                && let Some(profile) = self.load_profile(&profile_id).await?
            {
                return Ok(profile.minimum_seeders);
            }
            if let Some(minimum) = prowlarr_managed_minimum_seeders(&indexer) {
                return Ok(Some(minimum));
            }
        }

        if let Some(profile_id) = self.default_seeding_profile_id().await?
            && let Some(profile) = self.load_profile(&profile_id).await?
        {
            return Ok(profile.minimum_seeders);
        }
        Ok(None)
    }

    /// System floor. Bootstrap seeds this from the same constant, but a missing
    /// or unparseable row still resolves to it: losing the setting must not
    /// silently turn the protection off.
    async fn minimum_seeders_floor(&self) -> AppResult<i32> {
        const DEFAULT_FLOOR: i32 = MINIMUM_SEEDERS_FLOOR_DEFAULT;
        let Some(raw_value) = self
            .settings
            .get_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                MINIMUM_SEEDERS_FLOOR_SETTING_KEY,
                None,
            )
            .await?
        else {
            return Ok(DEFAULT_FLOOR);
        };
        Ok(serde_json::from_str::<Value>(raw_value.trim())
            .ok()
            .and_then(|value| match value {
                Value::Number(number) => number.as_i64(),
                Value::String(text) => text.trim().parse::<i64>().ok(),
                _ => None,
            })
            .map_or(DEFAULT_FLOOR, |value| {
                value.clamp(0, i64::from(i32::MAX)) as i32
            }))
    }

    /// Global default profile id from the nullable settings key. Mirrors
    /// `AppUseCase::default_seeding_profile_id`, reading the repository
    /// directly so the resolver works outside the use-case facade.
    async fn default_seeding_profile_id(&self) -> AppResult<Option<String>> {
        let Some(raw_value) = self
            .settings
            .get_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                DEFAULT_SEEDING_PROFILE_SETTING_KEY,
                None,
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(parse_setting_string(&raw_value))
    }
}

/// Compute the goals for a resolved profile: season-pack overrides first, then
/// the tracker-minimum clamp.
/// Seed criteria Scryer imported from Prowlarr for a managed child indexer.
///
/// Stored inside the child's managed-metadata blob rather than as a seeding
/// profile row, so these never appear in the profile manager and cannot be
/// edited, deleted, or assigned to another indexer — they belong to Prowlarr
/// and are refreshed by the next sync.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct ProwlarrManagedSeedCriteria {
    #[serde(default)]
    seed_ratio: Option<f64>,
    #[serde(default)]
    seed_time_minutes: Option<i64>,
    #[serde(default)]
    season_pack_seed_time_minutes: Option<i64>,
    #[serde(default)]
    minimum_seeders: Option<i32>,
}

/// The indexer-reported seeder count carried on a candidate.
///
/// Plugins deliver it through the untyped `extra` map (the indexer adapter
/// writes `extra["seeders"]`), which is also where the GraphQL mapper reads it,
/// so admission and the UI agree by construction. Anything that is not a
/// number reads as unknown.
pub fn seeders_from_extra(extra: &std::collections::HashMap<String, Value>) -> Option<i64> {
    extra.get("seeders").and_then(Value::as_i64)
}

/// Whether one candidate clears the minimum-seeder bar.
///
/// The single admission rule, called from candidate evaluation *and* from
/// signed-token redemption. Redemption exists precisely to stop the API being
/// used to bypass the UI, so a second copy of this logic would be a second
/// place for the bypass to reopen.
///
/// Mirrors Sonarr's `TorrentSeedingSpecification`, which accepts on every
/// ambiguity and rejects only on a known count below a positive threshold.
/// Absent, malformed, and negative counts are all "unknown", and unknown is
/// always eligible — an indexer that does not report seeders must never have
/// its releases quietly withheld.
pub fn meets_minimum_seeders(
    source_kind: Option<DownloadSourceKind>,
    indexer_id: Option<&str>,
    seeders: Option<i64>,
    threshold: i32,
) -> bool {
    if threshold <= 0 {
        return true;
    }
    if !matches!(
        source_kind,
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri)
    ) {
        return true;
    }
    if trimmed(indexer_id).is_none() {
        return true;
    }
    match seeders {
        Some(count) if count >= 0 => count >= i64::from(threshold),
        _ => true,
    }
}

/// The seed criteria Prowlarr holds for one indexer, or `None` when this
/// indexer is not Prowlarr's to speak for.
///
/// Only a Prowlarr-managed child can carry these; a standalone indexer with a
/// stray managed-metadata blob is not Prowlarr's to interpret.
fn prowlarr_managed_criteria(indexer: &IndexerConfig) -> Option<ProwlarrManagedSeedCriteria> {
    indexer.managed_parent_config_id.as_deref()?;
    serde_json::from_str(indexer.managed_metadata_json.as_deref()?).ok()
}

/// Builds the throwaway profile that carries Prowlarr's seed **goals** through
/// the normal resolution path. The id is empty because no such profile exists;
/// callers key the resolution off `SeedGoalResolutionSource::ProwlarrManaged`.
///
/// Goals only. `minimum_seeders` is an admission question — how dead a swarm may
/// be, not how long to seed — and is answered by
/// [`prowlarr_managed_minimum_seeders`]. A child that carries only a minimum
/// therefore supplies *no* goals and must fall through to the routing entry and
/// the global default rather than short-circuiting the walk with an empty
/// profile. That is Sonarr's shape too: `SeedConfigProvider` returns `null` when
/// the indexer has no seed criteria, leaving the client's own limit regime in
/// charge (`Indexers/SeedConfigProvider.cs`).
pub fn prowlarr_managed_goal_profile(indexer: &IndexerConfig) -> Option<SeedingProfile> {
    let criteria = prowlarr_managed_criteria(indexer)?;
    let ratio = criteria
        .seed_ratio
        .filter(|value| value.is_finite() && *value > 0.0);
    let seed_time_minutes = criteria.seed_time_minutes.filter(|value| *value > 0);
    let season_pack_seed_time_minutes = criteria
        .season_pack_seed_time_minutes
        .filter(|value| *value > 0);
    if ratio.is_none() && seed_time_minutes.is_none() && season_pack_seed_time_minutes.is_none() {
        return None;
    }

    let now = Utc::now();
    Some(SeedingProfile {
        id: String::new(),
        name: "Managed by Prowlarr".to_string(),
        ratio,
        seed_time_minutes,
        // Prowlarr carries a season-pack seed time but no season-pack ratio, so
        // an override only kicks in when it actually set one.
        season_pack_mode: if season_pack_seed_time_minutes.is_some() {
            SeasonPackSeedMode::Override
        } else {
            SeasonPackSeedMode::Inherit
        },
        season_pack_ratio: None,
        season_pack_seed_time_minutes,
        // The operator's Prowlarr goals are still a floor, not a ceiling: a
        // tracker that declares a higher minimum wins, same as for a profile.
        honor_tracker_minimums: true,
        goal_met_action: SeedGoalMetAction::default(),
        never_remove: false,
        // Deliberately blank: this profile speaks for the goals walk only, and
        // leaving the field populated would invite a reader to answer admission
        // from it and re-merge the two tiers.
        minimum_seeders: None,
        post_import_tracking: PostImportTracking::default(),
        created_at: now,
        updated_at: now,
    })
}

/// Prowlarr's imported `appMinimumSeeders` for a managed child — the admission
/// half of the split above.
///
/// Deliberately NOT filtered to `> 0` the way the goal fields are. For a goal,
/// zero and unset both mean "no goal", so collapsing them is harmless. Here
/// Prowlarr's zero is a decision — disable the check — and it must not read as
/// "inherit the floor". Prowlarr's own validator only warns on a non-positive
/// value, so zero really does arrive.
///
/// Public because the interface layer reads it back for the operator: without a
/// surface, an imported minimum governs admission while the indexer row still
/// reads "Inherit default".
pub fn prowlarr_managed_minimum_seeders(indexer: &IndexerConfig) -> Option<i32> {
    prowlarr_managed_criteria(indexer)?
        .minimum_seeders
        .filter(|value| *value >= 0)
}

fn apply_profile(
    profile: &SeedingProfile,
    request: &SeedGoalRequest,
    source: SeedGoalResolutionSource,
) -> ResolvedSeedGoals {
    let mut ratio = profile.effective_ratio(request.season_pack);
    let mut seed_time_minutes = profile.effective_seed_time_minutes(request.season_pack);

    if profile.honor_tracker_minimums {
        // Clamp UP only: a tracker minimum can raise a goal but never lower it,
        // and a minimum on an axis the profile leaves unset becomes the goal on
        // that axis (otherwise the tracker's H&R rule would go unenforced).
        let profile_ratio = ratio;
        let profile_seed_time_minutes = seed_time_minutes;
        let min_ratio = request.effective_min_ratio();
        let min_seed_time_minutes = request.effective_min_seed_time_minutes();
        ratio = clamp_up_f64(profile_ratio, min_ratio);
        seed_time_minutes = clamp_up_i64(profile_seed_time_minutes, min_seed_time_minutes);
        log_tracker_minimum_clamp(
            profile,
            request,
            source,
            TrackerMinimumClamp {
                profile_ratio,
                min_ratio,
                resolved_ratio: ratio,
                profile_seed_time_minutes,
                min_seed_time_minutes,
                resolved_seed_time_minutes: seed_time_minutes,
            },
        );
    }

    ResolvedSeedGoals {
        // Prowlarr-managed criteria are synthesized, so there is no profile row
        // to point at; the resolution source records where they came from.
        seeding_profile_id: (!profile.id.is_empty()).then(|| profile.id.clone()),
        seed_goal_ratio: ratio.filter(|value| value.is_finite() && *value > 0.0),
        seed_goal_seconds: seed_time_minutes
            .filter(|minutes| *minutes > 0)
            .and_then(|minutes| minutes.checked_mul(60)),
        never_remove: profile.never_remove,
        goal_met_action: Some(profile.goal_met_action),
        post_import_tracking: profile.post_import_tracking,
        resolution_source: source,
    }
}

/// Which axes a tracker minimum actually raised, as the single `axes` field of
/// the breadcrumb — `None` when nothing was raised and there is nothing to say.
///
/// Split out from the log call so the decision is unit-testable without a
/// subscriber: `scryer-application` has no log-capture harness, and the value
/// worth pinning is which axes count as clamped, not that `tracing` works.
///
/// Derived from the inputs rather than by comparing the goal before and after,
/// so a non-finite or non-positive profile value can never read as "clamped".
fn clamped_axes(clamp: &TrackerMinimumClamp) -> Option<&'static str> {
    let ratio_clamped = clamp
        .min_ratio
        .filter(|value| value.is_finite() && *value > 0.0)
        .is_some_and(|minimum| {
            clamp
                .profile_ratio
                .filter(|value| value.is_finite())
                .is_none_or(|value| minimum > value)
        });
    let seed_time_clamped = clamp
        .min_seed_time_minutes
        .filter(|minutes| *minutes > 0)
        .is_some_and(|minimum| {
            clamp
                .profile_seed_time_minutes
                .is_none_or(|value| minimum > value)
        });

    match (ratio_clamped, seed_time_clamped) {
        (true, true) => Some("ratio,seed_time"),
        (true, false) => Some("ratio"),
        (false, true) => Some("seed_time"),
        (false, false) => None,
    }
}

/// Both axes of one clamp, before and after, for the operator breadcrumb.
struct TrackerMinimumClamp {
    profile_ratio: Option<f64>,
    min_ratio: Option<f64>,
    resolved_ratio: Option<f64>,
    profile_seed_time_minutes: Option<i64>,
    min_seed_time_minutes: Option<i64>,
    resolved_seed_time_minutes: Option<i64>,
}

/// One structured line per grab when a tracker-declared minimum raised a goal
/// above the profile's value, or supplied a goal on an axis the profile leaves
/// unset.
///
/// This is the operator-facing evidence that hit-and-run protection engaged:
/// the goals a torrent is actually seeding to are frozen at grab time, so
/// without this line the only way to explain a goal that does not match the
/// profile is to read the submission row. Deliberately one event covering both
/// axes — a clamp is a single decision about one grab, not one per axis — and
/// silent when nothing was raised, so it stays a signal rather than per-grab
/// noise.
fn log_tracker_minimum_clamp(
    profile: &SeedingProfile,
    request: &SeedGoalRequest,
    source: SeedGoalResolutionSource,
    clamp: TrackerMinimumClamp,
) {
    let Some(axes) = clamped_axes(&clamp) else {
        return;
    };

    tracing::info!(
        indexer_id = request.indexer_id.as_deref().unwrap_or("unknown"),
        seeding_profile_id = profile.id.as_str(),
        seeding_profile = profile.name.as_str(),
        resolution_source = source.as_str(),
        season_pack = request.season_pack,
        axes,
        profile_ratio = ?clamp.profile_ratio,
        tracker_min_ratio = ?clamp.min_ratio,
        resolved_ratio = ?clamp.resolved_ratio,
        profile_seed_time_minutes = ?clamp.profile_seed_time_minutes,
        tracker_min_seed_time_minutes = ?clamp.min_seed_time_minutes,
        resolved_seed_time_minutes = ?clamp.resolved_seed_time_minutes,
        "seeding goal raised to the tracker-declared minimum (hit-and-run protection)"
    );
}

fn clamp_up_f64(value: Option<f64>, minimum: Option<f64>) -> Option<f64> {
    let minimum = minimum.filter(|value| value.is_finite() && *value > 0.0);
    match (value, minimum) {
        (Some(value), Some(minimum)) => Some(value.max(minimum)),
        (Some(value), None) => Some(value),
        (None, minimum) => minimum,
    }
}

fn clamp_up_i64(value: Option<i64>, minimum: Option<i64>) -> Option<i64> {
    let minimum = minimum.filter(|minutes| *minutes > 0);
    match (value, minimum) {
        (Some(value), Some(minimum)) => Some(value.max(minimum)),
        (Some(value), None) => Some(value),
        (None, minimum) => minimum,
    }
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Settings values are stored as JSON; the key holds either `null` or a quoted
/// id. Tolerate a bare (unquoted) id too, the way the quality-profile reader
/// does.
fn parse_setting_string(raw_value: &str) -> Option<String> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Null) => None,
        Ok(Value::String(value)) => {
            let normalized = value.trim();
            (!normalized.is_empty()).then(|| normalized.to_string())
        }
        Ok(_) => Some(trimmed.to_string()),
        Err(_) => Some(trimmed.to_string()),
    }
}

/// Read a tracker-declared minimum out of a release `extra` map. The indexer
/// adapter writes these as JSON numbers, but Torznab feeds proxied through
/// plugins sometimes stringify them, so both shapes are accepted.
pub fn release_extra_f64(
    extra: &std::collections::HashMap<String, Value>,
    key: &str,
) -> Option<f64> {
    match extra.get(key)? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite() && *value > 0.0)
}

pub fn release_extra_i64(
    extra: &std::collections::HashMap<String, Value>,
    key: &str,
) -> Option<i64> {
    match extra.get(key)? {
        Value::Number(value) => value
            .as_i64()
            .or_else(|| value.as_f64().map(|value| value.round() as i64)),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
    .filter(|value| *value > 0)
}

/// Tracker minimums lifted off a release `extra` map, in the order the indexer
/// adapter writes them (`indexer_adapter.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReleaseSeedMinimums {
    pub min_seed_ratio: Option<f64>,
    pub min_seed_time_minutes: Option<i64>,
    pub season_pack_seed_ratio: Option<f64>,
    pub season_pack_seed_time_minutes: Option<i64>,
}

impl ReleaseSeedMinimums {
    pub fn from_release_extra(extra: &std::collections::HashMap<String, Value>) -> Self {
        Self {
            min_seed_ratio: release_extra_f64(extra, "minimum_seed_ratio"),
            min_seed_time_minutes: release_extra_i64(extra, "minimum_seed_time_minutes"),
            season_pack_seed_ratio: release_extra_f64(extra, "season_pack_seed_ratio"),
            season_pack_seed_time_minutes: release_extra_i64(
                extra,
                "season_pack_seed_time_minutes",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_domain::{IndexerConfig, SeasonPackSeedMode};

    use super::*;
    use crate::{AppError, IndexerConfigUpdate, IndexerSystemBackoff};

    struct FakeSeedingProfiles {
        profiles: Vec<SeedingProfile>,
    }

    #[async_trait]
    impl SeedingProfileRepository for FakeSeedingProfiles {
        async fn list(&self) -> AppResult<Vec<SeedingProfile>> {
            Ok(self.profiles.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<SeedingProfile>> {
            Ok(self
                .profiles
                .iter()
                .find(|profile| profile.id == id)
                .cloned())
        }

        async fn create(&self, profile: SeedingProfile) -> AppResult<SeedingProfile> {
            Ok(profile)
        }

        async fn update(&self, profile: SeedingProfile) -> AppResult<SeedingProfile> {
            Ok(profile)
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct FakeIndexerConfigs {
        indexers: Vec<IndexerConfig>,
    }

    #[async_trait]
    impl IndexerConfigRepository for FakeIndexerConfigs {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.indexers.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(self
                .indexers
                .iter()
                .find(|indexer| indexer.id == id)
                .cloned())
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }

        async fn list_system_backoffs(&self) -> AppResult<HashMap<String, IndexerSystemBackoff>> {
            Ok(HashMap::new())
        }

        async fn update(&self, _update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::NotFound("not implemented".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct FakeSettings {
        values: HashMap<String, String>,
    }

    #[async_trait]
    impl SettingsRepository for FakeSettings {
        async fn get_setting_json(
            &self,
            _scope: &str,
            key_name: &str,
            _scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(self.values.get(key_name).cloned())
        }

        async fn get_setting_json_explicit(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            self.get_setting_json(scope, key_name, scope_id).await
        }

        async fn list_setting_json_explicit_for_scope_ids(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_ids: &[String],
        ) -> AppResult<Vec<(String, String)>> {
            Ok(Vec::new())
        }

        async fn upsert_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
            _value_json: String,
            _source: &str,
            _updated_by: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn delete_setting_value(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn delete_values_for_scope_id(&self, _scope_id: &str) -> AppResult<u32> {
            Ok(0)
        }
    }

    fn profile(id: &str, ratio: Option<f64>, seed_time_minutes: Option<i64>) -> SeedingProfile {
        let now = Utc::now();
        SeedingProfile {
            id: id.to_string(),
            name: id.to_string(),
            ratio,
            seed_time_minutes,
            season_pack_mode: SeasonPackSeedMode::Inherit,
            season_pack_ratio: None,
            season_pack_seed_time_minutes: None,
            honor_tracker_minimums: true,
            goal_met_action: SeedGoalMetAction::RemoveEntry,
            never_remove: false,
            minimum_seeders: None,
            post_import_tracking: PostImportTracking::Park,
            created_at: now,
            updated_at: now,
        }
    }

    fn indexer(id: &str, seeding_profile_id: Option<&str>) -> IndexerConfig {
        let now = Utc::now();
        IndexerConfig {
            id: id.to_string(),
            name: id.to_string(),
            provider_type: "torznab".to_string(),
            base_url: "https://example.invalid".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: seeding_profile_id.map(str::to_string),
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn resolver(
        profiles: Vec<SeedingProfile>,
        indexers: Vec<IndexerConfig>,
        default_profile_id: Option<&str>,
    ) -> SeedGoalResolver {
        let mut values = HashMap::new();
        if let Some(profile_id) = default_profile_id {
            values.insert(
                DEFAULT_SEEDING_PROFILE_SETTING_KEY.to_string(),
                serde_json::Value::String(profile_id.to_string()).to_string(),
            );
        }
        SeedGoalResolver::new(
            Arc::new(FakeSeedingProfiles { profiles }),
            Some(Arc::new(FakeIndexerConfigs { indexers })),
            Arc::new(FakeSettings { values }),
        )
    }

    fn request(indexer_id: Option<&str>, routing_profile_id: Option<&str>) -> SeedGoalRequest {
        SeedGoalRequest {
            indexer_id: indexer_id.map(str::to_string),
            routing_seeding_profile_id: routing_profile_id.map(str::to_string),
            ..SeedGoalRequest::default()
        }
    }

    /// A Prowlarr-managed child carrying imported seed criteria.
    fn prowlarr_child(
        id: &str,
        seeding_profile_id: Option<&str>,
        metadata: serde_json::Value,
    ) -> IndexerConfig {
        let mut config = indexer(id, seeding_profile_id);
        config.managed_parent_config_id = Some("prowlarr-parent".to_string());
        config.managed_metadata_json = Some(metadata.to_string());
        config
    }

    fn profile_with_minimum(id: &str, minimum_seeders: Option<i32>) -> SeedingProfile {
        SeedingProfile {
            minimum_seeders,
            ..profile(id, Some(1.0), None)
        }
    }

    fn resolver_with_floor(
        profiles: Vec<SeedingProfile>,
        indexers: Vec<IndexerConfig>,
        default_profile_id: Option<&str>,
        floor: Option<&str>,
    ) -> SeedGoalResolver {
        let mut values = HashMap::new();
        if let Some(profile_id) = default_profile_id {
            values.insert(
                DEFAULT_SEEDING_PROFILE_SETTING_KEY.to_string(),
                serde_json::Value::String(profile_id.to_string()).to_string(),
            );
        }
        if let Some(floor) = floor {
            values.insert(
                MINIMUM_SEEDERS_FLOOR_SETTING_KEY.to_string(),
                floor.to_string(),
            );
        }
        SeedGoalResolver::new(
            Arc::new(FakeSeedingProfiles { profiles }),
            Some(Arc::new(FakeIndexerConfigs { indexers })),
            Arc::new(FakeSettings { values }),
        )
    }

    #[tokio::test]
    async fn minimum_seeders_prefers_the_indexer_profile() {
        let resolver = resolver_with_floor(
            vec![profile_with_minimum("assigned", Some(5))],
            vec![indexer("idx", Some("assigned"))],
            None,
            Some("1"),
        );
        assert_eq!(
            resolver.resolve_minimum_seeders(Some("idx")).await.unwrap(),
            5
        );
    }

    #[tokio::test]
    async fn a_profile_zero_disables_the_check_rather_than_inheriting() {
        let resolver = resolver_with_floor(
            vec![profile_with_minimum("assigned", Some(0))],
            vec![indexer("idx", Some("assigned"))],
            None,
            Some("3"),
        );
        assert_eq!(
            resolver.resolve_minimum_seeders(Some("idx")).await.unwrap(),
            0,
            "an explicit 0 must not fall through to the floor"
        );
    }

    #[tokio::test]
    async fn a_profile_without_a_minimum_inherits_the_floor() {
        let resolver = resolver_with_floor(
            vec![profile_with_minimum("assigned", None)],
            vec![indexer("idx", Some("assigned"))],
            None,
            Some("4"),
        );
        assert_eq!(
            resolver.resolve_minimum_seeders(Some("idx")).await.unwrap(),
            4
        );
    }

    #[tokio::test]
    async fn a_missing_floor_row_still_resolves_to_one() {
        let resolver = resolver_with_floor(Vec::new(), Vec::new(), None, None);
        assert_eq!(
            resolver.resolve_minimum_seeders(Some("idx")).await.unwrap(),
            1,
            "losing the setting must not turn the protection off"
        );
    }

    #[tokio::test]
    async fn an_explicit_zero_floor_disables_the_check_globally() {
        let resolver = resolver_with_floor(Vec::new(), Vec::new(), None, Some("0"));
        assert_eq!(
            resolver.resolve_minimum_seeders(Some("idx")).await.unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn a_dangling_profile_assignment_falls_through_to_the_floor() {
        let resolver = resolver_with_floor(
            Vec::new(),
            vec![indexer("idx", Some("deleted-profile"))],
            None,
            Some("2"),
        );
        assert_eq!(
            resolver.resolve_minimum_seeders(Some("idx")).await.unwrap(),
            2,
            "a dangling assignment must never fail the search"
        );
    }

    #[tokio::test]
    async fn a_routing_only_profile_cannot_supply_an_admission_threshold() {
        // The grab-time walk has a routing tier; admission runs before a route
        // is chosen, so a profile reachable only through routing must not apply
        // here. Nothing else in this module makes that divergence visible.
        let resolver = resolver_with_floor(
            vec![profile_with_minimum("routing-only", Some(9))],
            vec![indexer("idx", None)],
            None,
            Some("1"),
        );
        let goals = resolver
            .resolve(&request(Some("idx"), Some("routing-only")))
            .await
            .unwrap();
        assert_eq!(
            goals.resolution_source,
            SeedGoalResolutionSource::RoutingEntry
        );
        assert_eq!(
            resolver.resolve_minimum_seeders(Some("idx")).await.unwrap(),
            1,
            "routing supplied the seed goals but must not supply the threshold"
        );
    }

    #[test]
    fn a_managed_child_carrying_only_a_minimum_supplies_admission_but_no_goals() {
        let indexer = prowlarr_child(
            "idx-managed",
            None,
            serde_json::json!({ "indexer_id": 7, "minimum_seeders": 4 }),
        );
        assert!(
            prowlarr_managed_goal_profile(&indexer).is_none(),
            "a minimum is not a seed goal: the goals walk must keep going"
        );
        assert_eq!(prowlarr_managed_minimum_seeders(&indexer), Some(4));
    }

    #[test]
    fn a_managed_child_minimum_of_zero_survives_as_an_explicit_disable() {
        let indexer = prowlarr_child(
            "idx-managed",
            None,
            serde_json::json!({ "indexer_id": 7, "minimum_seeders": 0 }),
        );
        assert_eq!(
            prowlarr_managed_minimum_seeders(&indexer),
            Some(0),
            "zero is a decision, not an absent field"
        );
    }

    #[test]
    fn only_a_managed_child_speaks_for_prowlarr_on_either_axis() {
        let mut standalone = indexer("idx-standalone", None);
        standalone.managed_metadata_json =
            Some(serde_json::json!({ "seed_ratio": 9.0, "minimum_seeders": 9 }).to_string());
        assert!(prowlarr_managed_goal_profile(&standalone).is_none());
        assert_eq!(prowlarr_managed_minimum_seeders(&standalone), None);
    }

    #[tokio::test]
    async fn a_managed_child_with_only_a_minimum_still_inherits_the_global_default_goals() {
        // The regression this split fixes: a child carrying nothing but
        // `appMinimumSeeders` used to short-circuit the goals walk with an empty
        // ProwlarrManaged profile, so the operator's global default was silently
        // not applied. Sonarr's shape is `seedCriteria == null` → next regime.
        let resolver = resolver_with_floor(
            vec![profile("global-profile", Some(2.0), Some(60))],
            vec![prowlarr_child(
                "idx-managed",
                None,
                serde_json::json!({ "indexer_id": 7, "minimum_seeders": 4 }),
            )],
            Some("global-profile"),
            Some("1"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-managed"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::GlobalDefault
        );
        assert_eq!(resolved.seed_goal_ratio, Some(2.0));
        assert_eq!(resolved.seed_goal_seconds, Some(3600));
        assert_eq!(
            resolver
                .resolve_minimum_seeders(Some("idx-managed"))
                .await
                .unwrap(),
            4,
            "admission still uses the minimum Prowlarr imported"
        );
    }

    #[tokio::test]
    async fn a_managed_child_with_only_a_minimum_still_inherits_the_routing_entry_goals() {
        let resolver = resolver_with_floor(
            vec![profile("routing-profile", Some(1.0), None)],
            vec![prowlarr_child(
                "idx-managed",
                None,
                serde_json::json!({ "indexer_id": 7, "minimum_seeders": 4 }),
            )],
            None,
            Some("1"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-managed"), Some("routing-profile")))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::RoutingEntry
        );
        assert_eq!(resolved.seed_goal_ratio, Some(1.0));
    }

    #[tokio::test]
    async fn a_private_torrent_on_a_minimum_only_child_is_not_held_by_the_private_rail() {
        // The operator-visible harm behind the split: with no goals resolved, an
        // observed private torrent hits the hard private rail and is held
        // forever — the opposite of what the default profile was configured to
        // do. Wired through the real gate so the two halves cannot drift.
        use crate::import::seeding_gate::{
            SeedingGateInput, SeedingGateOutcome, TorrentSeedingObservation, evaluate_seeding_gate,
            reason,
        };

        let resolver = resolver_with_floor(
            vec![profile("global-profile", Some(2.0), None)],
            vec![prowlarr_child(
                "idx-managed",
                None,
                serde_json::json!({ "indexer_id": 7, "minimum_seeders": 4 }),
            )],
            Some("global-profile"),
            Some("1"),
        );
        let resolved = resolver
            .resolve(&request(Some("idx-managed"), None))
            .await
            .expect("resolution should succeed");
        assert!(resolved.has_goals(), "the default profile supplies goals");

        // Field for field the way the router persists a resolution
        // (`downloads/clients/router.rs`), so the gate sees production's shape.
        let decision = evaluate_seeding_gate(&SeedingGateInput {
            client_type: "qbittorrent".to_string(),
            observation: Some(TorrentSeedingObservation {
                is_private: Some(true),
                seed_ratio: Some(3.0),
                ..TorrentSeedingObservation::default()
            }),
            goals: Some(crate::PersistedSeedGoals {
                seeding_profile_id: resolved.seeding_profile_id.clone(),
                seed_goal_ratio: resolved.seed_goal_ratio,
                seed_goal_seconds: resolved.seed_goal_seconds,
                never_remove: resolved.never_remove,
                goal_met_action: resolved.goal_met_action,
                post_import_tracking: resolved.post_import_tracking,
                resolution_source: resolved.resolution_source,
                info_hash: None,
            }),
            ..SeedingGateInput::default()
        });

        assert_ne!(decision.reason, reason::PRIVATE_WITHOUT_GOALS);
        assert_eq!(
            decision.outcome,
            SeedingGateOutcome::Released {
                action: SeedGoalMetAction::RemoveEntry
            }
        );
    }

    #[tokio::test]
    async fn prowlarr_goals_without_a_minimum_leave_admission_to_the_next_tier() {
        // The mirror image of the split: Prowlarr supplies goals but says
        // nothing about swarm health, so the operator's default profile answers
        // the admission question rather than the bare floor.
        let resolver = resolver_with_floor(
            vec![SeedingProfile {
                minimum_seeders: Some(6),
                ..profile("global-profile", Some(2.0), None)
            }],
            vec![prowlarr_child(
                "idx-managed",
                None,
                serde_json::json!({ "indexer_id": 7, "seed_ratio": 1.5 }),
            )],
            Some("global-profile"),
            Some("1"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-managed"), None))
            .await
            .expect("resolution should succeed");
        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::ProwlarrManaged
        );
        assert_eq!(resolved.seed_goal_ratio, Some(1.5));
        assert_eq!(
            resolver
                .resolve_minimum_seeders(Some("idx-managed"))
                .await
                .unwrap(),
            6
        );
    }

    #[tokio::test]
    async fn an_assigned_profile_without_a_minimum_inherits_the_floor_not_the_next_tier() {
        // First profile wins on the admission walk too: assigning a Scryer
        // profile is how an operator overrides what Prowlarr holds, so one that
        // leaves the field blank must not reach past itself to the default.
        let resolver = resolver_with_floor(
            vec![
                profile_with_minimum("assigned", None),
                profile_with_minimum("global-profile", Some(9)),
            ],
            vec![prowlarr_child(
                "idx-managed",
                Some("assigned"),
                serde_json::json!({ "indexer_id": 7, "minimum_seeders": 8 }),
            )],
            Some("global-profile"),
            Some("2"),
        );

        assert_eq!(
            resolver
                .resolve_minimum_seeders(Some("idx-managed"))
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn an_assigned_profile_overrides_what_prowlarr_holds() {
        let resolver = resolver_with_floor(
            vec![profile_with_minimum("scryer-owned", Some(2))],
            vec![prowlarr_child(
                "idx-managed",
                Some("scryer-owned"),
                serde_json::json!({ "indexer_id": 7, "minimum_seeders": 8 }),
            )],
            None,
            Some("1"),
        );
        assert_eq!(
            resolver
                .resolve_minimum_seeders(Some("idx-managed"))
                .await
                .unwrap(),
            2
        );
    }

    #[test]
    fn admission_accepts_on_every_ambiguity() {
        // Mirrors Sonarr: reject only a known count below a positive threshold.
        assert!(!meets_minimum_seeders(
            Some(DownloadSourceKind::TorrentFile),
            Some("idx"),
            Some(0),
            1
        ));
        assert!(meets_minimum_seeders(
            Some(DownloadSourceKind::TorrentFile),
            Some("idx"),
            Some(1),
            1
        ));
        assert!(meets_minimum_seeders(
            Some(DownloadSourceKind::MagnetUri),
            Some("idx"),
            Some(5),
            3
        ));
        assert!(!meets_minimum_seeders(
            Some(DownloadSourceKind::MagnetUri),
            Some("idx"),
            Some(2),
            3
        ));
        // threshold disabled
        assert!(meets_minimum_seeders(
            Some(DownloadSourceKind::TorrentFile),
            Some("idx"),
            Some(0),
            0
        ));
        // unknown, negative, non-torrent, and unattributed all stay eligible
        assert!(meets_minimum_seeders(
            Some(DownloadSourceKind::TorrentFile),
            Some("idx"),
            None,
            5
        ));
        assert!(meets_minimum_seeders(
            Some(DownloadSourceKind::TorrentFile),
            Some("idx"),
            Some(-1),
            5
        ));
        assert!(meets_minimum_seeders(
            Some(DownloadSourceKind::NzbUrl),
            Some("idx"),
            Some(0),
            5
        ));
        assert!(meets_minimum_seeders(
            Some(DownloadSourceKind::TorrentFile),
            None,
            Some(0),
            5
        ));
        assert!(meets_minimum_seeders(
            Some(DownloadSourceKind::TorrentFile),
            Some("   "),
            Some(0),
            5
        ));
    }

    #[test]
    fn seeder_counts_come_from_the_same_place_the_ui_reads() {
        let mut extra = std::collections::HashMap::new();
        extra.insert("seeders".to_string(), serde_json::json!(12));
        assert_eq!(seeders_from_extra(&extra), Some(12));
        extra.insert("seeders".to_string(), serde_json::json!("not a number"));
        assert_eq!(seeders_from_extra(&extra), None);
        assert_eq!(seeders_from_extra(&std::collections::HashMap::new()), None);
    }

    #[tokio::test]
    async fn prowlarr_seed_criteria_apply_when_the_child_has_no_profile() {
        let resolver = resolver(
            vec![profile("global-profile", Some(0.5), None)],
            vec![prowlarr_child(
                "idx-managed",
                None,
                serde_json::json!({
                    "indexer_id": 7,
                    "seed_ratio": 1.5,
                    "seed_time_minutes": 4320,
                }),
            )],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-managed"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::ProwlarrManaged
        );
        // Synthesized, so there is no profile row to point at.
        assert_eq!(resolved.seeding_profile_id, None);
        assert_eq!(resolved.seed_goal_ratio, Some(1.5));
        assert_eq!(resolved.seed_goal_seconds, Some(4320 * 60));
    }

    #[tokio::test]
    async fn a_scryer_profile_overrides_the_prowlarr_criteria() {
        let resolver = resolver(
            vec![profile("indexer-profile", Some(3.0), None)],
            vec![prowlarr_child(
                "idx-managed",
                Some("indexer-profile"),
                serde_json::json!({ "indexer_id": 7, "seed_ratio": 1.5 }),
            )],
            None,
        );

        let resolved = resolver
            .resolve(&request(Some("idx-managed"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::Indexer
        );
        assert_eq!(resolved.seed_goal_ratio, Some(3.0));
    }

    #[tokio::test]
    async fn a_managed_child_without_prowlarr_goals_falls_through_to_the_default() {
        let resolver = resolver(
            vec![profile("global-profile", Some(0.5), None)],
            vec![prowlarr_child(
                "idx-managed",
                None,
                // Prowlarr left the seed criteria blank for this tracker.
                serde_json::json!({ "indexer_id": 7 }),
            )],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-managed"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::GlobalDefault
        );
        assert_eq!(resolved.seed_goal_ratio, Some(0.5));
    }

    #[tokio::test]
    async fn seed_criteria_on_a_standalone_indexer_are_ignored() {
        // Only Prowlarr speaks for a managed child; a stray blob on an
        // unmanaged indexer is not Prowlarr's to interpret.
        let mut standalone = indexer("idx-standalone", None);
        standalone.managed_metadata_json =
            Some(serde_json::json!({ "seed_ratio": 9.0 }).to_string());
        let resolver = resolver(Vec::new(), vec![standalone], None);

        let resolved = resolver
            .resolve(&request(Some("idx-standalone"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.resolution_source, SeedGoalResolutionSource::None);
        assert_eq!(resolved.seed_goal_ratio, None);
    }

    #[tokio::test]
    async fn tracker_minimums_still_raise_prowlarr_criteria() {
        let resolver = resolver(
            Vec::new(),
            vec![prowlarr_child(
                "idx-managed",
                None,
                serde_json::json!({ "indexer_id": 7, "seed_ratio": 1.0 }),
            )],
            None,
        );
        let mut req = request(Some("idx-managed"), None);
        req.tracker_min_seed_ratio = Some(2.0);

        let resolved = resolver
            .resolve(&req)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(2.0));
    }

    #[tokio::test]
    async fn indexer_assignment_beats_routing_entry_and_global_default() {
        let resolver = resolver(
            vec![
                profile("indexer-profile", Some(2.0), None),
                profile("routing-profile", Some(1.0), None),
                profile("global-profile", Some(0.5), None),
            ],
            vec![indexer("idx-1", Some("indexer-profile"))],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), Some("routing-profile")))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.seeding_profile_id.as_deref(),
            Some("indexer-profile")
        );
        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::Indexer
        );
        assert_eq!(resolved.seed_goal_ratio, Some(2.0));
    }

    #[tokio::test]
    async fn routing_entry_beats_global_default_when_the_indexer_has_no_profile() {
        let resolver = resolver(
            vec![
                profile("routing-profile", Some(1.0), None),
                profile("global-profile", Some(0.5), None),
            ],
            vec![indexer("idx-1", None)],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), Some("routing-profile")))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.seeding_profile_id.as_deref(),
            Some("routing-profile")
        );
        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::RoutingEntry
        );
    }

    #[tokio::test]
    async fn global_default_applies_when_nothing_else_is_assigned() {
        let resolver = resolver(
            vec![profile("global-profile", Some(0.5), Some(60))],
            vec![indexer("idx-1", None)],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::GlobalDefault
        );
        assert_eq!(resolved.seed_goal_ratio, Some(0.5));
        assert_eq!(resolved.seed_goal_seconds, Some(3600));
    }

    #[tokio::test]
    async fn no_assignment_anywhere_resolves_to_no_goals() {
        let resolver = resolver(
            vec![profile("unused", Some(3.0), Some(120))],
            vec![indexer("idx-1", None)],
            None,
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), None))
            .await
            .expect("resolution should succeed");

        assert!(!resolved.is_resolved());
        assert_eq!(resolved, ResolvedSeedGoals::default());
        assert_eq!(resolved.seeding_profile_id, None);
        assert_eq!(resolved.seed_goal_ratio, None);
        assert_eq!(resolved.seed_goal_seconds, None);
        assert_eq!(resolved.goal_met_action, None);
        assert!(!resolved.never_remove);
        // No profile means Scryer keeps managing the torrent — the fail-closed
        // direction, and what every install did before this feature existed.
        assert_eq!(resolved.post_import_tracking, PostImportTracking::Park);
    }

    #[tokio::test]
    async fn a_dangling_assignment_falls_through_to_the_next_level() {
        let resolver = resolver(
            vec![profile("global-profile", Some(0.5), None)],
            vec![indexer("idx-1", Some("deleted-profile"))],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), Some("also-deleted")))
            .await
            .expect("a dangling assignment must not fail the grab");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::GlobalDefault
        );
    }

    #[tokio::test]
    async fn tracker_minimums_clamp_goals_up_but_never_down() {
        let resolver = resolver(
            vec![profile("p", Some(1.0), Some(60))],
            vec![indexer("idx-1", Some("p"))],
            None,
        );

        let mut goal_request = request(Some("idx-1"), None);
        goal_request.tracker_min_seed_ratio = Some(2.5);
        goal_request.tracker_min_seed_time_minutes = Some(30);

        let resolved = resolver
            .resolve(&goal_request)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(2.5));
        // The profile's 60 minutes already clears the 30-minute minimum.
        assert_eq!(resolved.seed_goal_seconds, Some(3600));
    }

    #[tokio::test]
    async fn a_tracker_minimum_becomes_the_goal_on_an_axis_the_profile_leaves_unset() {
        let resolver = resolver(
            vec![profile("p", Some(1.0), None)],
            vec![indexer("idx-1", Some("p"))],
            None,
        );

        let mut goal_request = request(Some("idx-1"), None);
        goal_request.tracker_min_seed_time_minutes = Some(4320);

        let resolved = resolver
            .resolve(&goal_request)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(1.0));
        assert_eq!(resolved.seed_goal_seconds, Some(4320 * 60));
    }

    #[tokio::test]
    async fn tracker_minimums_are_ignored_when_the_profile_opts_out() {
        let mut opted_out = profile("p", Some(1.0), None);
        opted_out.honor_tracker_minimums = false;
        let resolver = resolver(vec![opted_out], vec![indexer("idx-1", Some("p"))], None);

        let mut goal_request = request(Some("idx-1"), None);
        goal_request.tracker_min_seed_ratio = Some(2.5);
        goal_request.tracker_min_seed_time_minutes = Some(4320);

        let resolved = resolver
            .resolve(&goal_request)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(1.0));
        assert_eq!(resolved.seed_goal_seconds, None);
    }

    #[tokio::test]
    async fn season_pack_override_selects_the_pack_goals_and_pack_minimums() {
        let mut pack_profile = profile("p", Some(1.0), Some(60));
        pack_profile.season_pack_mode = SeasonPackSeedMode::Override;
        pack_profile.season_pack_ratio = Some(2.0);
        pack_profile.season_pack_seed_time_minutes = Some(120);
        let resolver = resolver(vec![pack_profile], vec![indexer("idx-1", Some("p"))], None);

        let mut episode_request = request(Some("idx-1"), None);
        episode_request.tracker_min_seed_ratio = Some(0.1);
        episode_request.season_pack_min_seed_ratio = Some(9.0);
        let episode = resolver
            .resolve(&episode_request)
            .await
            .expect("resolution should succeed");
        assert_eq!(episode.seed_goal_ratio, Some(1.0));
        assert_eq!(episode.seed_goal_seconds, Some(3600));

        let mut pack_request = episode_request.clone();
        pack_request.season_pack = true;
        let pack = resolver
            .resolve(&pack_request)
            .await
            .expect("resolution should succeed");
        // Pack goals win over the base goals, then the pack minimum clamps up.
        assert_eq!(pack.seed_goal_ratio, Some(9.0));
        assert_eq!(pack.seed_goal_seconds, Some(120 * 60));
    }

    #[tokio::test]
    async fn season_pack_inherit_mode_keeps_the_base_goals() {
        let resolver = resolver(
            vec![profile("p", Some(1.0), Some(60))],
            vec![indexer("idx-1", Some("p"))],
            None,
        );

        let mut pack_request = request(Some("idx-1"), None);
        pack_request.season_pack = true;
        let resolved = resolver
            .resolve(&pack_request)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(1.0));
        assert_eq!(resolved.seed_goal_seconds, Some(3600));
    }

    #[tokio::test]
    async fn profile_policy_flags_ride_along_with_the_goals() {
        let mut kept = profile("p", None, None);
        kept.never_remove = true;
        kept.goal_met_action = SeedGoalMetAction::StopSeeding;
        kept.post_import_tracking = PostImportTracking::HandOff;
        let resolver = resolver(vec![kept], vec![indexer("idx-1", Some("p"))], None);

        let resolved = resolver
            .resolve(&request(Some("idx-1"), None))
            .await
            .expect("resolution should succeed");

        assert!(resolved.is_resolved());
        assert!(!resolved.has_goals());
        assert!(resolved.never_remove);
        assert_eq!(
            resolved.goal_met_action,
            Some(SeedGoalMetAction::StopSeeding)
        );
        // Frozen with the goals: a torrent keeps the tracking mode it was
        // grabbed under even if the profile is later edited.
        assert_eq!(resolved.post_import_tracking, PostImportTracking::HandOff);
    }

    #[test]
    fn release_minimums_are_read_from_the_indexer_extra_map() {
        let mut extra = HashMap::new();
        extra.insert("minimum_seed_ratio".to_string(), serde_json::json!(1.25));
        extra.insert(
            "minimum_seed_time_minutes".to_string(),
            serde_json::json!(4320),
        );
        // Some plugins stringify torznab attrs; both shapes must read.
        extra.insert(
            "season_pack_seed_ratio".to_string(),
            serde_json::json!("2.5"),
        );
        extra.insert(
            "season_pack_seed_time_minutes".to_string(),
            serde_json::json!("10080"),
        );
        extra.insert(
            "minimum_seed_ratio_unused".to_string(),
            serde_json::json!(0),
        );

        let minimums = ReleaseSeedMinimums::from_release_extra(&extra);
        assert_eq!(minimums.min_seed_ratio, Some(1.25));
        assert_eq!(minimums.min_seed_time_minutes, Some(4320));
        assert_eq!(minimums.season_pack_seed_ratio, Some(2.5));
        assert_eq!(minimums.season_pack_seed_time_minutes, Some(10080));

        assert_eq!(
            ReleaseSeedMinimums::from_release_extra(&HashMap::new()),
            ReleaseSeedMinimums::default()
        );
    }

    fn clamp(
        profile_ratio: Option<f64>,
        min_ratio: Option<f64>,
        profile_seed_time_minutes: Option<i64>,
        min_seed_time_minutes: Option<i64>,
    ) -> TrackerMinimumClamp {
        TrackerMinimumClamp {
            profile_ratio,
            min_ratio,
            resolved_ratio: clamp_up_f64(profile_ratio, min_ratio),
            profile_seed_time_minutes,
            min_seed_time_minutes,
            resolved_seed_time_minutes: clamp_up_i64(
                profile_seed_time_minutes,
                min_seed_time_minutes,
            ),
        }
    }

    #[test]
    fn a_raised_axis_is_reported_as_a_clamp() {
        assert_eq!(
            clamped_axes(&clamp(Some(1.0), Some(2.0), None, None)),
            Some("ratio")
        );
        assert_eq!(
            clamped_axes(&clamp(None, None, Some(60), Some(4_320))),
            Some("seed_time")
        );
    }

    #[test]
    fn an_axis_the_profile_leaves_unset_is_reported_when_the_tracker_fills_it() {
        // The tracker minimum *becomes* the goal on that axis, which is a
        // policy change the operator has to be able to see.
        assert_eq!(
            clamped_axes(&clamp(None, Some(1.5), None, None)),
            Some("ratio")
        );
        assert_eq!(
            clamped_axes(&clamp(None, None, None, Some(4_320))),
            Some("seed_time")
        );
    }

    #[test]
    fn both_axes_combine_into_one_event() {
        assert_eq!(
            clamped_axes(&clamp(Some(1.0), Some(2.0), Some(60), Some(4_320))),
            Some("ratio,seed_time")
        );
    }

    #[test]
    fn nothing_is_reported_when_the_profile_already_covers_the_minimum() {
        // Equal is not raised, and a profile above the minimum is not raised.
        assert_eq!(clamped_axes(&clamp(Some(2.0), Some(2.0), None, None)), None);
        assert_eq!(clamped_axes(&clamp(Some(3.0), Some(2.0), None, None)), None);
        assert_eq!(
            clamped_axes(&clamp(None, None, Some(4_320), Some(4_320))),
            None
        );
        // No release minimums at all: the common case, and silent.
        assert_eq!(clamped_axes(&clamp(Some(1.0), None, Some(60), None)), None);
    }

    #[test]
    fn non_positive_minimums_are_never_reported_as_a_clamp() {
        assert_eq!(clamped_axes(&clamp(Some(1.0), Some(0.0), None, None)), None);
        assert_eq!(
            clamped_axes(&clamp(Some(1.0), Some(f64::NAN), None, None)),
            None
        );
        assert_eq!(clamped_axes(&clamp(None, None, Some(60), Some(0))), None);
    }

    #[test]
    fn resolution_sources_round_trip_through_their_persisted_labels() {
        for source in [
            SeedGoalResolutionSource::None,
            SeedGoalResolutionSource::Indexer,
            SeedGoalResolutionSource::RoutingEntry,
            SeedGoalResolutionSource::GlobalDefault,
        ] {
            assert_eq!(
                SeedGoalResolutionSource::parse(source.as_str()),
                Some(source)
            );
        }
        assert_eq!(SeedGoalResolutionSource::parse("nope"), None);
    }
}
