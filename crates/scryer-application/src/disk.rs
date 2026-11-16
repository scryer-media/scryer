//! The disk service, as the application reaches it.
//!
//! The owner of every filesystem operation is the `scryer-disk` crate; this
//! module re-exports it under the paths the application and the crates above
//! it use. What stays here is what needs application state: installing the
//! configured roots on the owner, and the free-space owner in [`capacity`],
//! which samples the library roots.

pub(crate) mod capacity;

pub use scryer_disk::{
    Access, DISK_STAT_REUSE, Disk, DiskError, DiskPass, DiskVerb, EntryKind, FileIdentity,
    IfPresent, KeptPermissions, Link, LinkOutcome, Listed, MoveOutcome, Open, OwnerOnly, Parents,
    PathReading, Removal, Remove, Replace, ScratchDir, Staged, Staging, SyncOutcome, Visited,
    VolumeClass, is_cross_device_error, reason,
};

#[cfg(test)]
pub(crate) use scryer_disk::test_support;
pub(crate) use scryer_disk::{
    DiskClock, Offloaded, SystemClock, configured_volumes, offload, read, reusable,
    staging_path_for, write,
};

impl crate::AppUseCase {
    /// Install the configured roots on the process's owner: every library
    /// root, and a custom recycle bin. Run once at startup and after every
    /// write that adds, changes or removes one.
    ///
    /// A root's class is its role, so this reads the library list and one
    /// setting and touches no filesystem. When the roots cannot be read, the
    /// roots already installed stay.
    pub async fn install_disk_volumes(&self) {
        let library_roots = match self.storage_capacity_roots().await {
            Ok(roots) => roots,
            Err(error) => {
                tracing::warn!(error = %error, "could not read the library roots; the disk owner keeps the roots it has");
                return;
            }
        };
        let (_, recycle_bin, _) = self.recycle_bin_config_values().await;
        Disk::process().set_volumes(configured_volumes(library_roots, recycle_bin));
    }
}

/// The free-space owner, reached from the disk owner. Callers still reach it
/// through `StorageCapacity::process` until they move onto this owner.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) trait DiskCapacity {
    fn capacity(&self) -> capacity::StorageCapacity;
}

impl DiskCapacity for Disk {
    fn capacity(&self) -> capacity::StorageCapacity {
        capacity::StorageCapacity::process()
    }
}
