//! Operation-ownership registry (D7, FR-084).
//!
//! While an operation owns a title or a root, every conflicting entry point —
//! library scans, imports, renames, title deletion, media-file mutation, other
//! location operations, root removal/configuration, and policy automation or
//! maintenance jobs — consults one choke-point helper and refuses. Unrelated
//! titles and libraries keep operating normally; there is no global lock.
//!
//! # Registering a new mutating entry point
//!
//! Every guarded call site goes through [`AppUseCase::ensure_location_ownership_allows`]
//! (or one of its entity-shaped wrappers) and names a [`GuardedEntry`] constant
//! declared in this module. Adding a new mutating entry point therefore means:
//!
//! 1. declare a `GuardedEntry` constant here, naming the module path and
//!    function that carries the check;
//! 2. add it to [`GUARDED_ENTRIES`];
//! 3. call the choke point with that constant at the entry point's admission.
//!
//! [`guarded_entries_are_wired`] reads the source of every registered module and
//! fails when a declared entry's function or constant is missing from it, so the
//! declaration and the wiring cannot drift apart silently.
//!
//! # Layering
//!
//! [`LocationOwnershipRegistry`] is the in-process fast path: the operation
//! runner writes claims into it as it takes them and clears them when the
//! operation reaches a terminal state, so a conflict inside this process is
//! answered without a query. Persisted claims (`LocationOperationRepository`)
//! remain the source of truth and are consulted whenever the registry is silent,
//! so a restart — which empties the registry but not the claim rows — still
//! refuses.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::{AppError, AppResult};

/// An entity an active operation holds for its duration. Persisted so ownership
/// survives a restart alongside the operation itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum OwnedEntity {
    /// One catalog title.
    Title(String),
    /// One root, by synthetic root id (FR-078).
    Root(String),
}

impl OwnedEntity {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Title(_) => "title",
            Self::Root(_) => "root",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Title(id) | Self::Root(id) => id,
        }
    }
}

/// The kinds of work the guard refuses while an entity is owned (FR-084). The
/// audit test in T016 enumerates one guarded entry point per variant.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GuardedAction {
    LibraryScan,
    Import,
    Rename,
    TitleDelete,
    MediaFileMutation,
    LocationOperation,
    RootConfiguration,
    MaintenanceJob,
}

impl GuardedAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LibraryScan => "library_scan",
            Self::Import => "import",
            Self::Rename => "rename",
            Self::TitleDelete => "title_delete",
            Self::MediaFileMutation => "media_file_mutation",
            Self::LocationOperation => "location_operation",
            Self::RootConfiguration => "root_configuration",
            Self::MaintenanceJob => "maintenance_job",
        }
    }

    /// How the refusal names the blocked work to an operator.
    pub fn label(&self) -> &'static str {
        match self {
            Self::LibraryScan => "a library scan",
            Self::Import => "an import",
            Self::Rename => "a rename",
            Self::TitleDelete => "deleting this title",
            Self::MediaFileMutation => "changing this title's media files",
            Self::LocationOperation => "another location operation",
            Self::RootConfiguration => "changing this root configuration",
            Self::MaintenanceJob => "this maintenance job",
        }
    }
}

/// A refusal from the choke-point helper, carrying enough context for an
/// actionable error (C6: typed, actionable, never silent reinterpretation).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnershipConflict {
    /// Operation currently holding the entity.
    pub operation_id: String,
    /// The entity that is owned.
    pub entity: OwnedEntity,
    /// What the caller was trying to do.
    pub action: GuardedAction,
}

/// In-process mirror of the persisted claims (D7).
///
/// The operation runner writes into it when a claim succeeds and clears the
/// operation's rows when the operation stops, so a same-process conflict is
/// refused without a query. It is a fast path, never the authority: an empty
/// registry (a fresh process, a claim taken before this table existed) still
/// falls through to the persisted claims.
#[derive(Clone, Default)]
pub struct LocationOwnershipRegistry {
    claims: Arc<RwLock<HashMap<OwnedEntity, String>>>,
}

impl LocationOwnershipRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mirrors a successful claim. Called only after the store accepted it, so
    /// the registry can never invent ownership the datastore does not hold.
    pub fn claim_all(&self, operation_id: &str, entities: &[OwnedEntity]) {
        let Ok(mut claims) = self.claims.write() else {
            return;
        };
        for entity in entities {
            claims.insert(entity.clone(), operation_id.to_string());
        }
    }

    /// Drops every entity this operation holds.
    pub fn release_operation(&self, operation_id: &str) {
        let Ok(mut claims) = self.claims.write() else {
            return;
        };
        claims.retain(|_, holder| holder != operation_id);
    }

    pub fn holder(&self, entity: &OwnedEntity) -> Option<String> {
        self.claims.read().ok()?.get(entity).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.claims
            .read()
            .map(|claims| claims.is_empty())
            .unwrap_or(true)
    }
}

impl std::fmt::Debug for LocationOwnershipRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocationOwnershipRegistry")
            .field(
                "claims",
                &self.claims.read().map(|claims| claims.len()).unwrap_or(0),
            )
            .finish()
    }
}

/// The typed refusal the choke point returns (C6: actionable, never a silent
/// reinterpretation of what the caller asked for).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationOwnershipDenied {
    /// The entry point that was refused.
    pub entry: &'static GuardedEntry,
    /// Every overlapping entity, in the order the caller listed them.
    pub conflicts: Vec<OwnershipConflict>,
}

impl LocationOwnershipDenied {
    pub fn action(&self) -> GuardedAction {
        self.entry.action
    }

    /// The operation ids holding the overlapping entities, deduplicated.
    pub fn holding_operation_ids(&self) -> Vec<String> {
        let mut ids = self
            .conflicts
            .iter()
            .map(|conflict| conflict.operation_id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn message(&self) -> String {
        let held = self
            .conflicts
            .iter()
            .map(|conflict| {
                format!(
                    "{} {} (operation {})",
                    conflict.entity.kind_str(),
                    conflict.entity.id(),
                    conflict.operation_id
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} is blocked while a location operation owns {}; it can run again once that operation finishes or is canceled",
            self.entry.action.label(),
            held
        )
    }
}

impl std::fmt::Display for LocationOwnershipDenied {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl LocationOwnershipDenied {
    /// Surfaces through each entry point's existing error convention: a refusal
    /// the operator can act on, not a repository failure.
    ///
    /// Deliberately an inherent method rather than a `From` impl — a blanket
    /// conversion into [`AppError`] makes `Ok(())` ambiguous in the crate's many
    /// inferred-error closures.
    pub fn into_app_error(self) -> AppError {
        AppError::Validation(self.message())
    }
}

/// One registered mutating entry point (FR-084).
///
/// `module` and `function` are what [`guarded_entries_are_wired`] checks against
/// the real source, and what an operator-facing diagnostic can print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardedEntry {
    pub action: GuardedAction,
    /// Path of the module carrying the call, relative to
    /// `crates/scryer-application/src/`.
    pub module: &'static str,
    /// The function whose admission performs the check.
    pub function: &'static str,
    /// This constant's own name, so the audit can find the call site.
    pub constant: &'static str,
}

/// Library scans: every scan session, however it was triggered, funnels here.
pub const LIBRARY_SCAN_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::LibraryScan,
    module: "library/library.rs",
    function: "run_started_library_scan_session",
    constant: "LIBRARY_SCAN_ENTRY",
};

/// Automatic and manual-review imports of a completed download, checked once the
/// target title is resolved and before any file is touched.
pub const COMPLETED_IMPORT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::Import,
    module: "import/workflow/series_movie.rs",
    function: "dispatch_completed_import_target",
    constant: "COMPLETED_IMPORT_ENTRY",
};

/// Operator-driven manual imports, which do not pass through the completed
/// download dispatcher.
pub const MANUAL_IMPORT_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::Import,
    module: "import/workflow/manual.rs",
    function: "execute_manual_import_with_release_evidence",
    constant: "MANUAL_IMPORT_ENTRY",
};

/// The rename apply path, shared by the per-title and per-facet entry points.
pub const RENAME_APPLY_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::Rename,
    module: "library/rename.rs",
    function: "apply_rename_plan",
    constant: "RENAME_APPLY_ENTRY",
};

/// Single-title deletion.
pub const TITLE_DELETE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleDelete,
    module: "catalog/workflow/delete.rs",
    function: "delete_title",
    constant: "TITLE_DELETE_ENTRY",
};

/// The bulk deletion job's per-title work, which does not route through
/// [`TITLE_DELETE_ENTRY`].
pub const TITLE_DELETE_JOB_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::TitleDelete,
    module: "catalog/workflow/delete.rs",
    function: "delete_title_job_item",
    constant: "TITLE_DELETE_JOB_ENTRY",
};

/// Media-file deletion, including the per-item work of the bulk deletion job.
pub const MEDIA_FILE_DELETE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::MediaFileMutation,
    module: "catalog/workflow/delete.rs",
    function: "delete_media_file",
    constant: "MEDIA_FILE_DELETE_ENTRY",
};

/// Primary media-file changes, which rewrite which file a title serves.
pub const MEDIA_FILE_PRIMARY_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::MediaFileMutation,
    module: "catalog/workflow/metadata.rs",
    function: "set_primary_movie_file",
    constant: "MEDIA_FILE_PRIMARY_ENTRY",
};

/// Root configuration changes on an existing library.
pub const LIBRARY_ROOTS_UPDATE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::RootConfiguration,
    module: "library/library.rs",
    function: "update_library",
    constant: "LIBRARY_ROOTS_UPDATE_ENTRY",
};

/// Library removal, which retires every root the library configures.
pub const LIBRARY_DELETE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::RootConfiguration,
    module: "library/library.rs",
    function: "delete_library",
    constant: "LIBRARY_DELETE_ENTRY",
};

/// Recycle-bin restore, the maintenance job that writes files back into a
/// title's folder.
pub const RECYCLE_RESTORE_ENTRY: GuardedEntry = GuardedEntry {
    action: GuardedAction::MaintenanceJob,
    module: "jobs/housekeeping.rs",
    function: "restore_recycled_item_from_context",
    constant: "RECYCLE_RESTORE_ENTRY",
};

/// Every registered mutating entry point. A new one is added here and nowhere
/// else; [`guarded_entries_are_wired`] fails when this list and the code drift.
pub const GUARDED_ENTRIES: &[&GuardedEntry] = &[
    &LIBRARY_SCAN_ENTRY,
    &COMPLETED_IMPORT_ENTRY,
    &MANUAL_IMPORT_ENTRY,
    &RENAME_APPLY_ENTRY,
    &TITLE_DELETE_ENTRY,
    &TITLE_DELETE_JOB_ENTRY,
    &MEDIA_FILE_DELETE_ENTRY,
    &MEDIA_FILE_PRIMARY_ENTRY,
    &LIBRARY_ROOTS_UPDATE_ENTRY,
    &LIBRARY_DELETE_ENTRY,
    &RECYCLE_RESTORE_ENTRY,
];

/// Actions that need no registered entry point because exclusivity is enforced
/// elsewhere.
///
/// [`GuardedAction::LocationOperation`] is the only one: two operations cannot
/// hold the same entity because
/// [`crate::ports::LocationOperationRepository::claim_location_operation_ownership`]
/// claims all-or-nothing behind the store's partial unique index on unreleased
/// claims, and reports the loser's conflicts directly.
pub const ACTIONS_GUARDED_BY_CLAIM: &[GuardedAction] = &[GuardedAction::LocationOperation];

/// Every action the guard knows about; kept beside the enum so the audit test
/// notices a variant nobody registered an entry point for.
pub const ALL_GUARDED_ACTIONS: &[GuardedAction] = &[
    GuardedAction::LibraryScan,
    GuardedAction::Import,
    GuardedAction::Rename,
    GuardedAction::TitleDelete,
    GuardedAction::MediaFileMutation,
    GuardedAction::LocationOperation,
    GuardedAction::RootConfiguration,
    GuardedAction::MaintenanceJob,
];

/// The choke point (D7): the one place that answers "may this action proceed
/// against these entities?".
pub struct LocationOwnershipGuard<'a> {
    store: &'a dyn crate::ports::LocationOperationRepository,
    registry: Option<&'a LocationOwnershipRegistry>,
}

impl<'a> LocationOwnershipGuard<'a> {
    pub fn new(
        store: &'a dyn crate::ports::LocationOperationRepository,
        registry: &'a LocationOwnershipRegistry,
    ) -> Self {
        Self {
            store,
            registry: Some(registry),
        }
    }

    /// Persisted claims only — for callers with no runtime state at hand.
    pub fn persisted_only(store: &'a dyn crate::ports::LocationOperationRepository) -> Self {
        Self {
            store,
            registry: None,
        }
    }

    /// The typed answer. `Ok(None)` means nothing owns any of `entities`.
    ///
    /// Entities are checked individually, so an operation on an unrelated title
    /// or root never denies: only an actual overlap does.
    pub async fn check(
        &self,
        entry: &'static GuardedEntry,
        entities: &[OwnedEntity],
    ) -> AppResult<Option<LocationOwnershipDenied>> {
        if entities.is_empty() {
            return Ok(None);
        }

        let mut conflicts = Vec::new();
        for entity in entities {
            let holder = match self.registry.and_then(|registry| registry.holder(entity)) {
                Some(holder) => Some(holder),
                None => self.store.location_ownership_holder(entity).await?,
            };
            if let Some(operation_id) = holder {
                conflicts.push(OwnershipConflict {
                    operation_id,
                    entity: entity.clone(),
                    action: entry.action,
                });
            }
        }

        if conflicts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(LocationOwnershipDenied { entry, conflicts }))
        }
    }

    /// [`Self::check`] mapped onto the caller's error convention.
    pub async fn ensure_not_owned(
        &self,
        entry: &'static GuardedEntry,
        entities: &[OwnedEntity],
    ) -> AppResult<()> {
        match self.check(entry, entities).await? {
            None => Ok(()),
            Some(denied) => Err(denied.into_app_error()),
        }
    }

    /// Every open claim, for facet-wide callers that cannot enumerate their own
    /// entities up front.
    pub async fn open_claims(&self) -> AppResult<Vec<crate::ports::LocationOwnershipClaim>> {
        self.store.list_location_ownership_claims().await
    }
}

impl crate::AppUseCase {
    fn location_ownership_guard(&self) -> LocationOwnershipGuard<'_> {
        LocationOwnershipGuard::new(
            self.services.library.location_operations.as_ref(),
            &self.runtime.library.location_ownership,
        )
    }

    /// Every open claim, for background work that cannot enumerate its own
    /// entities up front and only needs to *skip* what is owned rather than
    /// refuse (the full-hash backfill job, FR-047/SC-007).
    pub(crate) async fn location_ownership_open_claims(
        &self,
    ) -> AppResult<Vec<crate::ports::LocationOwnershipClaim>> {
        self.location_ownership_guard().open_claims().await
    }

    /// The choke point every mutating entry point calls (FR-084). `entry` names
    /// the [`GuardedEntry`] constant registered for this call site.
    pub(crate) async fn ensure_location_ownership_allows(
        &self,
        entry: &'static GuardedEntry,
        entities: &[OwnedEntity],
    ) -> AppResult<()> {
        self.location_ownership_guard()
            .ensure_not_owned(entry, entities)
            .await
    }

    /// Title-scoped shorthand.
    pub(crate) async fn ensure_location_ownership_allows_title(
        &self,
        entry: &'static GuardedEntry,
        title_id: &str,
    ) -> AppResult<()> {
        self.ensure_location_ownership_allows(entry, &[OwnedEntity::Title(title_id.to_string())])
            .await
    }

    /// Root-scoped shorthand covering every root a library configures.
    pub(crate) async fn ensure_location_ownership_allows_library_roots(
        &self,
        entry: &'static GuardedEntry,
        library_id: &str,
    ) -> AppResult<()> {
        let Some(library) = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
        else {
            return Ok(());
        };
        let entities = library
            .roots
            .iter()
            .map(|root| OwnedEntity::Root(root.id.clone()))
            .collect::<Vec<_>>();
        self.ensure_location_ownership_allows(entry, &entities)
            .await
    }

    /// Facet-wide shorthand for callers whose plan names no ids (bulk rename).
    ///
    /// Resolves the open claims instead of the caller's entities, then keeps
    /// only the ones inside this facet, so an operation on another facet's
    /// library never blocks the work.
    pub(crate) async fn ensure_location_ownership_allows_facet(
        &self,
        entry: &'static GuardedEntry,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<()> {
        let claims = self.location_ownership_guard().open_claims().await?;
        if claims.is_empty() {
            return Ok(());
        }

        let mut facet_root_ids = std::collections::HashSet::new();
        for library in self
            .services
            .catalog
            .libraries
            .list(Some(facet.clone()))
            .await?
        {
            for root in &library.roots {
                facet_root_ids.insert(root.id.clone());
            }
        }

        let mut overlapping = Vec::new();
        for claim in claims {
            let in_facet = match &claim.entity {
                OwnedEntity::Root(root_id) => facet_root_ids.contains(root_id),
                OwnedEntity::Title(title_id) => self
                    .services
                    .catalog
                    .titles
                    .get_by_id(title_id)
                    .await?
                    .is_some_and(|title| title.facet == *facet),
            };
            if in_facet {
                overlapping.push(claim.entity);
            }
        }

        self.ensure_location_ownership_allows(entry, &overlapping)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{
        LocationOperationRepository, LocationOwnershipClaim, LocationOwnershipOutcome,
    };
    use async_trait::async_trait;
    use chrono::Utc;

    /// Source of every module that registers a guarded entry point. The audit
    /// test reads these to prove a declared entry is actually wired.
    const GUARDED_ENTRY_SOURCES: &[(&str, &str)] = &[
        ("library/library.rs", include_str!("../library/library.rs")),
        ("library/rename.rs", include_str!("../library/rename.rs")),
        (
            "import/workflow/series_movie.rs",
            include_str!("../import/workflow/series_movie.rs"),
        ),
        (
            "import/workflow/manual.rs",
            include_str!("../import/workflow/manual.rs"),
        ),
        (
            "catalog/workflow/delete.rs",
            include_str!("../catalog/workflow/delete.rs"),
        ),
        (
            "catalog/workflow/metadata.rs",
            include_str!("../catalog/workflow/metadata.rs"),
        ),
        (
            "jobs/housekeeping.rs",
            include_str!("../jobs/housekeeping.rs"),
        ),
    ];

    #[derive(Default)]
    struct FakeOwnershipStore {
        claims: HashMap<OwnedEntity, String>,
    }

    impl FakeOwnershipStore {
        fn owning(entries: &[(OwnedEntity, &str)]) -> Self {
            Self {
                claims: entries
                    .iter()
                    .map(|(entity, operation_id)| (entity.clone(), (*operation_id).to_string()))
                    .collect(),
            }
        }
    }

    #[async_trait]
    impl LocationOperationRepository for FakeOwnershipStore {
        async fn create_location_operation(
            &self,
            _operation: &crate::location::model::LocationOperation,
            _plan_json: Option<&str>,
        ) -> AppResult<()> {
            unimplemented!("guard tests only read ownership")
        }

        async fn get_location_operation(
            &self,
            _operation_id: &str,
        ) -> AppResult<Option<crate::location::model::LocationOperation>> {
            unimplemented!("guard tests only read ownership")
        }

        async fn get_location_operation_plan_json(
            &self,
            _operation_id: &str,
        ) -> AppResult<Option<String>> {
            unimplemented!("guard tests only read ownership")
        }

        async fn list_active_location_operations(
            &self,
        ) -> AppResult<Vec<crate::location::model::LocationOperation>> {
            Ok(Vec::new())
        }

        async fn update_location_operation_progress(
            &self,
            _progress: &crate::ports::LocationOperationProgress,
        ) -> AppResult<()> {
            unimplemented!("guard tests only read ownership")
        }

        async fn request_location_operation_cancel(&self, _operation_id: &str) -> AppResult<bool> {
            unimplemented!("guard tests only read ownership")
        }

        async fn location_operation_cancel_requested(
            &self,
            _operation_id: &str,
        ) -> AppResult<bool> {
            Ok(false)
        }

        async fn upsert_location_title_checkpoint(
            &self,
            _checkpoint: &crate::location::model::TitleCheckpoint,
        ) -> AppResult<()> {
            unimplemented!("guard tests only read ownership")
        }

        async fn list_location_title_checkpoints(
            &self,
            _operation_id: &str,
        ) -> AppResult<Vec<crate::location::model::TitleCheckpoint>> {
            Ok(Vec::new())
        }

        async fn record_location_file_verification(
            &self,
            _record: &crate::location::model::FileVerificationRecord,
        ) -> AppResult<()> {
            unimplemented!("guard tests only read ownership")
        }

        async fn list_location_file_verifications(
            &self,
            _operation_id: &str,
            _title_id: Option<&str>,
        ) -> AppResult<Vec<crate::location::model::FileVerificationRecord>> {
            Ok(Vec::new())
        }

        async fn verified_destination_paths(
            &self,
            _operation_id: &str,
            _title_id: &str,
        ) -> AppResult<std::collections::BTreeSet<String>> {
            Ok(std::collections::BTreeSet::new())
        }

        async fn claim_location_operation_ownership(
            &self,
            _operation_id: &str,
            _entities: &[OwnedEntity],
        ) -> AppResult<LocationOwnershipOutcome> {
            unimplemented!("guard tests only read ownership")
        }

        async fn release_location_operation_ownership(
            &self,
            _operation_id: &str,
        ) -> AppResult<u64> {
            Ok(0)
        }

        async fn location_ownership_holder(
            &self,
            entity: &OwnedEntity,
        ) -> AppResult<Option<String>> {
            Ok(self.claims.get(entity).cloned())
        }

        async fn list_location_ownership_claims(&self) -> AppResult<Vec<LocationOwnershipClaim>> {
            Ok(self
                .claims
                .iter()
                .map(|(entity, operation_id)| LocationOwnershipClaim {
                    operation_id: operation_id.clone(),
                    entity: entity.clone(),
                    acquired_at: Utc::now(),
                })
                .collect())
        }
    }

    fn title(id: &str) -> OwnedEntity {
        OwnedEntity::Title(id.to_string())
    }

    fn root(id: &str) -> OwnedEntity {
        OwnedEntity::Root(id.to_string())
    }

    #[tokio::test]
    async fn persisted_claim_denies_an_overlapping_entity() {
        let store = FakeOwnershipStore::owning(&[(title("title-1"), "op-1")]);
        let guard = LocationOwnershipGuard::persisted_only(&store);

        let denied = guard
            .check(&TITLE_DELETE_ENTRY, &[title("title-1")])
            .await
            .expect("guard query")
            .expect("overlapping entity must be denied");

        assert_eq!(denied.action(), GuardedAction::TitleDelete);
        assert_eq!(denied.holding_operation_ids(), vec!["op-1".to_string()]);
        assert!(denied.message().contains("op-1"));
        assert!(denied.message().contains("title-1"));
    }

    #[tokio::test]
    async fn disjoint_entities_pass_through() {
        let store =
            FakeOwnershipStore::owning(&[(title("title-1"), "op-1"), (root("root-1"), "op-1")]);
        let guard = LocationOwnershipGuard::persisted_only(&store);

        guard
            .ensure_not_owned(&TITLE_DELETE_ENTRY, &[title("title-2")])
            .await
            .expect("an unrelated title must not be blocked");
        guard
            .ensure_not_owned(&LIBRARY_SCAN_ENTRY, &[root("root-2")])
            .await
            .expect("an unrelated root must not be blocked");
    }

    #[tokio::test]
    async fn released_claims_stop_denying() {
        let owned = FakeOwnershipStore::owning(&[(title("title-1"), "op-1")]);
        LocationOwnershipGuard::persisted_only(&owned)
            .ensure_not_owned(&RENAME_APPLY_ENTRY, &[title("title-1")])
            .await
            .expect_err("an owned title must be refused");

        // Release drops the row; the same query now answers "nothing owns it".
        let released = FakeOwnershipStore::default();
        LocationOwnershipGuard::persisted_only(&released)
            .ensure_not_owned(&RENAME_APPLY_ENTRY, &[title("title-1")])
            .await
            .expect("a released claim must stop denying");
    }

    #[tokio::test]
    async fn in_process_registry_denies_before_the_store_is_queried() {
        // The store knows nothing; only the in-process registry does.
        let store = FakeOwnershipStore::default();
        let registry = LocationOwnershipRegistry::new();
        registry.claim_all("op-7", &[title("title-9")]);
        let guard = LocationOwnershipGuard::new(&store, &registry);

        let denied = guard
            .check(&LIBRARY_SCAN_ENTRY, &[title("title-9")])
            .await
            .expect("guard query")
            .expect("the in-process fast path must deny");
        assert_eq!(denied.holding_operation_ids(), vec!["op-7".to_string()]);

        registry.release_operation("op-7");
        assert!(registry.is_empty());
        guard
            .ensure_not_owned(&LIBRARY_SCAN_ENTRY, &[title("title-9")])
            .await
            .expect("a released registry claim must stop denying");
    }

    #[tokio::test]
    async fn an_empty_entity_list_never_denies() {
        let store = FakeOwnershipStore::owning(&[(title("title-1"), "op-1")]);
        LocationOwnershipGuard::persisted_only(&store)
            .ensure_not_owned(&LIBRARY_ROOTS_UPDATE_ENTRY, &[])
            .await
            .expect("an entry point with no entities has nothing to overlap");
    }

    #[tokio::test]
    async fn every_conflicting_entity_is_reported() {
        let store =
            FakeOwnershipStore::owning(&[(title("title-1"), "op-1"), (root("root-1"), "op-2")]);
        let denied = LocationOwnershipGuard::persisted_only(&store)
            .check(
                &LIBRARY_DELETE_ENTRY,
                &[title("title-1"), title("title-2"), root("root-1")],
            )
            .await
            .expect("guard query")
            .expect("two owned entities must be denied");

        assert_eq!(denied.conflicts.len(), 2);
        assert_eq!(
            denied.holding_operation_ids(),
            vec!["op-1".to_string(), "op-2".to_string()]
        );
    }

    /// The plan's risk mitigation: the declared entry list and the wiring must
    /// not drift. A new mutating entry point registers a [`GuardedEntry`] here
    /// and calls the choke point with it; this test fails loudly otherwise.
    #[test]
    fn guarded_entries_are_wired() {
        for entry in GUARDED_ENTRIES {
            let source = GUARDED_ENTRY_SOURCES
                .iter()
                .find(|(module, _)| *module == entry.module)
                .map(|(_, source)| *source)
                .unwrap_or_else(|| {
                    panic!(
                        "guarded entry {} names module {} which GUARDED_ENTRY_SOURCES does not include; add an include_str! for it",
                        entry.constant, entry.module
                    )
                });

            assert!(
                source.contains(&format!("fn {}(", entry.function)),
                "guarded entry {} names {}::{}, which no longer exists",
                entry.constant,
                entry.module,
                entry.function
            );
            assert!(
                source.contains(entry.constant),
                "guarded entry {} is declared but {} never references it: the entry point is unguarded",
                entry.constant,
                entry.module
            );
        }
    }

    #[test]
    fn every_guarded_action_has_an_entry_point() {
        for action in ALL_GUARDED_ACTIONS {
            if ACTIONS_GUARDED_BY_CLAIM.contains(action) {
                assert!(
                    !GUARDED_ENTRIES.iter().any(|entry| entry.action == *action),
                    "{} is documented as guarded by the claim itself but also registers an entry point",
                    action.as_str()
                );
                continue;
            }
            assert!(
                GUARDED_ENTRIES.iter().any(|entry| entry.action == *action),
                "{} has no registered entry point; register one in GUARDED_ENTRIES or document it in ACTIONS_GUARDED_BY_CLAIM",
                action.as_str()
            );
        }
    }

    #[test]
    fn guarded_entry_constants_are_named_after_themselves() {
        for entry in GUARDED_ENTRIES {
            assert!(
                include_str!("ownership_guard.rs")
                    .contains(&format!("pub const {}: GuardedEntry", entry.constant)),
                "{} does not name its own constant",
                entry.constant
            );
        }
    }
}
