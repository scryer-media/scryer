//! The boundary between Scryer's error type and the shared upgrade core.
//!
//! The core carries a host error back unchanged in its `Host` variant, so an
//! [`AppError`] that travels through the core (through an injected seam such as
//! the free-space check or progress reporting) arrives back as the very same
//! variant and message it left as.

use crate::AppError;

/// Wrap an application error so it can travel through the shared core.
pub fn to_updater_error(error: AppError) -> application_updater::Error {
    application_updater::Error::host(error)
}

/// Map a shared-core error onto the application error it is reported as.
pub fn map_updater_error(error: application_updater::Error) -> AppError {
    match error {
        application_updater::Error::Validation(message) => AppError::Validation(message),
        application_updater::Error::Repository(message) => AppError::Repository(message),
        application_updater::Error::NotFound(message) => AppError::NotFound(message),
        application_updater::Error::Host(host) => match host.downcast::<AppError>() {
            Ok(error) => *error,
            Err(error) => AppError::Repository(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_errors_round_trip_through_the_shared_core_unchanged() {
        for error in [
            AppError::Validation("nope".to_string()),
            AppError::Repository("disk".to_string()),
            AppError::NotFound("gone".to_string()),
        ] {
            let expected = error.to_string();
            let round_tripped = map_updater_error(to_updater_error(error));
            assert_eq!(round_tripped.to_string(), expected);
        }
    }

    #[test]
    fn core_errors_map_onto_the_same_application_variants() {
        assert!(matches!(
            map_updater_error(application_updater::Error::Validation("v".to_string())),
            AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message == "v"
        ));
        assert!(matches!(
            map_updater_error(application_updater::Error::Repository("r".to_string())),
            AppError::Repository(message) | AppError::RepositoryFailure { message, .. } | AppError::RepositoryFailed { message, .. } if message == "r"
        ));
        assert!(matches!(
            map_updater_error(application_updater::Error::NotFound("n".to_string())),
            AppError::NotFound(message) | AppError::NotFoundReason { message, .. } if message == "n"
        ));
    }
}
