pub(crate) use crate::*;

pub(crate) mod discovery;
pub(crate) mod filename_parser;
pub mod filesystem_walk;
pub mod leftover_title_folders;
pub(crate) mod movie_scan_scope;
pub(crate) mod nfo;
pub(crate) mod pending_imports;
pub mod recycle_bin;
pub(crate) mod rename;
pub(crate) mod title_matching;
pub(crate) mod user_delete;
#[path = "library.rs"]
pub(crate) mod workflow;

pub(crate) use workflow as library;

pub(crate) fn require_library_enabled(library: &scryer_domain::Library) -> AppResult<()> {
    if !library.enabled {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::DISABLED,
            format!("library '{}' is disabled", library.name),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{Library, MediaFacet};

    fn library(enabled: bool) -> Library {
        let now = Utc::now();
        Library {
            id: "movies".to_string(),
            facet: MediaFacet::Movie,
            name: "Movies".to_string(),
            slug: "movies".to_string(),
            is_default: true,
            enabled,
            roots: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn disabled_library_refusal_has_a_stable_reason_code() {
        let error = require_library_enabled(&library(false))
            .expect_err("disabled libraries should be refused");

        assert_eq!(
            error.validation_reason(),
            Some(crate::error_reasons::library::DISABLED)
        );
    }

    #[test]
    fn enabled_library_passes_the_guard() {
        require_library_enabled(&library(true)).expect("enabled libraries pass");
    }
}
