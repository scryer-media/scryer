//! How Scryer spells a native path as a stored string, and how two stored
//! paths are compared. The definitions live in the disk service crate; this
//! module keeps them reachable under the paths the application has always
//! used.

#[cfg(test)]
pub(crate) use scryer_disk::stored_paths::folder_path_identity_key_for_platform;
pub use scryer_disk::stored_paths::{
    folder_path_identity_key, folder_path_lookup_key_for_platform, folder_path_match_candidates,
    folder_paths_match, is_escaped_stored_path, lexically_normalize, path_identity_key,
    path_to_stored_string, paths_match, paths_match_ignoring_case, stored_file_name,
    stored_path_to_display_string, stored_path_to_path_buf,
};
pub(crate) use scryer_disk::stored_paths::{
    lowercase_os_str, native_name_suffix, stored_path_is_lexically_within_folder,
    stored_path_is_native, stored_path_is_within_folder, suffix_path_stem,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    #[cfg(windows)]
    use std::os::windows::ffi::OsStringExt;

    #[cfg(any(unix, windows))]
    fn opaque_path(marker: u16) -> PathBuf {
        #[cfg(unix)]
        let name = OsString::from_vec([b"/library/\xC3\x84".as_slice(), &[marker as u8]].concat());
        #[cfg(windows)]
        let name = OsString::from_wide(
            &[
                "C:\\library\\\u{c4}".encode_utf16().collect::<Vec<_>>(),
                vec![marker],
            ]
            .concat(),
        );
        PathBuf::from(name)
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn native_path_containment_preserves_opaque_units_and_root_boundaries() {
        #[cfg(unix)]
        let markers = [0xff, 0xfe];
        #[cfg(windows)]
        let markers = [0xd800, 0xd801];
        let first = opaque_path(markers[0]);
        let second = opaque_path(markers[1]);
        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        let first_key = path_to_stored_string(&first);
        let second_key = path_to_stored_string(&second);
        let child = path_to_stored_string(first.join("movie.mkv"));
        assert!(crate::title_folder_rules::path_is_strictly_within(
            &child, &first_key
        ));
        assert!(!crate::title_folder_rules::path_is_strictly_within(
            &child,
            &second_key
        ));
        assert!(!crate::title_folder_rules::path_is_strictly_within(
            &path_to_stored_string(first.join("../outside.mkv")),
            &first_key
        ));
        assert!(!crate::title_folder_rules::path_is_strictly_within(
            &first_key, &first_key
        ));
    }
}
