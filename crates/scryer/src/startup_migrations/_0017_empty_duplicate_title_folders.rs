use scryer_application::AppUseCase;

use super::versioning::is_release_before;

pub(crate) const ID: &str = "0017_empty_duplicate_title_folders";

/// Whether this start is an upgrade from a release that could have left the
/// folders behind. A first run, a restart, and an unreadable version are not.
pub(crate) fn applies_to_upgrade_from(previous_version: Option<&str>) -> bool {
    previous_version.is_some_and(|previous| is_release_before(previous, 0, 21, 14))
}

/// Remove the empty title folders an old rename left beside the folder it
/// moved the title into.
///
/// One attempt only. The runner records this migration before it starts, so a
/// failure or an interrupted run is never retried and nothing is resumed.
pub(crate) async fn run(app: &AppUseCase) {
    match app.remove_empty_duplicate_title_folders().await {
        Ok(report) => tracing::info!(
            removed = report.removed.len(),
            not_removed = report.kept.len(),
            failed = report.failed.len(),
            "one-time cleanup of empty title folders left by an old rename finished"
        ),
        Err(error) => tracing::warn!(
            error = %error,
            "one-time cleanup of empty title folders left by an old rename failed and will not be retried"
        ),
    }
}
