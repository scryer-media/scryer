use crate::acquisition_coverage::{self, ReleaseCoverage};
use scryer_domain::{Episode, MediaFacet, Title};

pub(crate) const COVERAGE_RUNTIME_MISMATCH_CODE: &str = "coverage_runtime_mismatch";

const MIN_VALIDATED_RUNTIME_SECONDS: i32 = 60;
const MIN_REAL_RUNTIME_COVERAGE_PERCENT: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CoverageValidationCoverage {
    EpisodeSet { episode_count: usize },
    Collection { episode_count: usize },
}

impl CoverageValidationCoverage {
    fn description(&self) -> &'static str {
        match self {
            Self::EpisodeSet { .. } => "episode range",
            Self::Collection { .. } => "season pack",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CoverageValidationIssue {
    pub code: &'static str,
    pub message: String,
    pub expected_runtime_minutes: i32,
    pub actual_runtime_minutes: i32,
    pub covered_episode_count: usize,
    pub real_runtime_coverage_count: usize,
    pub parsed_coverage: CoverageValidationCoverage,
}

struct CoverageRuntimeSnapshot {
    coverage: CoverageValidationCoverage,
    expected_runtime_minutes: i32,
    covered_episode_count: usize,
    real_runtime_coverage_count: usize,
    threshold_percent: i32,
}

struct RuntimeCoverageEstimate {
    average_runtime_minutes: i32,
    real_runtime_coverage_count: usize,
}

pub(crate) fn validate_broad_episode_coverage(
    title: &Title,
    parsed: &crate::ParsedReleaseMetadata,
    target_episodes: &[Episode],
    accepted: &crate::post_download_gate::ImportedFileAcceptance,
) -> Result<(), CoverageValidationIssue> {
    if title.facet == MediaFacet::Anime
        || accepted.scan_error.is_some()
        || target_episodes.is_empty()
    {
        return Ok(());
    }

    let Some(analysis) = accepted.analysis.as_ref() else {
        return Ok(());
    };
    let Some(actual_runtime_seconds) = analysis.duration_seconds else {
        return Ok(());
    };
    if actual_runtime_seconds < MIN_VALIDATED_RUNTIME_SECONDS {
        return Ok(());
    }

    let Some(parsed_episode) = parsed.episode.as_ref() else {
        return Ok(());
    };

    let coverage = acquisition_coverage::resolve_release_coverage(
        parsed,
        target_episodes,
        &[],
        target_episodes.first(),
    );
    let Some(snapshot) = CoverageRuntimeSnapshot::from_inputs(
        parsed_episode.release_type,
        &coverage,
        parsed,
        target_episodes,
    ) else {
        return Ok(());
    };

    let expected_runtime_seconds = i64::from(snapshot.expected_runtime_minutes) * 60;
    if expected_runtime_seconds <= 0 {
        return Ok(());
    }

    let actual_runtime_seconds = i64::from(actual_runtime_seconds);
    if actual_runtime_seconds * 100
        >= expected_runtime_seconds * i64::from(snapshot.threshold_percent)
    {
        return Ok(());
    }

    let actual_runtime_minutes = i32::try_from(actual_runtime_seconds / 60).unwrap_or(i32::MAX);

    Err(CoverageValidationIssue {
        code: COVERAGE_RUNTIME_MISMATCH_CODE,
        message: format!(
            "claimed {} across {} episode(s) is not plausible: expected about {}m from catalog runtime but probed file is {}m",
            snapshot.coverage.description(),
            snapshot.covered_episode_count,
            snapshot.expected_runtime_minutes,
            actual_runtime_minutes,
        ),
        expected_runtime_minutes: snapshot.expected_runtime_minutes,
        actual_runtime_minutes,
        covered_episode_count: snapshot.covered_episode_count,
        real_runtime_coverage_count: snapshot.real_runtime_coverage_count,
        parsed_coverage: snapshot.coverage,
    })
}

impl CoverageRuntimeSnapshot {
    fn from_inputs(
        release_type: crate::ParsedEpisodeReleaseType,
        coverage: &ReleaseCoverage,
        parsed: &crate::ParsedReleaseMetadata,
        target_episodes: &[Episode],
    ) -> Option<Self> {
        match (release_type, coverage) {
            (
                crate::ParsedEpisodeReleaseType::RangePack,
                ReleaseCoverage::EpisodeSet(episode_ids),
            ) if episode_ids.len() >= 3 => {
                let runtime_estimate =
                    runtime_coverage_for_episode_ids(target_episodes, episode_ids)?;
                let expected_runtime_minutes = acquisition_coverage::coverage_runtime_minutes(
                    coverage,
                    parsed,
                    target_episodes,
                    Some(runtime_estimate.average_runtime_minutes),
                )?;
                Some(Self {
                    coverage: CoverageValidationCoverage::EpisodeSet {
                        episode_count: episode_ids.len(),
                    },
                    expected_runtime_minutes,
                    covered_episode_count: episode_ids.len(),
                    real_runtime_coverage_count: runtime_estimate.real_runtime_coverage_count,
                    threshold_percent: 45,
                })
            }
            (
                crate::ParsedEpisodeReleaseType::SeasonPack,
                ReleaseCoverage::Collection(collection_id),
            ) => collection_snapshot(collection_id, coverage, parsed, target_episodes, 25),
            (
                crate::ParsedEpisodeReleaseType::SeasonPack,
                ReleaseCoverage::EpisodeSet(episode_ids),
            ) if episode_ids.len() >= 3 => {
                let runtime_estimate =
                    runtime_coverage_for_episode_ids(target_episodes, episode_ids)?;
                let expected_runtime_minutes = acquisition_coverage::coverage_runtime_minutes(
                    coverage,
                    parsed,
                    target_episodes,
                    Some(runtime_estimate.average_runtime_minutes),
                )?;
                Some(Self {
                    coverage: CoverageValidationCoverage::EpisodeSet {
                        episode_count: episode_ids.len(),
                    },
                    expected_runtime_minutes,
                    covered_episode_count: episode_ids.len(),
                    real_runtime_coverage_count: runtime_estimate.real_runtime_coverage_count,
                    threshold_percent: 25,
                })
            }
            _ => None,
        }
    }
}

fn collection_snapshot(
    collection_id: &str,
    coverage: &ReleaseCoverage,
    parsed: &crate::ParsedReleaseMetadata,
    target_episodes: &[Episode],
    threshold_percent: i32,
) -> Option<CoverageRuntimeSnapshot> {
    let covered_episodes = target_episodes
        .iter()
        .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
        .collect::<Vec<_>>();
    if covered_episodes.is_empty() {
        return None;
    }

    let covered_episode_count = covered_episodes.len();
    let runtime_estimate = runtime_coverage_from_durations(
        covered_episodes
            .iter()
            .map(|episode| episode.duration_seconds),
        covered_episode_count,
    )?;
    let expected_runtime_minutes = acquisition_coverage::coverage_runtime_minutes(
        coverage,
        parsed,
        target_episodes,
        Some(runtime_estimate.average_runtime_minutes),
    )?;

    (expected_runtime_minutes > 0).then_some(CoverageRuntimeSnapshot {
        coverage: CoverageValidationCoverage::Collection {
            episode_count: covered_episode_count,
        },
        expected_runtime_minutes,
        covered_episode_count,
        real_runtime_coverage_count: runtime_estimate.real_runtime_coverage_count,
        threshold_percent,
    })
}

fn runtime_coverage_for_episode_ids(
    episodes: &[Episode],
    episode_ids: &[String],
) -> Option<RuntimeCoverageEstimate> {
    runtime_coverage_from_durations(
        episode_ids.iter().map(|episode_id| {
            episodes
                .iter()
                .find(|episode| episode.id == *episode_id)
                .and_then(|episode| episode.duration_seconds)
        }),
        episode_ids.len(),
    )
}

fn runtime_coverage_from_durations<I>(
    durations_seconds: I,
    covered_episode_count: usize,
) -> Option<RuntimeCoverageEstimate>
where
    I: IntoIterator<Item = Option<i64>>,
{
    let real_runtime_minutes = durations_seconds
        .into_iter()
        .filter_map(|duration_seconds| {
            duration_seconds
                .and_then(|seconds| i32::try_from(seconds / 60).ok())
                .filter(|minutes| *minutes > 0)
        })
        .collect::<Vec<_>>();
    let real_runtime_coverage_count = real_runtime_minutes.len();
    if real_runtime_coverage_count == 0
        || real_runtime_coverage_count * 100
            < covered_episode_count * MIN_REAL_RUNTIME_COVERAGE_PERCENT
    {
        return None;
    }

    let average_runtime_minutes = real_runtime_minutes.iter().sum::<i32>()
        / i32::try_from(real_runtime_coverage_count).ok()?;
    (average_runtime_minutes > 0).then_some(RuntimeCoverageEstimate {
        average_runtime_minutes,
        real_runtime_coverage_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_domain::{EpisodeType, Title};

    fn title(facet: MediaFacet) -> Title {
        Title {
            id: "title-1".to_string(),
            name: "Coverage Show".to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            facet,
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: Some(24),
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn episode(
        id: &str,
        collection_id: Option<&str>,
        number: u32,
        duration_seconds: Option<i64>,
    ) -> Episode {
        Episode {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            collection_id: collection_id.map(str::to_string),
            episode_type: EpisodeType::Standard,
            episode_number: Some(number.to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some(format!("S01E{number:02}")),
            title: Some(format!("Episode {number}")),
            air_date: None,
            duration_seconds,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn parsed_season_pack(partial: bool) -> crate::ParsedReleaseMetadata {
        let mut parsed = crate::ParsedReleaseMetadata::empty("Coverage.Show.S01.Complete", "test");
        parsed.episode = Some(crate::ParsedEpisodeMetadata {
            season: Some(1),
            full_season: true,
            is_partial_season: partial,
            release_type: crate::ParsedEpisodeReleaseType::SeasonPack,
            ..Default::default()
        });
        parsed
    }

    fn parsed_episode_range(
        episode_numbers: &[u32],
        release_type: crate::ParsedEpisodeReleaseType,
    ) -> crate::ParsedReleaseMetadata {
        let mut parsed = crate::ParsedReleaseMetadata::empty("Coverage.Show.S01E01-E03", "test");
        parsed.episode = Some(crate::ParsedEpisodeMetadata {
            season: Some(1),
            episode_numbers: episode_numbers.to_vec(),
            release_type,
            ..Default::default()
        });
        parsed
    }

    fn acceptance(
        duration_seconds: Option<i32>,
        scan_error: Option<&str>,
    ) -> crate::post_download_gate::ImportedFileAcceptance {
        crate::post_download_gate::ImportedFileAcceptance {
            analysis: duration_seconds.map(|duration_seconds| crate::MediaFileAnalysis {
                details: Default::default(),
                video_codec: None,
                video_width: None,
                video_height: None,
                video_bitrate_kbps: None,
                video_bit_depth: None,
                video_hdr_format: None,
                dovi_profile: None,
                dovi_bl_compat_id: None,
                video_frame_rate: None,
                video_profile: None,
                audio_codec: None,
                audio_profile: None,
                audio_channels: None,
                audio_bitrate_kbps: None,
                audio_languages: Vec::new(),
                audio_streams: Vec::new(),
                subtitle_languages: Vec::new(),
                subtitle_codecs: Vec::new(),
                subtitle_streams: Vec::new(),
                has_multiaudio: false,
                duration_seconds: Some(duration_seconds),
                num_chapters: None,
                container_format: None,
            }),
            scan_error: scan_error.map(str::to_string),
            rule_file_doc: None,
            audio_language_warning: None,
        }
    }

    #[test]
    fn coverage_runtime_season_pack_single_episode_runtime_is_rejected() {
        let parsed = parsed_season_pack(false);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
            episode("ep-4", Some("season-1"), 4, Some(1_440)),
            episode("ep-5", Some("season-1"), 5, Some(1_440)),
        ];

        let issue = validate_broad_episode_coverage(
            &title(MediaFacet::Series),
            &parsed,
            &episodes,
            &acceptance(Some(1_440), None),
        )
        .expect_err("season pack should be rejected");

        assert_eq!(issue.code, COVERAGE_RUNTIME_MISMATCH_CODE);
        assert_eq!(
            issue.parsed_coverage,
            CoverageValidationCoverage::Collection { episode_count: 5 }
        );
    }

    #[test]
    fn coverage_runtime_large_range_pack_single_episode_runtime_is_rejected() {
        let parsed = parsed_episode_range(&[1, 2, 3], crate::ParsedEpisodeReleaseType::RangePack);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
        ];

        let issue = validate_broad_episode_coverage(
            &title(MediaFacet::Series),
            &parsed,
            &episodes,
            &acceptance(Some(1_440), None),
        )
        .expect_err("range pack should be rejected");

        assert_eq!(issue.code, COVERAGE_RUNTIME_MISMATCH_CODE);
        assert_eq!(
            issue.parsed_coverage,
            CoverageValidationCoverage::EpisodeSet { episode_count: 3 }
        );
    }

    #[test]
    fn coverage_runtime_two_episode_range_pack_passes_through() {
        let parsed = parsed_episode_range(&[1, 2], crate::ParsedEpisodeReleaseType::RangePack);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
        ];

        assert!(
            validate_broad_episode_coverage(
                &title(MediaFacet::Series),
                &parsed,
                &episodes,
                &acceptance(Some(1_440), None),
            )
            .is_ok()
        );
    }

    #[test]
    fn coverage_runtime_missing_episode_duration_passes_through() {
        let parsed = parsed_episode_range(&[1, 2, 3], crate::ParsedEpisodeReleaseType::RangePack);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, None),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
        ];

        assert!(
            validate_broad_episode_coverage(
                &title(MediaFacet::Series),
                &parsed,
                &episodes,
                &acceptance(Some(1_440), None),
            )
            .is_ok()
        );
    }

    #[test]
    fn coverage_runtime_large_range_pack_with_one_missing_duration_still_rejects() {
        let parsed =
            parsed_episode_range(&[1, 2, 3, 4, 5], crate::ParsedEpisodeReleaseType::RangePack);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
            episode("ep-4", Some("season-1"), 4, Some(1_440)),
            episode("ep-5", Some("season-1"), 5, None),
        ];

        let issue = validate_broad_episode_coverage(
            &title(MediaFacet::Series),
            &parsed,
            &episodes,
            &acceptance(Some(1_440), None),
        )
        .expect_err("range pack should still be rejected when most runtimes are present");

        assert_eq!(issue.code, COVERAGE_RUNTIME_MISMATCH_CODE);
        assert_eq!(issue.covered_episode_count, 5);
        assert_eq!(issue.real_runtime_coverage_count, 4);
    }

    #[test]
    fn coverage_runtime_season_pack_with_one_missing_duration_still_rejects() {
        let parsed = parsed_season_pack(false);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
            episode("ep-4", Some("season-1"), 4, Some(1_440)),
            episode("ep-5", Some("season-1"), 5, None),
        ];

        let issue = validate_broad_episode_coverage(
            &title(MediaFacet::Series),
            &parsed,
            &episodes,
            &acceptance(Some(1_440), None),
        )
        .expect_err("season pack should still be rejected when most runtimes are present");

        assert_eq!(issue.code, COVERAGE_RUNTIME_MISMATCH_CODE);
        assert_eq!(issue.covered_episode_count, 5);
        assert_eq!(issue.real_runtime_coverage_count, 4);
    }

    #[test]
    fn coverage_runtime_anime_passes_through() {
        let parsed = parsed_season_pack(false);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
            episode("ep-4", Some("season-1"), 4, Some(1_440)),
            episode("ep-5", Some("season-1"), 5, Some(1_440)),
        ];

        assert!(
            validate_broad_episode_coverage(
                &title(MediaFacet::Anime),
                &parsed,
                &episodes,
                &acceptance(Some(1_440), None),
            )
            .is_ok()
        );
    }

    #[test]
    fn coverage_runtime_multi_episode_release_passes_through() {
        let parsed =
            parsed_episode_range(&[1, 2, 3], crate::ParsedEpisodeReleaseType::MultiEpisode);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
        ];

        assert!(
            validate_broad_episode_coverage(
                &title(MediaFacet::Series),
                &parsed,
                &episodes,
                &acceptance(Some(1_440), None),
            )
            .is_ok()
        );
    }

    #[test]
    fn coverage_runtime_partial_season_uses_weaker_expected_runtime() {
        let parsed = parsed_season_pack(true);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
            episode("ep-4", Some("season-1"), 4, Some(1_440)),
            episode("ep-5", Some("season-1"), 5, Some(1_440)),
            episode("ep-6", Some("season-1"), 6, Some(1_440)),
        ];

        assert!(
            validate_broad_episode_coverage(
                &title(MediaFacet::Series),
                &parsed,
                &episodes,
                &acceptance(Some(2_100), None),
            )
            .is_ok()
        );
    }

    #[test]
    fn coverage_runtime_scan_error_passes_through() {
        let parsed = parsed_season_pack(false);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
            episode("ep-4", Some("season-1"), 4, Some(1_440)),
            episode("ep-5", Some("season-1"), 5, Some(1_440)),
        ];

        assert!(
            validate_broad_episode_coverage(
                &title(MediaFacet::Series),
                &parsed,
                &episodes,
                &acceptance(None, Some("probe failed")),
            )
            .is_ok()
        );
    }

    #[test]
    fn coverage_runtime_sub_minute_probe_passes_through() {
        let parsed = parsed_season_pack(false);
        let episodes = vec![
            episode("ep-1", Some("season-1"), 1, Some(1_440)),
            episode("ep-2", Some("season-1"), 2, Some(1_440)),
            episode("ep-3", Some("season-1"), 3, Some(1_440)),
            episode("ep-4", Some("season-1"), 4, Some(1_440)),
            episode("ep-5", Some("season-1"), 5, Some(1_440)),
        ];

        assert!(
            validate_broad_episode_coverage(
                &title(MediaFacet::Series),
                &parsed,
                &episodes,
                &acceptance(Some(30), None),
            )
            .is_ok()
        );
    }
}
