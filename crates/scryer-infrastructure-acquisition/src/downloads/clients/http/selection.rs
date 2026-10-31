//! What the native HTTP downloader fetches from a release, and where it
//! writes it.
//!
//! The host's selection is a candidate set: every resource of a transport
//! the client declared, minus thumbnails. This picks the one media file that
//! best fits the profile's ceiling and preferences, plus one subtitle sidecar
//! per wanted language, and names each output after the host's basename. The
//! next-best media files are kept, bounded, for when the server refuses the
//! chosen one.

use std::path::{Path, PathBuf};

use scryer_application::{AppError, AppResult, http_release_refusal};
use scryer_plugin_sdk::{
    HttpResourceRole, HttpSubtitleMode, HttpSubtitleRole, HttpTransport, PluginHttpDownloadSource,
    PluginHttpResource,
};

use super::state::HttpFileRole;

/// Longest output basename accepted from the host. The host keeps its own
/// well under this; anything longer is not a name this client wrote.
const MAX_BASENAME_LEN: usize = 200;
const MAX_EXTENSION_LEN: usize = 8;
/// Alternative media resources kept for a job, beyond its first choice.
pub(crate) const MAX_MEDIA_FALLBACKS: usize = 3;

/// What a job fetches: its files, and the media resources to fall back to,
/// best first, when the chosen one is refused.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannedDownload {
    pub output_dir: PathBuf,
    pub files: Vec<PlannedFile>,
    pub media_fallbacks: Vec<PluginHttpResource>,
}

/// One file the job will write.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannedFile {
    pub role: HttpFileRole,
    pub resource: PluginHttpResource,
    pub final_path: PathBuf,
    pub language: Option<String>,
}

/// Decide what to fetch and where it goes, or refuse with the reason.
///
/// Refusals carry a reason code from
/// [`scryer_application::http_release_refusal`] so callers can tell them
/// apart: the release offers nothing this client can fetch, only an audio
/// and video pair is on offer, nothing playable was selected, or no required
/// subtitle sidecar was offered.
pub(crate) fn plan_download(
    source: &PluginHttpDownloadSource,
    download_dir: &Path,
) -> AppResult<PlannedDownload> {
    let basename = validate_output_basename(&source.policy.output_basename)?;
    let offered = source.release.offered_transports();
    if !offered.contains(&HttpTransport::DirectFile) {
        return Err(AppError::validation_refused(
            http_release_refusal::UNSUPPORTED_TRANSPORT,
            format!(
                "this release offers only {}; the HTTP downloader fetches direct files",
                offered
                    .iter()
                    .map(transport_label)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    let selected: Vec<&PluginHttpResource> = if source.selected.resources.is_empty() {
        source.release.resources.iter().collect()
    } else {
        source
            .selected
            .resources
            .iter()
            .filter_map(|index| {
                usize::try_from(*index)
                    .ok()
                    .and_then(|index| source.release.resources.get(index))
            })
            .collect()
    };
    let direct: Vec<&PluginHttpResource> = selected
        .into_iter()
        .filter(|resource| resource.transport == HttpTransport::DirectFile)
        .collect();

    let media_candidates: Vec<&PluginHttpResource> = direct
        .iter()
        .copied()
        .filter(|resource| is_complete_media(resource))
        .collect();
    if media_candidates.is_empty() {
        let needs_muxing = direct.iter().any(|resource| {
            matches!(
                resource.role,
                HttpResourceRole::Video | HttpResourceRole::Audio
            )
        });
        if needs_muxing {
            return Err(AppError::validation_refused(
                http_release_refusal::SEPARATE_TRACKS,
                "this release offers separate audio and video tracks, which need muxing; the HTTP downloader does not mux",
            ));
        }
        return Err(AppError::validation_refused(
            http_release_refusal::NO_MEDIA_RESOURCE,
            "the selection holds no direct media file to download",
        ));
    }
    let media_candidates = media_candidates
        .into_iter()
        .filter(|resource| {
            // Unlabelled candidates have already been evaluated by the host using
            // release/title evidence. Never override an explicit mismatch.
            let explicit = resource
                .audio
                .as_ref()
                .and_then(|audio| audio.language.as_deref())
                .filter(|language| {
                    !language.trim().is_empty() && !language.eq_ignore_ascii_case("und")
                });
            explicit.is_none_or(|language| {
                source.selected.audio_languages.iter().all(|wanted| {
                    let actual =
                        scryer_application::normalize_detected_audio_language_code(language);
                    let wanted = scryer_application::normalize_detected_audio_language_code(wanted);
                    actual.is_some() && actual == wanted
                })
            })
        })
        .collect::<Vec<_>>();
    if media_candidates.is_empty() {
        return Err(AppError::validation_refused(
            http_release_refusal::NO_AUDIO_RESOURCE,
            "no media resource satisfies the required audio languages",
        ));
    }
    let ranked = rank_media(&media_candidates, source);
    let media = ranked[0];
    // Retrying the same URL under another candidate would only repeat the
    // refusal, so each URL is kept once.
    let mut media_fallbacks: Vec<PluginHttpResource> = Vec::new();
    for resource in ranked.iter().skip(1) {
        if media_fallbacks.len() >= MAX_MEDIA_FALLBACKS {
            break;
        }
        if resource.url != media.url && !media_fallbacks.iter().any(|kept| kept.url == resource.url)
        {
            media_fallbacks.push((*resource).clone());
        }
    }

    let output_dir = download_dir.join(basename);
    let mut files = vec![PlannedFile {
        role: HttpFileRole::Media,
        resource: media.clone(),
        final_path: media_final_path(&output_dir, basename, media),
        language: None,
    }];

    let subtitles = choose_subtitles(&direct, source)?;
    for subtitle in subtitles {
        let language = subtitle
            .subtitle
            .as_ref()
            .and_then(|track| track.language.as_deref())
            .and_then(sanitize_token);
        let extension = extension_for(
            subtitle,
            subtitle
                .subtitle
                .as_ref()
                .and_then(|track| track.format.as_deref()),
        )
        .unwrap_or_else(|| "srt".to_string());
        let mut stem = basename.to_string();
        if let Some(language) = language.as_deref() {
            stem.push('.');
            stem.push_str(language);
        }
        match subtitle.subtitle.as_ref().and_then(|track| track.role) {
            Some(HttpSubtitleRole::Forced) => stem.push_str(".forced"),
            Some(HttpSubtitleRole::Sdh) => stem.push_str(".sdh"),
            _ => {}
        }
        let mut final_path = output_dir.join(format!("{stem}.{extension}"));
        let mut suffix = 2;
        while files.iter().any(|file| file.final_path == final_path) {
            final_path = output_dir.join(format!("{stem}.{suffix}.{extension}"));
            suffix += 1;
        }
        files.push(PlannedFile {
            role: HttpFileRole::Subtitle,
            resource: subtitle.clone(),
            final_path,
            language,
        });
    }

    Ok(PlannedDownload {
        output_dir,
        files,
        media_fallbacks,
    })
}

/// Where a media resource lands: the basename with the resource's own
/// extension, so a fallback in another container gets the right one. With
/// no container stated and no extension in the URL, the file is planned as
/// `.bin` and renamed for the container its probe reads before it is
/// published.
pub(crate) fn media_final_path(
    output_dir: &Path,
    basename: &str,
    resource: &PluginHttpResource,
) -> PathBuf {
    let extension = extension_for(resource, None).unwrap_or_else(|| "bin".to_string());
    output_dir.join(format!("{basename}.{extension}"))
}

/// The host's basename, refused unless it is a single plain path component.
pub(crate) fn validate_output_basename(raw: &str) -> AppResult<&str> {
    let basename = raw.trim();
    let valid = !basename.is_empty()
        && basename.len() <= MAX_BASENAME_LEN
        && basename != "."
        && basename != ".."
        && !basename.starts_with('.')
        && basename
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'));
    if valid {
        Ok(basename)
    } else {
        Err(AppError::validation_refused(
            crate::error_reasons::ACQUISITION_OUTPUT_FILENAME_INVALID,
            format!("the output name {raw:?} is not a plain file name"),
        ))
    }
}

/// A file that plays on its own: a muxed file, or a video resource that does
/// not depend on a separate audio track.
fn is_complete_media(resource: &PluginHttpResource) -> bool {
    match resource.role {
        HttpResourceRole::Muxed => true,
        HttpResourceRole::Video => !resource.separate_tracks,
        _ => false,
    }
}

/// The candidates best first: the tallest within the profile's ceiling (the
/// shortest when none fits), then the preferred codec, then the preferred
/// container, then the indexer's order. When any candidate fits the ceiling,
/// only those that fit are returned, so a fallback never exceeds it.
fn rank_media<'a>(
    candidates: &[&'a PluginHttpResource],
    source: &PluginHttpDownloadSource,
) -> Vec<&'a PluginHttpResource> {
    let height = |resource: &PluginHttpResource| resource.video.as_ref().and_then(|v| v.height);
    let ceiling = source.selected.max_height;
    let within: Vec<&'a PluginHttpResource> = candidates
        .iter()
        .copied()
        .filter(|resource| match (ceiling, height(resource)) {
            (Some(ceiling), Some(height)) => height <= ceiling,
            _ => true,
        })
        .collect();
    let preference_rank = |preferences: &[String], value: Option<&str>| -> usize {
        value
            .and_then(|value| {
                preferences
                    .iter()
                    .position(|preferred| preferred.eq_ignore_ascii_case(value))
            })
            .unwrap_or(preferences.len())
    };
    let rank = |resource: &PluginHttpResource| {
        (
            preference_rank(
                &source.selected.video_codec_preference,
                resource.video.as_ref().and_then(|v| v.codec.as_deref()),
            ),
            preference_rank(
                &source.selected.container_preference,
                resource.container.as_deref(),
            ),
        )
    };
    // Stable sorts, so the indexer's order breaks ties.
    if within.is_empty() {
        let mut ranked = candidates.to_vec();
        ranked.sort_by_key(|resource| (height(resource).unwrap_or(u32::MAX), rank(resource)));
        return ranked;
    }
    let mut ranked = within;
    ranked.sort_by_key(|resource| {
        (
            std::cmp::Reverse(height(resource).unwrap_or(0)),
            rank(resource),
        )
    });
    ranked
}

/// One sidecar per wanted language, in the order the languages were asked
/// for, when the selection asks for sidecars at all.
fn choose_subtitles<'a>(
    direct: &[&'a PluginHttpResource],
    source: &PluginHttpDownloadSource,
) -> AppResult<Vec<&'a PluginHttpResource>> {
    let wants_sidecars = matches!(
        source.selected.subtitle_mode,
        HttpSubtitleMode::SidecarPreferred | HttpSubtitleMode::SidecarRequired
    );
    if !wants_sidecars {
        return Ok(Vec::new());
    }
    let available: Vec<&'a PluginHttpResource> = direct
        .iter()
        .copied()
        .filter(|resource| resource.role == HttpResourceRole::Subtitle)
        .collect();
    let mut chosen: Vec<&'a PluginHttpResource> = Vec::new();
    for wanted in &source.selected.subtitle_languages {
        let found = available.iter().copied().find(|resource| {
            resource
                .subtitle
                .as_ref()
                .and_then(|track| track.language.as_deref())
                .is_some_and(|language| language_matches(language, wanted))
                && !chosen.iter().any(|picked| std::ptr::eq(*picked, *resource))
        });
        if let Some(found) = found {
            chosen.push(found);
        }
    }
    if chosen.is_empty()
        && source.selected.subtitle_mode == HttpSubtitleMode::SidecarRequired
        && !source.selected.subtitle_languages.is_empty()
    {
        return Err(AppError::validation_refused(
            http_release_refusal::NO_SUBTITLE_RESOURCE,
            "subtitle sidecars are required but none in a wanted language was offered",
        ));
    }
    Ok(chosen)
}

/// Match canonical subtitle identities, retaining regional distinctions.
fn language_matches(offered: &str, wanted: &str) -> bool {
    scryer_application::subtitles::language::same_subtitle_language(offered, wanted)
}

/// The output extension: the stated format or container, else the URL's
/// own extension. Only short alphanumeric tokens are accepted, so nothing
/// from the indexer can reach the file system as a path.
pub(crate) fn extension_for(resource: &PluginHttpResource, format: Option<&str>) -> Option<String> {
    format
        .and_then(sanitize_extension)
        .or_else(|| resource.container.as_deref().and_then(sanitize_extension))
        .or_else(|| url_extension(&resource.url))
}

fn url_extension(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    let last = url.path_segments()?.next_back()?.to_string();
    let (_, extension) = last.rsplit_once('.')?;
    sanitize_extension(extension)
}

fn sanitize_extension(raw: &str) -> Option<String> {
    let value = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    (!value.is_empty()
        && value.len() <= MAX_EXTENSION_LEN
        && value.chars().all(|ch| ch.is_ascii_alphanumeric()))
    .then_some(value)
}

fn sanitize_token(raw: &str) -> Option<String> {
    let value: String = raw
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .take(16)
        .collect();
    (!value.is_empty()).then(|| value.to_ascii_lowercase())
}

fn transport_label(transport: &HttpTransport) -> &'static str {
    match transport {
        HttpTransport::ProviderPage => "provider_page",
        HttpTransport::DirectFile => "direct_file",
        HttpTransport::Hls => "hls",
        HttpTransport::Dash => "dash",
    }
}
