//! Location operations: one subsystem behind every workflow that changes where a
//! title's catalog record or files live.
//!
//! The six operation types (folder reassignment, title-scoped root move, root
//! change, root consolidation, cross-library transfer, external adoption) share
//! one persisted, checkpointed, resumable operation model so Activity, resume,
//! cancellation, and the concurrency guard are written once.
//!
//! Module map (see `specs/0001-library-location-and-movement/`):
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`model`] | Operation, checkpoint, and verification-record types shared by every workflow. |
//! | [`preview`] | Plan building, plan fingerprinting, complete counts with sampled items, free-space estimation, typed confirmation (FR-080–082). |
//! | [`classify`] | Per-title classification of a requested destination (FR-015, FR-076). |
//! | [`folder_match`] | Folder-match correction: preview, assign, swap, take over — catalog only, never a file operation (FR-001–008, FR-014). |
//! | [`root_move`] | Title-scoped root-move planner: calculated destination folders, per-title plan items, execution instructions (FR-012–013, FR-076). |
//! | [`root_change`] | Root-scoped planner for replacing a root's path: every-title accounting, identity/default retention, unmanaged-content buckets, retirement ordering (FR-020–029, FR-087). |
//! | [`execution`] | The root-move runner seams: mover, reconciler, admission check (FR-031–032, FR-044, FR-089). |
//! | [`operations`] | The use-case API GraphQL calls: preview, confirm-and-start, cancel, restart resume (FR-030, FR-033, FR-083). |
//! | [`executor`] | The operation runner: state machine, per-title checkpoints, safe-cancel points, restart resume (FR-030–033, FR-089, FR-092). |
//! | [`verify`] | Verified streaming copy: CRC + full BLAKE3 in one pass, depth-governed read-back (FR-040–044). |
//! | [`collisions`] | Destination-wins naming, disambiguation, sidecar grouping, BLAKE3 dedup (FR-072–075). |
//! | [`hardlinks`] | Link-count detection and the seeding/disk warnings previews surface (FR-085). |
//! | [`identity`] | Destination-title detection by stable metadata identity and redirects (FR-055). |
//! | [`transfer_effects`] | Series↔anime facet conversion and the link/kind/collection dispositions a transfer states (FR-057–058, FR-060–062). |
//! | [`merge`] | Identity mapping and per-table dispositions when a destination title already exists (FR-063–067). |
//! | [`adoption`] | "Files are already there": destination accounting against stored catalog proof (FR-050–053). |
//! | [`asset_listing`] | Which files a finished operation renamed and deduplicated, read back off its confirmed plan (FR-091). |
//! | [`ownership_guard`] | Persisted + in-process (title, root) ownership for the duration of an operation (FR-084). |
//! | [`backfill`] | The throttled, resumable full-hash convergence job (FR-047). |
//! | [`media_server_refresh`] | Targeted media-server refresh for the folders a finished operation changed (FR-088). |

pub mod adoption;
pub mod asset_listing;
pub mod backfill;
pub mod classify;
pub mod collisions;
pub mod execution;
pub mod executor;
pub mod folder_match;
pub mod hardlinks;
pub mod identity;
pub mod media_server_refresh;
pub mod merge;
pub mod model;
pub mod operations;
pub mod ownership_guard;
pub mod preview;
pub mod root_change;
pub mod root_move;
#[cfg(test)]
pub(crate) mod test_support;
pub mod transfer_effects;
pub mod verify;
