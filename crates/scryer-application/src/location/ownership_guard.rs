//! Operation-ownership registry (D7, FR-084). Guard and choke-point helper land
//! in T016.
//!
//! While an operation owns a title or a root, every conflicting entry point —
//! library scans, imports, renames, title deletion, media-file mutation, other
//! location operations, root removal/configuration, and policy automation or
//! maintenance jobs — consults one choke-point helper and refuses. Unrelated
//! titles and libraries keep operating normally; there is no global lock.

use serde::{Deserialize, Serialize};

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
