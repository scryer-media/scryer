//! Process-local copy admission, as the application installs it. The disk
//! service owns the turns; this reads the operator's worker limit once and
//! hands it over.

use std::sync::Once;

pub use crate::disk::CopyAdmission as CopyCoordinator;

/// The process's copy admission: one budget shared by import and location
/// work. The worker limit is read from the environment once, before the
/// first copy asks for a turn.
pub fn shared() -> CopyCoordinator {
    install_copy_workers();
    crate::disk::Disk::process().copy_admission()
}

/// Hands the operator's worker limit to the disk service. Run at startup with
/// the roots, and before any policy that copies is built.
pub(crate) fn install_copy_workers() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let value = std::env::var("SCRYER_FILE_TRANSFER_WORKERS_PER_VOLUME")
            .or_else(|_| std::env::var("SCRYER_IMPORT_COPY_WORKERS_PER_VOLUME"))
            .ok();
        crate::disk::Disk::process().install_copy_workers(worker_limit(value.as_deref()));
    });
}

fn worker_limit(value: Option<&str>) -> usize {
    value
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(2)
        .min(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_worker_setting_is_bounded() {
        assert_eq!(worker_limit(None), 2);
        assert_eq!(worker_limit(Some("0")), 2);
        assert_eq!(worker_limit(Some("bad")), 2);
        assert_eq!(worker_limit(Some("1")), 1);
        assert_eq!(worker_limit(Some("100")), 8);
    }
}
