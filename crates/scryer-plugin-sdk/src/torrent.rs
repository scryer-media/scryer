use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::{
    DownloadInputKind, DownloadTorrentCapabilities, PluginCompletedDownload,
    PluginDownloadClientAddRequest, PluginDownloadItem, PluginDownloadOutputKind,
    PluginDownloadRelease, PluginDownloadSource, PluginTorrentItem,
};

pub fn choose_source_kind(
    capabilities: Option<&DownloadTorrentCapabilities>,
    request: &PluginDownloadClientAddRequest,
) -> Option<DownloadInputKind> {
    let preferred = request
        .torrent
        .as_ref()
        .map(|torrent| torrent.source_preference.as_slice())
        .filter(|values| !values.is_empty())
        .or_else(|| {
            capabilities
                .map(|torrent| torrent.preferred_sources.as_slice())
                .filter(|values| !values.is_empty())
        });

    let supported = capabilities
        .map(|torrent| torrent.supported_sources.as_slice())
        .filter(|values| !values.is_empty());

    let mut candidates = Vec::new();
    if let Some(preferred) = preferred {
        candidates.extend_from_slice(preferred);
    } else if let Some(supported) = supported {
        candidates.extend_from_slice(supported);
    }

    if candidates.is_empty() {
        candidates.extend([
            DownloadInputKind::MagnetUri,
            DownloadInputKind::TorrentBytes,
            DownloadInputKind::TorrentUrl,
            DownloadInputKind::TorrentFile,
            DownloadInputKind::Nzb,
            DownloadInputKind::NzbUrl,
        ]);
    }

    candidates
        .into_iter()
        .find(|candidate| source_kind_available(*candidate, &request.source))
        .or_else(|| fallback_source_kind(&request.source))
}

pub fn decode_torrent_bytes(source: &PluginDownloadSource) -> Result<Option<Vec<u8>>, String> {
    source
        .torrent_bytes_base64
        .as_deref()
        .map(|value| {
            BASE64
                .decode(value)
                .map_err(|error| format!("invalid torrent_bytes_base64: {error}"))
        })
        .transpose()
}

pub fn normalize_info_hash(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .chars()
                .filter(|ch| ch.is_ascii_hexdigit())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|value| matches!(value.len(), 40 | 64))
}

pub fn normalize_info_hash_pair(
    release: &PluginDownloadRelease,
) -> (Option<String>, Option<String>) {
    let info_hash_v1 = normalize_info_hash(
        release
            .info_hash_v1
            .as_deref()
            .or(release.info_hash_hint.as_deref()),
    )
    .filter(|value| value.len() == 40);
    let info_hash_v2 =
        normalize_info_hash(release.info_hash_v2.as_deref()).filter(|value| value.len() == 64);
    (info_hash_v1, info_hash_v2)
}

pub fn seed_seconds_to_minutes(seconds: Option<i64>) -> Option<i64> {
    seconds.and_then(|seconds| {
        if seconds <= 0 {
            None
        } else {
            Some((seconds + 59) / 60)
        }
    })
}

pub fn attach_torrent_projection(item: &mut PluginDownloadItem, torrent: PluginTorrentItem) {
    item.torrent = Some(torrent);
}

pub fn set_completed_output(
    item: &mut PluginCompletedDownload,
    output_kind: PluginDownloadOutputKind,
    content_paths: Vec<String>,
) {
    item.output_kind = Some(output_kind);
    item.content_paths = content_paths;
}

fn has_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn source_kind_available(kind: DownloadInputKind, source: &PluginDownloadSource) -> bool {
    match kind {
        DownloadInputKind::TorrentBytes => has_value(source.torrent_bytes_base64.as_deref()),
        DownloadInputKind::TorrentFile => {
            has_value(source.torrent_bytes_base64.as_deref())
                || (source.kind == DownloadInputKind::TorrentFile
                    && has_value(source.download_url.as_deref()))
        }
        DownloadInputKind::TorrentUrl => {
            source
                .torrent_url
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                || (source.kind == DownloadInputKind::TorrentUrl
                    && has_value(source.download_url.as_deref()))
        }
        DownloadInputKind::MagnetUri => source
            .magnet_uri
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        DownloadInputKind::Nzb => has_value(source.nzb_bytes_base64.as_deref()),
        DownloadInputKind::NzbUrl => {
            source.kind == DownloadInputKind::NzbUrl && has_value(source.download_url.as_deref())
        }
        DownloadInputKind::HttpResource => {
            source.kind == DownloadInputKind::HttpResource && source.http.is_some()
        }
    }
}

fn fallback_source_kind(source: &PluginDownloadSource) -> Option<DownloadInputKind> {
    if source_kind_available(DownloadInputKind::MagnetUri, source) {
        Some(DownloadInputKind::MagnetUri)
    } else if source_kind_available(DownloadInputKind::TorrentBytes, source) {
        Some(DownloadInputKind::TorrentBytes)
    } else if source_kind_available(DownloadInputKind::TorrentUrl, source) {
        Some(DownloadInputKind::TorrentUrl)
    } else if source_kind_available(DownloadInputKind::TorrentFile, source) {
        Some(DownloadInputKind::TorrentFile)
    } else if source_kind_available(DownloadInputKind::Nzb, source) {
        Some(DownloadInputKind::Nzb)
    } else if source_kind_available(DownloadInputKind::NzbUrl, source) {
        Some(DownloadInputKind::NzbUrl)
    } else {
        None
    }
}
