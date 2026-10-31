//! Paths as a WASI guest sees them.
//!
//! A guest path is always `/`-separated, whatever the host OS: the guest's
//! libc matches a path to its preopen on `/` alone, so a host separator such
//! as Windows' `\` leaves the path matching no preopen and the guest reports
//! the file missing. Every adapter that hands a path to a guest builds it
//! here rather than with [`std::path::Path::join`].

use std::path::{Component, Path};

/// A guest path under `root`, built from the normal components of
/// `relative`. Current-dir, parent, root and prefix components are dropped,
/// so a host-built relative path never contributes a host separator.
pub(crate) fn guest_child_path(root: &str, relative: &Path) -> String {
    let mut guest_path = root.trim_end_matches('/').to_string();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            guest_path.push('/');
            guest_path.push_str(&part.to_string_lossy());
        }
    }
    guest_path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_a_plain_name_with_a_forward_slash() {
        assert_eq!(
            guest_child_path("/subtitle", Path::new("synthetic-track.srt")),
            "/subtitle/synthetic-track.srt"
        );
        assert_eq!(
            guest_child_path("/subtitle/", Path::new("./synthetic-track.srt")),
            "/subtitle/synthetic-track.srt"
        );
    }

    #[test]
    fn joins_host_built_nested_paths_with_forward_slashes() {
        // `Path::join` uses the host separator, so on Windows this relative
        // path carries a `\` between its components.
        let relative = Path::new("Synthetic Pack").join("synthetic-track.srt");
        let guest = guest_child_path("/reference", &relative);
        assert_eq!(guest, "/reference/Synthetic Pack/synthetic-track.srt");
        assert!(!guest.contains('\\'), "{guest}");
    }

    #[cfg(windows)]
    #[test]
    fn joins_windows_separated_names_with_forward_slashes() {
        assert_eq!(
            guest_child_path("/output", Path::new(r"Synthetic Pack\synthetic-track.srt")),
            "/output/Synthetic Pack/synthetic-track.srt"
        );
    }
}
