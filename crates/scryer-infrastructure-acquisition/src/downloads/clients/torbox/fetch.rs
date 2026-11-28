//! Which files of a finished TorBox item Scryer fetches, and where they land.
//!
//! Only the playable video files and the subtitle sidecars are fetched;
//! everything else an item carries (samples' companions, NFOs, archives,
//! par2 sets) stays on TorBox. TorBox's file names are paths chosen by
//! whoever made the release, so every component is rewritten into a plain,
//! portable name the same way every time.

use std::collections::HashSet;
use std::path::Path;

use scryer_application::torbox_refusal;

use super::api::{TorboxItem, TorboxItemKind};
use crate::downloads::clients::http::owned::{HttpFileRole, LinkedFile};

/// Longest single path component, in bytes.
const MAX_COMPONENT_LEN: usize = 255;
/// Longest relative path, in bytes.
const MAX_RELATIVE_PATH_LEN: usize = 1024;
/// Names Windows reserves for devices, whatever the extension.
const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Why an item's files cannot be fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FetchRefusal {
    pub reason: &'static str,
    pub message: String,
}

impl FetchRefusal {
    fn new(reason: &'static str, message: String) -> Self {
        Self { reason, message }
    }
}

/// The engine's key for one file's link: the item family, the item id and
/// the file id.
pub(crate) fn link_key(kind: TorboxItemKind, item_id: i64, file_id: i64) -> String {
    let family = match kind {
        TorboxItemKind::Torrent => 't',
        TorboxItemKind::Usenet => 'u',
    };
    format!("{family}:{item_id}:{file_id}")
}

/// The item and file a link key names.
pub(crate) fn parse_link_key(key: &str) -> Option<(TorboxItemKind, i64, i64)> {
    let mut parts = key.split(':');
    let kind = match parts.next()? {
        "t" => TorboxItemKind::Torrent,
        "u" => TorboxItemKind::Usenet,
        _ => return None,
    };
    let item_id = parts.next()?.parse().ok()?;
    let file_id = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((kind, item_id, file_id))
}

/// One component rewritten into a plain portable name: characters a
/// filesystem reads as a separator, a drive, a wildcard or a control become
/// `_`, trailing dots and spaces go, a device name gets a `_` in front, and
/// an over-long name is cut, keeping its extension.
pub(crate) fn sanitize_component(raw: &str) -> String {
    let mut name: String = raw
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    while name.ends_with('.') || name.ends_with(' ') {
        name.pop();
    }
    if name.is_empty() {
        return "_".to_string();
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_lowercase();
    if RESERVED_DEVICE_NAMES.contains(&stem.as_str()) {
        name.insert(0, '_');
    }
    if name.len() > MAX_COMPONENT_LEN {
        name = shorten(&name);
    }
    name
}

/// Cut a name to the component limit, keeping a short extension.
fn shorten(name: &str) -> String {
    let extension = name
        .rfind('.')
        .map(|dot| &name[dot..])
        .filter(|extension| extension.len() <= 16)
        .unwrap_or("");
    let mut cut = MAX_COMPONENT_LEN - extension.len();
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut stem = name[..cut].to_string();
    while stem.ends_with('.') || stem.ends_with(' ') {
        stem.pop();
    }
    if stem.is_empty() {
        stem.push('_');
    }
    format!("{stem}{extension}")
}

/// The job folder for a release name: one plain component that does not
/// start with a dot.
pub(crate) fn job_folder_for(release_name: &str) -> String {
    let folder = sanitize_component(release_name);
    let trimmed = folder.trim_start_matches('.');
    if trimmed.is_empty() {
        "_".to_string()
    } else if trimmed.len() == folder.len() {
        folder
    } else {
        sanitize_component(trimmed)
    }
}

/// A folder no other job of this client uses, compared without case: the
/// plain name, else the name with ` (2)`, ` (3)` and so on.
pub(crate) fn unique_job_folder(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    (2..)
        .map(|n| {
            let suffix = format!(" ({n})");
            let mut cut = MAX_COMPONENT_LEN
                .saturating_sub(suffix.len())
                .min(base.len());
            while !base.is_char_boundary(cut) {
                cut -= 1;
            }
            format!("{}{suffix}", &base[..cut])
        })
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
        .expect("an unused folder name exists")
}

/// A TorBox file path as plain components, separators of either kind.
fn components(raw: &str) -> Vec<String> {
    raw.split(['/', '\\'])
        .map(str::trim)
        .filter(|component| !component.is_empty() && *component != ".")
        .map(sanitize_component)
        .collect()
}

/// The files to fetch from a finished item, planned under the job folder.
///
/// An item holding any infected file is refused outright, as is one whose
/// wanted files TorBox only offers zipped. An item with no video is refused.
/// When every chosen file sits under the same top folder (the item's own
/// folder), that folder is dropped so the job folder does not repeat it.
pub(crate) fn plan_fetch(
    kind: TorboxItemKind,
    item: &TorboxItem,
) -> Result<Vec<LinkedFile>, FetchRefusal> {
    if let Some(file) = item.files.iter().find(|file| file.infected) {
        return Err(FetchRefusal::new(
            torbox_refusal::FILE_INFECTED,
            format!(
                "TorBox flagged {} in this download as infected; nothing was fetched",
                display_name(file.name.as_deref().or(file.short_name.as_deref()))
            ),
        ));
    }
    let mut chosen: Vec<(HttpFileRole, Vec<String>, &super::api::TorboxFile)> = Vec::new();
    for file in &item.files {
        let Some(raw) = file
            .name
            .as_deref()
            .or(file.short_name.as_deref())
            .filter(|name| !name.trim().is_empty())
        else {
            continue;
        };
        let parts = components(raw);
        let Some(leaf) = parts.last() else {
            continue;
        };
        let role = if scryer_domain::is_standalone_video_file(Path::new(leaf)) {
            HttpFileRole::Media
        } else if scryer_domain::is_subtitle_file(Path::new(leaf)) {
            HttpFileRole::Subtitle
        } else {
            continue;
        };
        if file.zipped {
            return Err(FetchRefusal::new(
                torbox_refusal::FILE_ZIPPED,
                format!(
                    "TorBox offers {} only inside a zip, which Scryer does not fetch",
                    display_name(Some(raw))
                ),
            ));
        }
        chosen.push((role, parts, file));
    }
    if !chosen.iter().any(|(role, ..)| *role == HttpFileRole::Media) {
        return Err(FetchRefusal::new(
            torbox_refusal::NO_VIDEO_FILE,
            "the download holds no video file".to_string(),
        ));
    }

    let shared_top = chosen
        .first()
        .filter(|(_, parts, _)| parts.len() > 1)
        .map(|(_, parts, _)| parts[0].to_lowercase())
        .filter(|top| {
            chosen
                .iter()
                .all(|(_, parts, _)| parts.len() > 1 && parts[0].to_lowercase() == *top)
        });
    let mut seen: HashSet<String> = HashSet::with_capacity(chosen.len());
    let mut planned = Vec::with_capacity(chosen.len());
    for (role, parts, file) in chosen {
        let parts = if shared_top.is_some() {
            &parts[1..]
        } else {
            &parts[..]
        };
        let relative_path = parts.join("/");
        if relative_path.len() > MAX_RELATIVE_PATH_LEN {
            return Err(FetchRefusal::new(
                torbox_refusal::FILE_PATHS_UNUSABLE,
                format!("a file path in the download is longer than {MAX_RELATIVE_PATH_LEN} bytes"),
            ));
        }
        if !seen.insert(relative_path.to_lowercase()) {
            return Err(FetchRefusal::new(
                torbox_refusal::FILE_PATHS_UNUSABLE,
                format!("two files of the download would both be saved as {relative_path:?}"),
            ));
        }
        planned.push(LinkedFile {
            role,
            relative_path,
            link_key: link_key(kind, item.id, file.id),
            size_bytes: file.size.and_then(|size| u64::try_from(size).ok()),
            language: None,
        });
    }
    Ok(planned)
}

fn display_name(raw: Option<&str>) -> String {
    raw.and_then(|raw| raw.rsplit(['/', '\\']).next())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| format!("{name:?}"))
        .unwrap_or_else(|| "a file".to_string())
}
