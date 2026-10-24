use crate::{AppError, AppResult, DownloadClientStatus};
use scryer_domain::CompletedDownload;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const REMOTE_PATH_MAPPINGS_KEY: &str = "remote_path_mappings";
const LEGACY_REMOTE_PATH_MAPPINGS_KEY: &str = "remotePathMappings";
const REMOTE_PATH_MAPPING_APPLIED_KEY: &str = "*scryer_remote_path_mapping_applied";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemotePathStyle {
    Unix,
    Windows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadClientRemotePathMapping {
    local_root: String,
    joined_remote_root: String,
    normalized_remote_root: String,
    remote_style: RemotePathStyle,
}

impl DownloadClientRemotePathMapping {
    pub fn local_root(&self) -> &str {
        &self.local_root
    }

    fn duplicate_key(&self) -> String {
        format!("{:?}:{}", self.remote_style, self.normalized_remote_root)
    }
}

pub fn parse_download_client_remote_path_mappings(
    config_json: &str,
) -> AppResult<Vec<DownloadClientRemotePathMapping>> {
    let config = parse_download_client_config_json(config_json)?;
    let Some(raw_value) = read_remote_path_mappings_value(&config)? else {
        return Ok(Vec::new());
    };

    let mut mappings = Vec::new();
    let mut seen = HashSet::new();

    for (line_index, raw_line) in raw_value.lines().enumerate() {
        let trimmed_line = raw_line.trim();
        if trimmed_line.is_empty() {
            continue;
        }

        let Some((remote_raw, local_raw)) = raw_line.split_once("=>") else {
            return Err(AppError::Validation(format!(
                "remote path mappings line {} must use REMOTE => LOCAL format",
                line_index + 1
            )));
        };

        let remote_root = parse_remote_root(remote_raw, line_index + 1)?;

        let local_root = local_raw.trim();
        if local_root.is_empty() {
            return Err(AppError::Validation(format!(
                "remote path mappings line {} requires a local path",
                line_index + 1
            )));
        }

        if !Path::new(local_root).is_absolute() {
            return Err(AppError::Validation(format!(
                "remote path mappings line {} local path must be absolute: {local_root}",
                line_index + 1
            )));
        }

        let remote_style = detect_remote_path_style(remote_root);
        let joined_remote_root = normalize_remote_path_for_join(remote_root, remote_style);
        let normalized_remote_root = normalize_remote_path(remote_root, remote_style);
        let mapping = DownloadClientRemotePathMapping {
            local_root: local_root.to_string(),
            joined_remote_root,
            normalized_remote_root,
            remote_style,
        };

        if !seen.insert(mapping.duplicate_key()) {
            return Err(AppError::Validation(format!(
                "remote path mappings line {} duplicates an existing remote path root",
                line_index + 1
            )));
        }

        mappings.push(mapping);
    }

    Ok(mappings)
}

pub fn has_download_client_remote_path_mappings(config_json: &str) -> AppResult<bool> {
    Ok(!parse_download_client_remote_path_mappings(config_json)?.is_empty())
}

pub fn remap_remote_path(
    path: &str,
    mappings: &[DownloadClientRemotePathMapping],
) -> Option<String> {
    let trimmed_path = path.trim();
    if trimmed_path.is_empty() {
        return None;
    }

    let candidate_style = detect_remote_path_style(trimmed_path);
    let joined_candidate = normalize_remote_path_for_join(trimmed_path, candidate_style);
    let normalized_candidate = normalize_remote_path(trimmed_path, candidate_style);
    let mut best_match: Option<(&DownloadClientRemotePathMapping, String, usize)> = None;

    for mapping in mappings {
        if mapping.remote_style != candidate_style {
            continue;
        }

        let Some(_) = strip_remote_root(
            &mapping.normalized_remote_root,
            &normalized_candidate,
            candidate_style,
        ) else {
            continue;
        };

        let Some(relative_suffix) = strip_remote_root_for_join(
            &mapping.joined_remote_root,
            &joined_candidate,
            candidate_style,
        ) else {
            continue;
        };

        let specificity = mapping.normalized_remote_root.len();
        if best_match
            .as_ref()
            .is_none_or(|(_, _, current_specificity)| specificity > *current_specificity)
        {
            best_match = Some((mapping, relative_suffix.to_string(), specificity));
        }
    }

    best_match.and_then(|(mapping, relative_suffix, _)| {
        rebase_to_local_root(&mapping.local_root, &relative_suffix)
    })
}

pub fn apply_remote_path_mappings_to_completed_download(
    completed: &mut CompletedDownload,
    mappings: &[DownloadClientRemotePathMapping],
) {
    if completed_download_has_remote_path_mapping_applied(completed) {
        return;
    }

    if let Some(remapped_path) = remap_remote_path(&completed.dest_dir, mappings) {
        completed.dest_dir = remapped_path;
        mark_completed_download_remote_path_mapping_applied(completed);
    }
}

pub fn apply_remote_path_mappings_to_status(
    status: &mut DownloadClientStatus,
    mappings: &[DownloadClientRemotePathMapping],
) {
    status.remote_output_roots = status
        .remote_output_roots
        .iter()
        .map(|path| remap_remote_path(path, mappings).unwrap_or_else(|| path.clone()))
        .collect();
}

fn parse_download_client_config_json(config_json: &str) -> AppResult<serde_json::Value> {
    let trimmed = config_json.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::json!({}));
    }

    serde_json::from_str(trimmed).map_err(|error| {
        AppError::Validation(format!("invalid download client config JSON: {error}"))
    })
}

fn read_remote_path_mappings_value(config: &serde_json::Value) -> AppResult<Option<&str>> {
    let Some(value) = config
        .get(REMOTE_PATH_MAPPINGS_KEY)
        .or_else(|| config.get(LEGACY_REMOTE_PATH_MAPPINGS_KEY))
    else {
        return Ok(None);
    };

    if value.is_null() {
        return Ok(None);
    }

    value
        .as_str()
        .map(Some)
        .ok_or_else(|| AppError::Validation("remote_path_mappings must be a string".to_string()))
}

fn parse_remote_root(remote_raw: &str, line_number: usize) -> AppResult<&str> {
    let remote_root = remote_raw.strip_suffix(' ').unwrap_or(remote_raw);

    if remote_root.trim().is_empty() {
        return Err(AppError::Validation(format!(
            "remote path mappings line {} requires a remote path",
            line_number
        )));
    }

    if remote_root.chars().next().is_some_and(char::is_whitespace) {
        return Err(AppError::Validation(format!(
            "remote path mappings line {} remote path cannot start with whitespace",
            line_number
        )));
    }

    if remote_root.chars().last().is_some_and(char::is_whitespace) {
        return Err(AppError::Validation(format!(
            "remote path mappings line {} remote path cannot end with whitespace",
            line_number
        )));
    }

    Ok(remote_root)
}

fn detect_remote_path_style(path: &str) -> RemotePathStyle {
    if path.contains('\\') || is_windows_drive_path(path) || path.starts_with("\\\\") {
        RemotePathStyle::Windows
    } else {
        RemotePathStyle::Unix
    }
}

fn is_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn normalize_remote_path(path: &str, style: RemotePathStyle) -> String {
    match style {
        RemotePathStyle::Unix => normalize_unix_remote_path(path),
        RemotePathStyle::Windows => normalize_windows_remote_path(path, true),
    }
}

fn normalize_remote_path_for_join(path: &str, style: RemotePathStyle) -> String {
    match style {
        RemotePathStyle::Unix => normalize_unix_remote_path(path),
        RemotePathStyle::Windows => normalize_windows_remote_path(path, false),
    }
}

fn normalize_unix_remote_path(path: &str) -> String {
    let is_absolute = path.starts_with('/');
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    if is_absolute {
        if segments.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", segments.join("/"))
        }
    } else {
        segments.join("/")
    }
}

fn normalize_windows_remote_path(path: &str, lowercase: bool) -> String {
    let replaced = if lowercase {
        path.replace('\\', "/").to_ascii_lowercase()
    } else {
        path.replace('\\', "/")
    };

    if let Some(rest) = replaced.strip_prefix("//") {
        let segments = rest
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();

        return match segments.as_slice() {
            [] => "//".to_string(),
            [server] => format!("//{server}"),
            [server, share] => format!("//{server}/{share}"),
            [server, share, tail @ ..] => {
                format!("//{server}/{share}/{}", tail.join("/"))
            }
        };
    }

    if is_windows_drive_path(&replaced) {
        let drive = &replaced[..2];
        let rest = replaced[2..].trim_start_matches('/');
        if rest.is_empty() {
            return format!("{drive}/");
        }

        let segments = rest
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        return format!("{drive}/{}", segments.join("/"));
    }

    let is_absolute = replaced.starts_with('/');
    let segments = replaced
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    if is_absolute {
        if segments.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", segments.join("/"))
        }
    } else {
        segments.join("/")
    }
}

fn strip_remote_root<'a>(
    remote_root: &str,
    candidate: &'a str,
    style: RemotePathStyle,
) -> Option<&'a str> {
    if candidate == remote_root {
        return Some("");
    }

    let stripped = candidate.strip_prefix(remote_root)?;
    if is_volume_root(remote_root, style) {
        return Some(stripped);
    }

    stripped.strip_prefix('/')
}

fn strip_remote_root_for_join<'a>(
    remote_root: &str,
    candidate: &'a str,
    style: RemotePathStyle,
) -> Option<&'a str> {
    if candidate.len() < remote_root.len() {
        return None;
    }

    if candidate.len() == remote_root.len() {
        return Some("");
    }

    let stripped = candidate.get(remote_root.len()..)?;
    if is_volume_root(remote_root, style) {
        return Some(stripped);
    }

    stripped.strip_prefix('/')
}

fn is_volume_root(path: &str, style: RemotePathStyle) -> bool {
    match style {
        RemotePathStyle::Unix => path == "/",
        RemotePathStyle::Windows => {
            if path == "/" || path == "//" {
                return true;
            }

            path.len() == 3
                && path.as_bytes()[1] == b':'
                && path.as_bytes()[2] == b'/'
                && path.as_bytes()[0].is_ascii_alphabetic()
        }
    }
}

fn completed_download_has_remote_path_mapping_applied(completed: &CompletedDownload) -> bool {
    completed
        .parameters
        .iter()
        .any(|(key, _)| key == REMOTE_PATH_MAPPING_APPLIED_KEY)
}

fn mark_completed_download_remote_path_mapping_applied(completed: &mut CompletedDownload) {
    if completed_download_has_remote_path_mapping_applied(completed) {
        return;
    }

    completed.parameters.push((
        REMOTE_PATH_MAPPING_APPLIED_KEY.to_string(),
        "true".to_string(),
    ));
}

fn rebase_to_local_root(local_root: &str, relative_suffix: &str) -> Option<String> {
    if relative_suffix.is_empty() {
        return Some(local_root.to_string());
    }

    let mut rebased = PathBuf::from(local_root);
    for segment in relative_suffix
        .split('/')
        .filter(|segment| !segment.is_empty())
        .filter(|segment| *segment != ".")
    {
        if segment == ".." {
            return None;
        }

        rebased.push(segment);
    }

    Some(rebased.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_download_client_remote_path_mappings_returns_empty_when_field_missing() {
        let mappings =
            parse_download_client_remote_path_mappings(r#"{"host":"example","port":"8080"}"#)
                .expect("parse mappings");
        assert!(mappings.is_empty());
    }

    #[test]
    fn parse_download_client_remote_path_mappings_accepts_ui_serialized_remote_to_local_rules() {
        let mappings = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads/tv => /Volumes/media/downloads/tv\nD:\\Data\\Anime => /Volumes/anime"}"#,
        )
        .expect("parse mappings");

        assert_eq!(
            remap_remote_path("/downloads/tv/Show.Name", &mappings).as_deref(),
            Some("/Volumes/media/downloads/tv/Show.Name"),
        );
        assert_eq!(
            remap_remote_path(r#"D:\Data\Anime\Series.Name"#, &mappings).as_deref(),
            Some("/Volumes/anime/Series.Name"),
        );
    }

    #[test]
    fn parse_download_client_remote_path_mappings_rejects_remote_whitespace() {
        let error = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":" /downloads => /Volumes/downloads"}"#,
        )
        .expect_err("expected validation error");

        assert!(
            error
                .to_string()
                .contains("remote path cannot start with whitespace")
        );
    }

    #[test]
    fn parse_download_client_remote_path_mappings_rejects_remote_trailing_whitespace() {
        let error = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads  => /Volumes/downloads"}"#,
        )
        .expect_err("expected validation error");

        assert!(
            error
                .to_string()
                .contains("remote path cannot end with whitespace")
        );
    }

    #[test]
    fn parse_download_client_remote_path_mappings_rejects_non_absolute_local_path() {
        let error = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads => relative/path"}"#,
        )
        .expect_err("expected validation error");

        assert!(error.to_string().contains("local path must be absolute"));
    }

    #[test]
    fn parse_download_client_remote_path_mappings_rejects_duplicate_remote_roots() {
        let error = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads => /Volumes/downloads\n/downloads/ => /Volumes/other"}"#,
        )
        .expect_err("expected duplicate validation error");

        assert!(
            error
                .to_string()
                .contains("duplicates an existing remote path root")
        );
    }

    #[test]
    fn remap_remote_path_rewrites_exact_prefix_matches() {
        let mappings = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads => /Volumes/downloads"}"#,
        )
        .expect("parse mappings");

        let remapped =
            remap_remote_path("/downloads/show/episode.mkv", &mappings).expect("remapped path");

        assert_eq!(remapped, "/Volumes/downloads/show/episode.mkv");
    }

    #[test]
    fn remap_remote_path_prefers_the_most_specific_remote_root() {
        let mappings = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads => /Volumes/downloads\n/downloads/anime => /Volumes/anime"}"#,
        )
        .expect("parse mappings");

        let remapped =
            remap_remote_path("/downloads/anime/show/file.mkv", &mappings).expect("remapped path");

        assert_eq!(remapped, "/Volumes/anime/show/file.mkv");
    }

    #[test]
    fn remap_remote_path_returns_none_for_unmatched_paths() {
        let mappings = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads => /Volumes/downloads"}"#,
        )
        .expect("parse mappings");

        let remapped = remap_remote_path("/other/file.mkv", &mappings);

        assert!(remapped.is_none());
    }

    #[test]
    fn remap_remote_path_returns_none_for_parent_directory_segments() {
        let mappings = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads => /Volumes/downloads"}"#,
        )
        .expect("parse mappings");

        let remapped = remap_remote_path("/downloads/../secret/file.mkv", &mappings);

        assert!(remapped.is_none());
    }

    #[test]
    fn remap_remote_path_matches_windows_paths_case_insensitively() {
        let mappings = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"D:\\Data\\Completed => /Volumes/downloads"}"#,
        )
        .expect("parse mappings");

        let remapped =
            remap_remote_path("d:/data/completed/Show/File.mkv", &mappings).expect("remapped path");

        assert_eq!(remapped, "/Volumes/downloads/Show/File.mkv");
    }

    #[test]
    fn apply_remote_path_mappings_to_status_rewrites_remote_output_roots() {
        let mappings = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads => /Volumes/downloads"}"#,
        )
        .expect("parse mappings");
        let mut status = DownloadClientStatus {
            remote_output_roots: vec!["/downloads/complete".to_string()],
            ..DownloadClientStatus::default()
        };

        apply_remote_path_mappings_to_status(&mut status, &mappings);

        assert_eq!(
            status.remote_output_roots,
            vec!["/Volumes/downloads/complete".to_string()]
        );
    }

    #[test]
    fn apply_remote_path_mappings_to_completed_download_rewrites_dest_dir() {
        let mappings = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads => /Volumes/downloads"}"#,
        )
        .expect("parse mappings");
        let mut completed = CompletedDownload {
            subtitle_paths: Vec::new(),
            client_type: "qbittorrent".to_string(),
            client_id: "client-1".to_string(),
            download_client_item_id: "item-1".to_string(),
            download_id: None,
            name: "Example".to_string(),
            release_name: None,
            dest_dir: "/downloads/Example".to_string(),
            category: None,
            size_bytes: None,
            completed_at: None,
            parameters: Vec::new(),
            posted_file_names: Vec::new(),
        };

        apply_remote_path_mappings_to_completed_download(&mut completed, &mappings);

        assert_eq!(completed.dest_dir, "/Volumes/downloads/Example");
    }

    #[test]
    fn apply_remote_path_mappings_to_completed_download_is_idempotent_for_nested_local_roots() {
        let mappings = parse_download_client_remote_path_mappings(
            r#"{"remote_path_mappings":"/downloads => /downloads/local"}"#,
        )
        .expect("parse mappings");
        let mut completed = CompletedDownload {
            subtitle_paths: Vec::new(),
            client_type: "qbittorrent".to_string(),
            client_id: "client-1".to_string(),
            download_client_item_id: "item-1".to_string(),
            download_id: None,
            name: "Example".to_string(),
            release_name: None,
            dest_dir: "/downloads/Example".to_string(),
            category: None,
            size_bytes: None,
            completed_at: None,
            parameters: Vec::new(),
            posted_file_names: Vec::new(),
        };

        apply_remote_path_mappings_to_completed_download(&mut completed, &mappings);
        apply_remote_path_mappings_to_completed_download(&mut completed, &mappings);

        assert_eq!(completed.dest_dir, "/downloads/local/Example");
        assert_eq!(
            completed
                .parameters
                .iter()
                .filter(|(key, _)| key == REMOTE_PATH_MAPPING_APPLIED_KEY)
                .count(),
            1
        );
    }
}
