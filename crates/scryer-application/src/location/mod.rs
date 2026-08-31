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
//! | [`preview`] | Plan building, plan fingerprinting, complete counts with sampled items (FR-080–082). |
//! | [`classify`] | Per-title classification of a requested destination (FR-015, FR-076). |
//! | [`executor`] | The operation runner: state machine, per-title checkpoints, safe-cancel points, restart resume (FR-030–033, FR-089, FR-092). |
//! | [`verify`] | Verified streaming copy: CRC + full BLAKE3 in one pass, depth-governed read-back (FR-040–044). |
//! | [`collisions`] | Destination-wins naming, disambiguation, sidecar grouping, BLAKE3 dedup (FR-072–075). |
//! | [`merge`] | Identity mapping and per-table dispositions when a destination title already exists (FR-063–067). |
//! | [`adoption`] | "Files are already there": destination accounting against stored catalog proof (FR-050–053). |
//! | [`ownership_guard`] | Persisted + in-process (title, root) ownership for the duration of an operation (FR-084). |

pub mod adoption;
pub mod classify;
pub mod collisions;
pub mod executor;
pub mod merge;
pub mod model;
pub mod ownership_guard;
pub mod preview;
pub mod verify;
