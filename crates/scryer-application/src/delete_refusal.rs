//! Reasons shared by media-file, episode-file, and subtitle deletion.

/// The confirmed file set changed.
pub const PREVIEW_STALE: &str = "DELETE_PREVIEW_STALE";
/// The selection requires typed confirmation.
pub const TYPED_CONFIRMATION_REQUIRED: &str = "DELETE_TYPED_CONFIRMATION_REQUIRED";
/// A required library root is unavailable.
pub const ROOT_UNAVAILABLE: &str = "DELETE_ROOT_UNAVAILABLE";
/// A path lies outside every configured library root.
pub const PATH_OUTSIDE_ROOTS: &str = "DELETE_PATH_OUTSIDE_ROOTS";
/// Every shared deletion refusal.
pub const ALL: &[&str] = &[
    PREVIEW_STALE,
    TYPED_CONFIRMATION_REQUIRED,
    ROOT_UNAVAILABLE,
    PATH_OUTSIDE_ROOTS,
];

/// Add context only to uncoded validation; retain typed faults and existing reasons.
pub(crate) fn classify_validation(error: crate::AppError, reason: &'static str) -> crate::AppError {
    match error {
        crate::AppError::Validation(message) => {
            crate::AppError::validation_refused(reason, message)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delete_refusals_are_distinct_and_preserve_error_classes() {
        let mut seen = std::collections::HashSet::new();
        for reason in ALL {
            assert!(reason.starts_with("DELETE_"));
            assert!(reason.bytes().all(|c| c.is_ascii_uppercase() || c == b'_'));
            assert!(seen.insert(reason));
        }
        let error = classify_validation(
            crate::AppError::Repository("database unavailable".into()),
            ROOT_UNAVAILABLE,
        );
        assert!(matches!(error, crate::AppError::Repository(_)));
        let error = classify_validation(
            crate::AppError::validation_refused(PREVIEW_STALE, "changed"),
            ROOT_UNAVAILABLE,
        );
        assert_eq!(error.validation_reason(), Some(PREVIEW_STALE));
        let error = classify_validation(
            crate::AppError::Validation("offline".into()),
            ROOT_UNAVAILABLE,
        );
        assert_eq!(error.to_string(), "validation: offline");
    }

    #[test]
    fn path_outside_every_root_keeps_its_own_reason() {
        let root = std::env::temp_dir().join("scryer-synthetic-delete-root");
        let error = crate::fs_safety::resolve_available_root_for_path(
            std::path::Path::new("/scryer-synthetic-elsewhere/file.mkv"),
            &[root],
        )
        .unwrap_err();
        let error = classify_validation(error, ROOT_UNAVAILABLE);
        assert_eq!(error.validation_reason(), Some(PATH_OUTSIDE_ROOTS));
    }
}
