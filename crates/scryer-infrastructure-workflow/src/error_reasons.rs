//! Stable reasons for conditions raised at this infrastructure boundary.

/// another import of this download holds its sources; refresh the queue
pub const WORKFLOW_IMPORT_SOURCES_HELD: &str = "WORKFLOW_IMPORT_SOURCES_HELD";

/// canonical download for import retry
pub const WORKFLOW_IMPORT_RETRY_DOWNLOAD_NOT_FOUND: &str =
    "WORKFLOW_IMPORT_RETRY_DOWNLOAD_NOT_FOUND";

/// download is awaiting retry reconciliation
pub const WORKFLOW_DOWNLOAD_AWAITING_RETRY_RECONCILIATION: &str =
    "WORKFLOW_DOWNLOAD_AWAITING_RETRY_RECONCILIATION";

/// download binding changed during retry; reconciliation required
pub const WORKFLOW_DOWNLOAD_BINDING_CHANGED_DURING_RETRY: &str =
    "WORKFLOW_DOWNLOAD_BINDING_CHANGED_DURING_RETRY";

/// this download has no stable identity yet; its sources stay held
pub const WORKFLOW_DOWNLOAD_IDENTITY_UNRESOLVED: &str = "WORKFLOW_DOWNLOAD_IDENTITY_UNRESOLVED";

/// download retry no longer owns this job
pub const WORKFLOW_DOWNLOAD_RETRY_OWNERSHIP_LOST: &str = "WORKFLOW_DOWNLOAD_RETRY_OWNERSHIP_LOST";

/// this download is still being imported; release its sources once that finishes
pub const WORKFLOW_IMPORT_RUNNING: &str = "WORKFLOW_IMPORT_RUNNING";

/// download submission not found
pub const WORKFLOW_DOWNLOAD_SUBMISSION_NOT_FOUND: &str = "WORKFLOW_DOWNLOAD_SUBMISSION_NOT_FOUND";

/// the import changed while its release was requested; refresh the queue
pub const WORKFLOW_IMPORT_RELEASE_STALE: &str = "WORKFLOW_IMPORT_RELEASE_STALE";

/// import not found
pub const WORKFLOW_IMPORT_NOT_FOUND: &str = "WORKFLOW_IMPORT_NOT_FOUND";

/// maintenance Search requires an accepted receipt for its acquisition job run
pub const WORKFLOW_ACQUISITION_RECEIPT_REQUIRED: &str = "WORKFLOW_ACQUISITION_RECEIPT_REQUIRED";

/// manual cleanup requires title and facet
pub const WORKFLOW_CLEANUP_IDENTITY_REQUIRED: &str = "WORKFLOW_CLEANUP_IDENTITY_REQUIRED";

/// password retry is awaiting reconciliation; binding preserved
pub const WORKFLOW_PASSWORD_RETRY_AWAITING_RECONCILIATION: &str =
    "WORKFLOW_PASSWORD_RETRY_AWAITING_RECONCILIATION";

/// retry response belongs to another download; reconciliation required
pub const WORKFLOW_RETRY_DOWNLOAD_MISMATCH: &str = "WORKFLOW_RETRY_DOWNLOAD_MISMATCH";

/// retry response conflicts with observed progress; reconciliation required
pub const WORKFLOW_RETRY_PROGRESS_CONFLICT: &str = "WORKFLOW_RETRY_PROGRESS_CONFLICT";

/// source cleanup already owns this download
pub const WORKFLOW_SOURCE_CLEANUP_ALREADY_CLAIMED: &str = "WORKFLOW_SOURCE_CLEANUP_ALREADY_CLAIMED";

/// workflow operation {id}
pub const WORKFLOW_OPERATION_NOT_FOUND: &str = "WORKFLOW_OPERATION_NOT_FOUND";

#[cfg(test)]
const ALL: &[&str] = &[
    WORKFLOW_IMPORT_SOURCES_HELD,
    WORKFLOW_IMPORT_RETRY_DOWNLOAD_NOT_FOUND,
    WORKFLOW_DOWNLOAD_AWAITING_RETRY_RECONCILIATION,
    WORKFLOW_DOWNLOAD_BINDING_CHANGED_DURING_RETRY,
    WORKFLOW_DOWNLOAD_IDENTITY_UNRESOLVED,
    WORKFLOW_DOWNLOAD_RETRY_OWNERSHIP_LOST,
    WORKFLOW_IMPORT_RUNNING,
    WORKFLOW_DOWNLOAD_SUBMISSION_NOT_FOUND,
    WORKFLOW_IMPORT_RELEASE_STALE,
    WORKFLOW_IMPORT_NOT_FOUND,
    WORKFLOW_ACQUISITION_RECEIPT_REQUIRED,
    WORKFLOW_CLEANUP_IDENTITY_REQUIRED,
    WORKFLOW_PASSWORD_RETRY_AWAITING_RECONCILIATION,
    WORKFLOW_RETRY_DOWNLOAD_MISMATCH,
    WORKFLOW_RETRY_PROGRESS_CONFLICT,
    WORKFLOW_SOURCE_CLEANUP_ALREADY_CLAIMED,
    WORKFLOW_OPERATION_NOT_FOUND,
];

#[cfg(test)]
mod tests {
    #[test]
    fn reason_codes_are_distinct_and_stable_format() {
        let mut seen = std::collections::HashSet::new();
        for code in super::ALL {
            assert!(seen.insert(*code), "duplicate reason: {code}");
            assert!(["WORKFLOW_"].iter().any(|prefix| code.starts_with(prefix)));
            assert!(
                code.bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            );
        }
    }
}
