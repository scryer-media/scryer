//! The HTTP-family grab: turning the release an `http` indexer listed into
//! the complete `http` block a download client receives.
//!
//! An HTTP release is not a download URL. The indexer describes an item
//! (provider, stable id, canonical page) and, optionally, the resources it
//! currently knows how to reach. Right before submission the host asks the
//! indexer to resolve the item again, refuses the answer if it describes a
//! different item, chooses what to fetch from the effective quality profile
//! and the subtitle language settings, and names the output.

use super::*;

use scryer_plugin_sdk::{
    HttpAvailability, HttpResourceRole, HttpSubtitleMode, HttpTransport, PluginHttpDownloadSource,
    PluginHttpPolicy, PluginHttpRelease, PluginHttpSelection, PluginHttpVerify,
};

use crate::quality::profile::QualityProfile;
use crate::release_parser::VideoCodec;

/// Host transport order before the router narrows it to what the chosen
/// client declared: a muxed file beats a segmented stream, and a provider
/// page is the last resort because only a page-capable client can take it.
const HOST_TRANSPORT_ORDER: [HttpTransport; 4] = [
    HttpTransport::DirectFile,
    HttpTransport::Hls,
    HttpTransport::Dash,
    HttpTransport::ProviderPage,
];

/// Longest release-title part of an output basename. Clients add only an
/// extension, so the whole name must stay well inside filesystem limits
/// once the download id is appended.
const OUTPUT_BASENAME_TITLE_LIMIT: usize = 120;

/// The language half of the selection, read from the subtitle settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HttpSubtitlePreferences {
    pub enabled: bool,
    pub languages: Vec<String>,
}

/// Compare a resolved release against the listed one. The identity the
/// grab was approved for is the provider's item id and its canonical page;
/// a resolve that answers with a different item must not be submitted.
pub(crate) fn verify_resolved_http_release(
    listed: &PluginHttpRelease,
    resolved: &PluginHttpRelease,
) -> AppResult<()> {
    if resolved.item_id != listed.item_id {
        return Err(AppError::Validation(format!(
            "the indexer resolved a different item ({} instead of {}); refusing to submit",
            resolved.item_id, listed.item_id
        )));
    }
    if resolved.page_url != listed.page_url {
        return Err(AppError::Validation(format!(
            "the indexer resolved a different page ({} instead of {}); refusing to submit",
            resolved.page_url, listed.page_url
        )));
    }
    Ok(())
}

/// An HTTP-family grab must carry the release its indexer listed. Without it
/// there is nothing to describe to the client, and the source hint is a
/// provider page, not an artifact the host may fetch. That happens when the
/// listing that offered the result has expired, so the grab is refused with a
/// prompt to search again rather than submitted as a page download.
pub(crate) fn require_http_release_for_http_source(
    source_kind: Option<DownloadSourceKind>,
    http_release: Option<&PluginHttpRelease>,
) -> AppResult<()> {
    if matches!(source_kind, Some(DownloadSourceKind::HttpResource)) && http_release.is_none() {
        return Err(AppError::Validation(
            "this HTTP release's details are no longer available; search again and grab it from the new results"
                .to_string(),
        ));
    }
    Ok(())
}

/// Refuse a release the provider says cannot be downloaded now. These are
/// safety guards, not user rules, so they apply to operator grabs as well:
/// a live stream, an announcement and a removed item would all reach the
/// client and fail there.
///
/// Every other availability is submitted and left to the client. A
/// geo-restricted release is submitted since the client retries a geo block
/// and an assigned proxy may route around it. An expired one is submitted for
/// the indexer to resolve again. `Unknown` is the default for indexers that do
/// not report availability.
pub(crate) fn refuse_unavailable_http_release(release: &PluginHttpRelease) -> AppResult<()> {
    let reason = match release.availability {
        HttpAvailability::Live => "this release is a live stream",
        HttpAvailability::Upcoming => "this release is not yet available",
        HttpAvailability::Removed => "this release was removed by the provider",
        HttpAvailability::Available
        | HttpAvailability::Drm
        | HttpAvailability::AuthRequired
        | HttpAvailability::Expired
        | HttpAvailability::GeoRestricted
        | HttpAvailability::Unknown => return Ok(()),
    };
    Err(AppError::Validation(format!(
        "{reason}, so it cannot be downloaded; refusing to submit"
    )))
}

/// Build the host selection for a release from the effective quality profile
/// and the subtitle preferences.
///
/// The profile expresses a resolution ceiling through its quality tiers and a
/// codec order through its allowlist; it has no container preference, so the
/// client default stands. Audio languages are the profile's required ones in
/// order. Subtitles are asked for as sidecars when the subtitle settings are
/// enabled with at least one language, since that is what the import path
/// already reconciles next to a media file.
pub(crate) fn build_http_selection(
    release: &PluginHttpRelease,
    profile: &QualityProfile,
    subtitles: &HttpSubtitlePreferences,
) -> PluginHttpSelection {
    let offered = release.offered_transports();
    let transport_preference: Vec<HttpTransport> = HOST_TRANSPORT_ORDER
        .iter()
        .copied()
        .filter(|transport| offered.contains(transport))
        .collect();
    let resources: Vec<u32> = release
        .resources
        .iter()
        .enumerate()
        .filter(|(_, resource)| {
            resource.role != HttpResourceRole::Thumbnail
                && transport_preference.contains(&resource.transport)
        })
        .filter_map(|(index, _)| u32::try_from(index).ok())
        .collect();
    let subtitle_languages: Vec<String> = if subtitles.enabled {
        dedup_languages(subtitles.languages.iter().map(String::as_str))
    } else {
        Vec::new()
    };
    let subtitle_mode = if subtitle_languages.is_empty() {
        HttpSubtitleMode::None
    } else {
        HttpSubtitleMode::SidecarPreferred
    };
    PluginHttpSelection {
        transport_preference,
        resources,
        max_height: profile_max_height(profile),
        video_codec_preference: profile
            .criteria
            .video_codec_allowlist
            .iter()
            .map(http_video_codec_name)
            .collect(),
        container_preference: Vec::new(),
        audio_languages: dedup_languages(
            profile
                .criteria
                .required_audio_languages
                .iter()
                .map(String::as_str),
        ),
        audio_role: Default::default(),
        subtitle_languages,
        subtitle_mode,
        playlist_policy: Default::default(),
        clip: None,
    }
}

/// Build the output policy: a sanitised, unique basename and what the host
/// expects the completed media to be.
///
/// The expected size is exact only when the indexer reported one; a release
/// that offers only a provider page has none.
pub(crate) fn build_http_policy(
    release: &PluginHttpRelease,
    release_title: &str,
    download_id: &scryer_domain::download_identity::DownloadId,
    release_size_bytes: Option<i64>,
) -> PluginHttpPolicy {
    PluginHttpPolicy {
        output_basename: http_output_basename(release_title, download_id),
        expected_duration_seconds: release.duration_seconds,
        expected_size_bytes: release_size_bytes.filter(|size| *size > 0),
        verify: PluginHttpVerify::default(),
    }
}

/// `<sanitised title>-<download id>`. The download id makes the name unique
/// per grab even when the same item is grabbed twice; the title keeps it
/// readable in the client's queue and on disk.
pub(crate) fn http_output_basename(
    release_title: &str,
    download_id: &scryer_domain::download_identity::DownloadId,
) -> String {
    let mut title = String::new();
    let mut last_was_separator = true;
    for ch in release_title.trim().chars() {
        let keep = ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_');
        if keep {
            title.push(ch);
            last_was_separator = matches!(ch, '.' | '-' | '_');
        } else if !last_was_separator {
            title.push('_');
            last_was_separator = true;
        }
        if title.len() >= OUTPUT_BASENAME_TITLE_LIMIT {
            break;
        }
    }
    let title = title.trim_matches(|ch| matches!(ch, '.' | '-' | '_'));
    if title.is_empty() {
        format!("download-{download_id}")
    } else {
        format!("{title}-{download_id}")
    }
}

/// The tallest frame the profile admits, read from its quality tiers
/// (`2160p`, `1080p`, ...). A profile whose tiers name no resolution has no
/// ceiling.
fn profile_max_height(profile: &QualityProfile) -> Option<u32> {
    profile
        .criteria
        .quality_tiers
        .iter()
        .filter_map(|tier| tier_height(tier))
        .max()
}

fn tier_height(tier: &str) -> Option<u32> {
    let tier = tier.trim();
    let digits = tier.strip_suffix(['p', 'P', 'i', 'I'])?;
    digits.parse::<u32>().ok().filter(|height| *height > 0)
}

/// The SDK's codec vocabulary (`av1`, `h265`, `h264`, ...) for a profile
/// codec.
fn http_video_codec_name(codec: &VideoCodec) -> String {
    match codec {
        VideoCodec::H264 => "h264".to_string(),
        VideoCodec::H265 => "h265".to_string(),
        VideoCodec::Av1 => "av1".to_string(),
        VideoCodec::Vp9 => "vp9".to_string(),
        other => other.as_str().to_ascii_lowercase(),
    }
}

fn dedup_languages<'a>(languages: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for language in languages {
        let language = language.trim().to_ascii_lowercase();
        if !language.is_empty() && !out.contains(&language) {
            out.push(language);
        }
    }
    out
}

impl AppUseCase {
    /// The complete `http` block for an `HttpResource` grab, or `None` when
    /// the intent carries no HTTP release.
    ///
    /// Runs the indexer's `resolve` action when the release declares one and
    /// refuses a result that names a different item, then builds the
    /// selection and policy from the title's effective quality profile and
    /// the subtitle settings.
    pub(crate) async fn build_http_download_source(
        &self,
        request: &DownloadClientAddRequest,
        listed: &PluginHttpRelease,
        download_id: &scryer_domain::download_identity::DownloadId,
        release_size_bytes: Option<i64>,
    ) -> AppResult<PluginHttpDownloadSource> {
        let release = self.resolve_http_release_for_grab(request, listed).await?;
        let profile = self
            .resolve_quality_profile_for_title(&request.title)
            .await?;
        let subtitle_settings = self.subtitle_settings().await?;
        let subtitles = HttpSubtitlePreferences {
            enabled: subtitle_settings.enabled,
            languages: subtitle_settings
                .languages
                .iter()
                .map(|language| language.code.clone())
                .collect(),
        };
        let selected = build_http_selection(&release, &profile, &subtitles);
        let release_title = request
            .release_title
            .as_deref()
            .or(request.source_title.as_deref())
            .unwrap_or(&request.title.name);
        let policy = build_http_policy(&release, release_title, download_id, release_size_bytes);
        Ok(PluginHttpDownloadSource {
            release,
            selected,
            policy,
        })
    }

    /// The release to submit: the listed one when it declares no `resolve`
    /// action or the indexer answers that it has none, otherwise the fresh
    /// one the indexer returned, verified to be the same item.
    async fn resolve_http_release_for_grab(
        &self,
        request: &DownloadClientAddRequest,
        listed: &PluginHttpRelease,
    ) -> AppResult<PluginHttpRelease> {
        if listed.resolve.is_none() {
            return Ok(listed.clone());
        }
        let indexer_id = request
            .indexer_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::download_submit_unavailable(
                    "this release must be resolved by its indexer before submission, but the grab names no indexer",
                )
            })?;
        let config = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(indexer_id)
            .await?
            .ok_or_else(|| {
                AppError::download_submit_unavailable(format!(
                    "indexer {indexer_id} is no longer configured, so this release cannot be resolved"
                ))
            })?;
        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| {
                AppError::download_submit_unavailable(
                    "indexer plugins are unavailable, so this release cannot be resolved",
                )
            })?;
        let proxy = self.assigned_indexer_proxy(&config).await?;
        let client = provider
            .client_for_provider_with_accounting(
                &config,
                proxy.as_ref(),
                crate::IndexerAccountingContext::for_config(&config).as_ref(),
            )
            .ok_or_else(|| {
                AppError::download_submit_unavailable(format!(
                    "no indexer client is available for {}, so this release cannot be resolved",
                    config.name
                ))
            })?;
        match client.resolve_http_release(listed).await? {
            Some(resolved) => {
                verify_resolved_http_release(listed, &resolved)?;
                refuse_unavailable_http_release(&resolved)?;
                Ok(resolved)
            }
            None => Ok(listed.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::profile::QualityProfileCriteria;
    use scryer_plugin_sdk::{HttpRangeSupport, PluginHttpResolve, PluginHttpResource};

    fn release() -> PluginHttpRelease {
        PluginHttpRelease {
            provider: "mediathek".into(),
            item_id: "item-1".into(),
            page_url: "https://provider.invalid/watch/item-1".into(),
            duration_seconds: Some(2700),
            resolve: Some(PluginHttpResolve {
                action: "resolve".into(),
                ttl_seconds: Some(600),
            }),
            ..Default::default()
        }
    }

    fn resource(transport: HttpTransport, role: HttpResourceRole) -> PluginHttpResource {
        PluginHttpResource {
            transport,
            role,
            url: "https://cdn.invalid/stream".into(),
            expires_at: None,
            headers: Vec::new(),
            auth_ref: None,
            size_bytes: None,
            size_estimate: None,
            validators: Default::default(),
            container: None,
            video: None,
            audio: None,
            subtitle: None,
            separate_tracks: false,
            range_support: HttpRangeSupport::Unknown,
        }
    }

    fn profile(tiers: &[&str], codecs: Vec<VideoCodec>, audio: &[&str]) -> QualityProfile {
        QualityProfile {
            id: "profile".into(),
            name: "Profile".into(),
            criteria: QualityProfileCriteria {
                quality_tiers: tiers.iter().map(|tier| tier.to_string()).collect(),
                video_codec_allowlist: codecs,
                required_audio_languages: audio.iter().map(|code| code.to_string()).collect(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn resolved_release_must_keep_item_id_and_page_url() {
        let listed = release();
        let same = PluginHttpRelease {
            duration_seconds: Some(2701),
            resources: vec![resource(HttpTransport::DirectFile, HttpResourceRole::Muxed)],
            ..listed.clone()
        };
        verify_resolved_http_release(&listed, &same).expect("same item resolves");

        let other_item = PluginHttpRelease {
            item_id: "item-2".into(),
            ..listed.clone()
        };
        let error = verify_resolved_http_release(&listed, &other_item).unwrap_err();
        assert!(matches!(error, AppError::Validation(_)), "{error:?}");

        let other_page = PluginHttpRelease {
            page_url: "https://provider.invalid/watch/item-1?alt=1".into(),
            ..listed.clone()
        };
        let error = verify_resolved_http_release(&listed, &other_page).unwrap_err();
        assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    }

    #[test]
    fn http_source_without_its_listed_release_is_refused() {
        let error =
            require_http_release_for_http_source(Some(DownloadSourceKind::HttpResource), None)
                .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)), "{error:?}");

        let listed = release();
        require_http_release_for_http_source(Some(DownloadSourceKind::HttpResource), Some(&listed))
            .expect("a listed release is submitted");
        for kind in [
            None,
            Some(DownloadSourceKind::NzbUrl),
            Some(DownloadSourceKind::TorrentFile),
            Some(DownloadSourceKind::MagnetUri),
        ] {
            require_http_release_for_http_source(kind, None)
                .expect("other families never need an HTTP release");
        }
    }

    #[test]
    fn unavailable_releases_are_refused_with_a_reason_per_availability() {
        for (availability, reason) in [
            (HttpAvailability::Live, "live stream"),
            (HttpAvailability::Upcoming, "not yet available"),
            (HttpAvailability::Removed, "removed by the provider"),
        ] {
            let listed = PluginHttpRelease {
                availability,
                ..release()
            };
            let error = refuse_unavailable_http_release(&listed).unwrap_err();
            match error {
                AppError::Validation(message) => {
                    assert!(message.contains(reason), "{availability:?}: {message}")
                }
                other => panic!("{availability:?}: unexpected error {other:?}"),
            }
        }
    }

    #[test]
    fn downloadable_or_retryable_releases_are_submitted() {
        for availability in [
            HttpAvailability::Available,
            HttpAvailability::Unknown,
            HttpAvailability::GeoRestricted,
            HttpAvailability::Expired,
            HttpAvailability::Drm,
            HttpAvailability::AuthRequired,
        ] {
            let listed = PluginHttpRelease {
                availability,
                ..release()
            };
            refuse_unavailable_http_release(&listed)
                .unwrap_or_else(|error| panic!("{availability:?} refused: {error:?}"));
        }
    }

    #[test]
    fn selection_reads_profile_ceiling_codecs_and_languages() {
        let listed = PluginHttpRelease {
            resources: vec![
                resource(HttpTransport::Hls, HttpResourceRole::Muxed),
                resource(HttpTransport::DirectFile, HttpResourceRole::Thumbnail),
                resource(HttpTransport::DirectFile, HttpResourceRole::Muxed),
                resource(HttpTransport::DirectFile, HttpResourceRole::Subtitle),
            ],
            ..release()
        };
        let profile = profile(
            &["720p", "2160p", "1080p"],
            vec![VideoCodec::Av1, VideoCodec::H265, VideoCodec::H264],
            &["deu", "ENG", "deu"],
        );
        let subtitles = HttpSubtitlePreferences {
            enabled: true,
            languages: vec!["eng".into(), "deu".into()],
        };
        let selected = build_http_selection(&listed, &profile, &subtitles);
        assert_eq!(
            selected.transport_preference,
            vec![HttpTransport::DirectFile, HttpTransport::Hls]
        );
        assert_eq!(selected.resources, vec![0, 2, 3]);
        assert_eq!(selected.max_height, Some(2160));
        assert_eq!(selected.video_codec_preference, vec!["av1", "h265", "h264"]);
        assert!(selected.container_preference.is_empty());
        assert_eq!(selected.audio_languages, vec!["deu", "eng"]);
        assert_eq!(selected.subtitle_languages, vec!["eng", "deu"]);
        assert_eq!(selected.subtitle_mode, HttpSubtitleMode::SidecarPreferred);
    }

    #[test]
    fn provider_page_release_selects_page_and_no_resources() {
        let profile = profile(&["1080p"], Vec::new(), &[]);
        let subtitles = HttpSubtitlePreferences {
            enabled: false,
            languages: vec!["eng".into()],
        };
        let selected = build_http_selection(&release(), &profile, &subtitles);
        assert_eq!(
            selected.transport_preference,
            vec![HttpTransport::ProviderPage]
        );
        assert!(selected.resources.is_empty());
        assert_eq!(selected.max_height, Some(1080));
        assert!(selected.video_codec_preference.is_empty());
        assert!(selected.audio_languages.is_empty());
        assert!(selected.subtitle_languages.is_empty());
        assert_eq!(selected.subtitle_mode, HttpSubtitleMode::None);
    }

    #[test]
    fn policy_names_a_sanitised_unique_output_and_carries_expectations() {
        let download_id = scryer_domain::download_identity::DownloadId::new();
        let policy = build_http_policy(
            &release(),
            "  Show: Episode / One (2026) [WEB]  ",
            &download_id,
            Some(123_456),
        );
        let expected_basename = format!("Show_Episode_One_2026_WEB-{download_id}");
        assert_eq!(policy.output_basename, expected_basename);
        assert_eq!(policy.expected_duration_seconds, Some(2700));
        assert_eq!(policy.expected_size_bytes, Some(123_456));
        assert_eq!(policy.verify, PluginHttpVerify::default());

        let other = build_http_policy(&release(), "", &download_id, None);
        assert_eq!(other.output_basename, format!("download-{download_id}"));
        assert_eq!(other.expected_size_bytes, None);

        let long_title = "x".repeat(400);
        let long = http_output_basename(&long_title, &download_id);
        assert!(long.len() <= OUTPUT_BASENAME_TITLE_LIMIT + 1 + download_id.to_string().len());
    }
}
