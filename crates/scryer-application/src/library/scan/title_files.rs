use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileSourceSignature {
    pub(crate) scheme: String,
    pub(crate) value: String,
}

#[derive(Clone, Debug)]
pub(crate) struct FileSourceSnapshot {
    pub(crate) size_bytes: i64,
    pub(crate) signature: Option<FileSourceSignature>,
}

#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(super) struct TitleEpisodeLookup {
    by_air_date: HashMap<String, Vec<Episode>>,
    by_collection_episode: HashMap<(String, String), Episode>,
    by_absolute_number: HashMap<String, Episode>,
    by_collection_index: HashMap<String, Vec<Episode>>,
}

#[derive(Clone, Debug)]
pub(crate) struct PlannedTitleScanFile {
    pub(crate) file: LibraryFile,
    pub(crate) parsed: crate::ParsedReleaseMetadata,
    pub(crate) target_episodes: Vec<Episode>,
    pub(crate) series_movie_link_id: Option<String>,
    pub(crate) snapshot: FileSourceSnapshot,
    pub(crate) record: PlannedTitleScanRecord,
}

#[derive(Clone, Debug)]
pub(crate) enum PlannedTitleScanRecord {
    Existing {
        file_id: String,
        should_skip_analysis: bool,
        should_refresh_source_signature: bool,
    },
    New,
}

pub(crate) fn file_source_signature_from_metadata(
    metadata: &std::fs::Metadata,
) -> Option<FileSourceSignature> {
    source_signature_from_std_metadata(metadata)
        .map(|(scheme, value)| FileSourceSignature { scheme, value })
}

pub(crate) fn file_source_snapshot_from_library_file(
    file: &LibraryFile,
) -> Option<FileSourceSnapshot> {
    let size_bytes = file.size_bytes?;
    let signature = match (
        file.source_signature_scheme.clone(),
        file.source_signature_value.clone(),
    ) {
        (Some(scheme), Some(value)) => Some(FileSourceSignature { scheme, value }),
        _ => None,
    };

    Some(FileSourceSnapshot {
        size_bytes,
        signature,
    })
}

pub(super) fn title_media_file_matches_snapshot(
    media_file: &TitleMediaFile,
    snapshot: &FileSourceSnapshot,
) -> bool {
    if media_file.scan_status != "scanned"
        || media_file.size_bytes != snapshot.size_bytes
        || !title_media_file_has_persisted_analysis(media_file)
    {
        return false;
    }

    match (
        &media_file.source_signature_scheme,
        &media_file.source_signature_value,
    ) {
        // Older rows created before source signatures existed can safely reuse
        // the previous analysis on the first post-migration scan when size and
        // scan status still match. The signature gets backfilled separately.
        (None, None) => true,
        (Some(scheme), Some(value)) => snapshot
            .signature
            .as_ref()
            .is_some_and(|signature| signature.scheme == *scheme && signature.value == *value),
        _ => false,
    }
}

fn title_media_file_has_persisted_analysis(media_file: &TitleMediaFile) -> bool {
    media_file.video_codec.is_some()
        || media_file.video_width.is_some()
        || media_file.video_height.is_some()
        || media_file.video_bitrate_kbps.is_some()
        || media_file.video_bit_depth.is_some()
        || media_file.video_hdr_format.is_some()
        || media_file.video_frame_rate.is_some()
        || media_file.video_profile.is_some()
        || media_file.audio_codec.is_some()
        || media_file.audio_channels.is_some()
        || media_file.audio_bitrate_kbps.is_some()
        || !media_file.audio_languages.is_empty()
        || !media_file.audio_streams.is_empty()
        || !media_file.subtitle_languages.is_empty()
        || !media_file.subtitle_codecs.is_empty()
        || !media_file.subtitle_streams.is_empty()
        || media_file.duration_seconds.is_some()
        || media_file.num_chapters.is_some()
        || media_file.container_format.is_some()
        || media_file.has_multiaudio
}

#[cfg(test)]
pub(super) fn build_title_episode_lookup(
    collections: &[Collection],
    episodes: &[Episode],
) -> TitleEpisodeLookup {
    let collection_indexes = collections
        .iter()
        .map(|collection| (collection.id.clone(), collection.collection_index.clone()))
        .collect::<HashMap<_, _>>();

    let mut lookup = TitleEpisodeLookup::default();
    for episode in episodes {
        if let Some(air_date) = episode.air_date.as_ref() {
            lookup
                .by_air_date
                .entry(air_date.clone())
                .or_default()
                .push(episode.clone());
        }

        if let (Some(season_number), Some(episode_number)) = (
            episode.season_number.as_ref(),
            episode.episode_number.as_ref(),
        ) {
            if let Some(collection_id) = episode.collection_id.as_ref()
                && let Some(collection_index) = collection_indexes.get(collection_id)
            {
                lookup
                    .by_collection_episode
                    .entry((collection_index.clone(), episode_number.clone()))
                    .or_insert_with(|| episode.clone());
            } else {
                lookup
                    .by_collection_episode
                    .entry((season_number.clone(), episode_number.clone()))
                    .or_insert_with(|| episode.clone());
            }
        }

        if let Some(absolute_number) = episode.absolute_number.as_ref() {
            lookup
                .by_absolute_number
                .entry(absolute_number.clone())
                .or_insert_with(|| episode.clone());
        }

        if let Some(collection_id) = episode.collection_id.as_ref()
            && let Some(collection_index) = collection_indexes.get(collection_id)
        {
            lookup
                .by_collection_index
                .entry(collection_index.clone())
                .or_default()
                .push(episode.clone());
        }
    }

    for episodes in lookup.by_air_date.values_mut() {
        episodes.sort_by_key(|episode| {
            episode
                .episode_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
    }
    for episodes in lookup.by_collection_index.values_mut() {
        episodes.sort_by_key(|episode| {
            episode
                .episode_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
    }

    lookup
}

#[cfg(test)]
pub(super) fn resolve_target_episodes_from_lookup(
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
    lookup: &TitleEpisodeLookup,
) -> Vec<Episode> {
    let mut episodes = Vec::new();
    let mut seen = HashSet::new();
    let target_season = crate::parsed_episode_lookup_season(ep_meta, season_str);

    if let Some(air_date) = ep_meta.air_date {
        let air_date_str = air_date.format("%Y-%m-%d").to_string();
        if let Some(matches) = lookup.by_air_date.get(&air_date_str) {
            if let Some(part) = ep_meta.daily_part {
                let part_index = part.saturating_sub(1) as usize;
                if let Some(episode) = matches.get(part_index)
                    && seen.insert(episode.id.clone())
                {
                    episodes.push(episode.clone());
                }
            } else {
                for episode in matches {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode.clone());
                    }
                }
            }
        }
    }

    for episode_number in &ep_meta.episode_numbers {
        let key = (target_season.clone(), episode_number.to_string());
        if let Some(episode) = lookup.by_collection_episode.get(&key)
            && seen.insert(episode.id.clone())
        {
            episodes.push(episode.clone());
        }
    }

    if episodes.is_empty()
        && ep_meta.season.is_some()
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
        && let Some(collection_episodes) = lookup.by_collection_index.get(&target_season)
    {
        for episode in collection_episodes {
            if episode.season_number.as_deref() == Some(target_season.as_str())
                && seen.insert(episode.id.clone())
            {
                episodes.push(episode.clone());
            }
        }
    }

    if episodes.is_empty() && !ep_meta.special_absolute_episode_numbers.is_empty() {
        for special_number in &ep_meta.special_absolute_episode_numbers {
            let key = ("0".to_string(), special_number.to_string());
            if let Some(episode) = lookup.by_collection_episode.get(&key)
                && seen.insert(episode.id.clone())
            {
                episodes.push(episode.clone());
            }
        }
    }

    if episodes.is_empty()
        && (ep_meta.absolute_episode.is_some() || !ep_meta.absolute_episode_numbers.is_empty())
    {
        let absolute_numbers: Vec<u32> = if !ep_meta.absolute_episode_numbers.is_empty() {
            ep_meta.absolute_episode_numbers.clone()
        } else if ep_meta.episode_numbers.is_empty() {
            vec![ep_meta.absolute_episode.unwrap_or_default()]
        } else {
            ep_meta.episode_numbers.clone()
        };

        for absolute_number in absolute_numbers {
            if let Some(episode) = lookup.by_absolute_number.get(&absolute_number.to_string())
                && seen.insert(episode.id.clone())
            {
                episodes.push(episode.clone());
            }
        }
    }

    episodes
}

const SEASON_FOLDER_TAG_PREFIX: &str = "scryer:season-folder:";

fn set_structured_title_tag(tags: &mut Vec<String>, prefix: &str, value: Option<&str>) {
    tags.retain(|tag| !tag.starts_with(prefix));
    let Some(value) = value else {
        return;
    };
    let normalized = value.trim();
    if normalized.is_empty() {
        return;
    }
    tags.push(format!("{prefix}{normalized}"));
}

pub(super) fn merge_title_scan_option_tags(
    mut tags: Vec<String>,
    use_season_folders: bool,
) -> Vec<String> {
    set_structured_title_tag(
        &mut tags,
        SEASON_FOLDER_TAG_PREFIX,
        Some(if use_season_folders {
            "enabled"
        } else {
            "disabled"
        }),
    );
    tags
}

fn normalize_layout_component(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut prev_sep = false;
    for ch in name.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_whitespace() || matches!(lower, '.' | '_' | '-') {
            if !prev_sep {
                normalized.push(' ');
                prev_sep = true;
            }
        } else {
            normalized.push(lower);
            prev_sep = false;
        }
    }
    normalized.trim().to_string()
}

pub(crate) fn recognize_season_folder_name(name: &str) -> Option<u32> {
    let normalized = normalize_layout_component(name);
    if normalized.is_empty() {
        return None;
    }

    let compact = normalized.replace(' ', "");
    if matches!(compact.as_str(), "specials" | "specialepisodes") {
        return Some(0);
    }

    for prefix in ["season", "series", "s"] {
        let Some(rest) = compact.strip_prefix(prefix) else {
            continue;
        };
        if rest.is_empty() || !rest.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        return rest.parse::<u32>().ok();
    }

    None
}

fn infer_target_season_number(target_episodes: &[Episode]) -> Option<u32> {
    let mut seasons = target_episodes
        .iter()
        .map(|episode| episode.season_number.as_deref()?.parse::<u32>().ok())
        .collect::<Option<HashSet<_>>>()?;
    if seasons.len() == 1 {
        seasons.drain().next()
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TitleScanLayoutObservation {
    Flat,
    SeasonFolder,
    Ambiguous,
}

pub(super) fn classify_title_scan_layout(
    title_dir: &Path,
    file_path: &Path,
    target_episodes: &[Episode],
) -> TitleScanLayoutObservation {
    let Ok(relative) = file_path.strip_prefix(title_dir) else {
        return TitleScanLayoutObservation::Ambiguous;
    };

    let Some(parent) = relative.parent() else {
        return TitleScanLayoutObservation::Flat;
    };

    let first_component = parent
        .components()
        .find_map(|component| component.as_os_str().to_str())
        .filter(|component| !component.is_empty());

    let Some(first_component) = first_component else {
        return TitleScanLayoutObservation::Flat;
    };

    let Some(folder_season) = recognize_season_folder_name(first_component) else {
        return TitleScanLayoutObservation::Ambiguous;
    };

    match infer_target_season_number(target_episodes) {
        Some(target_season) if target_season == folder_season => {
            TitleScanLayoutObservation::SeasonFolder
        }
        _ => TitleScanLayoutObservation::Ambiguous,
    }
}

#[derive(Default)]
pub(super) struct TitleScanLayoutSummary {
    saw_flat: bool,
    saw_season_folder: bool,
    ambiguous: bool,
}

impl TitleScanLayoutSummary {
    pub(super) fn observe(&mut self, observation: TitleScanLayoutObservation) {
        match observation {
            TitleScanLayoutObservation::Flat => self.saw_flat = true,
            TitleScanLayoutObservation::SeasonFolder => self.saw_season_folder = true,
            TitleScanLayoutObservation::Ambiguous => self.ambiguous = true,
        }
    }

    pub(super) fn inferred_use_season_folders(&self) -> Option<bool> {
        if self.ambiguous || self.saw_flat == self.saw_season_folder {
            None
        } else if self.saw_season_folder {
            Some(true)
        } else if self.saw_flat {
            Some(false)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_episode(season_number: &str) -> Episode {
        Episode {
            id: "episode-1".into(),
            title_id: "title-1".into(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".into()),
            season_number: Some(season_number.into()),
            episode_label: Some("S01E01".into()),
            title: Some("Pilot".into()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        }
    }

    fn build_test_media_file(
        size_bytes: i64,
        source_signature_scheme: Option<&str>,
        source_signature_value: Option<&str>,
        analyzed: bool,
    ) -> TitleMediaFile {
        TitleMediaFile {
            id: "file-1".into(),
            title_id: "title-1".into(),
            episode_id: Some("episode-1".into()),
            series_movie_link_ids: Vec::new(),
            role: crate::MediaFileRole::Primary,
            file_path: "/library/Show/Season 01/Show.S01E01.mkv".into(),
            size_bytes,
            source_signature_scheme: source_signature_scheme.map(str::to_string),
            source_signature_value: source_signature_value.map(str::to_string),
            quality_label: None,
            scan_status: "scanned".into(),
            created_at: String::new(),
            video_codec: analyzed
                .then(|| crate::release_parser::VideoCodec::parse("h264").expect("parse codec")),
            video_width: analyzed.then_some(1920),
            video_height: analyzed.then_some(1080),
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: analyzed.then(|| "aac".into()),
            audio_channels: analyzed.then_some(2),
            audio_bitrate_kbps: None,
            audio_languages: vec![],
            audio_streams: vec![],
            subtitle_languages: vec![],
            subtitle_codecs: vec![],
            subtitle_streams: vec![],
            has_multiaudio: false,
            duration_seconds: analyzed.then_some(1440),
            num_chapters: None,
            container_format: analyzed.then(|| "matroska".into()),
            scene_name: None,
            release_group: None,
            source_type: None,
            resolution: None,
            video_codec_parsed: None,
            audio_codec_parsed: None,
            audio_profile: None,
            audio_channels_parsed: None,
            acquisition_score: None,
            scoring_log: None,
            indexer_source: None,
            grabbed_release_title: None,
            grabbed_at: None,
            edition: None,
            original_file_path: None,
            release_hash: None,
        }
    }

    #[test]
    fn recognize_season_folder_name_accepts_common_variants() {
        assert_eq!(recognize_season_folder_name("Season 01"), Some(1));
        assert_eq!(recognize_season_folder_name("Series_1"), Some(1));
        assert_eq!(recognize_season_folder_name("S01"), Some(1));
        assert_eq!(recognize_season_folder_name("Season-00"), Some(0));
        assert_eq!(recognize_season_folder_name("Special Episodes"), Some(0));
        assert_eq!(recognize_season_folder_name("specials"), Some(0));
        assert_eq!(recognize_season_folder_name("Extras"), None);
    }

    #[test]
    fn classify_title_scan_layout_marks_conflicting_season_folders_ambiguous() {
        let title_dir = PathBuf::from("/library/Example Show");
        let file_path = title_dir.join("Series 02/Example.Show.S01E01.mkv");
        let target_episodes = vec![test_episode("1")];

        assert_eq!(
            classify_title_scan_layout(&title_dir, &file_path, &target_episodes),
            TitleScanLayoutObservation::Ambiguous
        );
    }

    #[test]
    fn file_source_signature_uses_platform_scheme() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("movie.mkv");
        std::fs::write(&path, b"video").expect("write test file");

        let metadata = std::fs::metadata(&path).expect("metadata");
        let signature = file_source_signature_from_metadata(&metadata).expect("signature");

        #[cfg(unix)]
        assert_eq!(signature.scheme, "unix_mtime_nsec_v1");
        #[cfg(windows)]
        assert_eq!(signature.scheme, "windows_last_write_100ns_v1");
        #[cfg(all(not(unix), not(windows)))]
        assert_eq!(signature.scheme, "system_time_nsec_v1");

        assert!(!signature.value.trim().is_empty());
    }

    #[test]
    fn title_media_file_matches_snapshot_backfills_missing_signatures_without_reanalysis() {
        let media_file = build_test_media_file(1234, None, None, true);
        let snapshot = FileSourceSnapshot {
            size_bytes: 1234,
            signature: Some(FileSourceSignature {
                scheme: "unix_mtime_nsec_v1".into(),
                value: "1:2".into(),
            }),
        };

        assert!(title_media_file_matches_snapshot(&media_file, &snapshot));
    }

    #[test]
    fn title_media_file_matches_snapshot_requires_persisted_analysis() {
        let media_file =
            build_test_media_file(1234, Some("unix_mtime_nsec_v1"), Some("1:2"), false);
        let snapshot = FileSourceSnapshot {
            size_bytes: 1234,
            signature: Some(FileSourceSignature {
                scheme: "unix_mtime_nsec_v1".into(),
                value: "1:2".into(),
            }),
        };

        assert!(!title_media_file_matches_snapshot(&media_file, &snapshot));
    }

    #[test]
    fn resolve_target_episodes_from_lookup_uses_collection_index_and_preserves_first_duplicate() {
        let collection = Collection {
            id: "collection-2".into(),
            title_id: "title-1".into(),
            collection_type: CollectionType::Season,
            collection_index: "2".into(),
            label: Some("Season 2".into()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".into()),
            last_episode_number: Some("10".into()),
            monitored: true,
            created_at: Utc::now(),
        };
        let first = Episode {
            id: "episode-a".into(),
            title_id: "title-1".into(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".into()),
            season_number: Some("1".into()),
            episode_label: Some("S01E01".into()),
            title: Some("First".into()),
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("101".into()),
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let second = Episode {
            id: "episode-b".into(),
            absolute_number: Some("101".into()),
            ..first.clone()
        };

        let lookup =
            build_title_episode_lookup(std::slice::from_ref(&collection), &[first.clone(), second]);
        let ep_meta = crate::ParsedEpisodeMetadata {
            season: Some(2),
            episode_numbers: vec![1],
            release_type: crate::ParsedEpisodeReleaseType::SingleEpisode,
            ..Default::default()
        };

        let episodes = resolve_target_episodes_from_lookup(&ep_meta, "2", &lookup);

        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].id, first.id);
    }

    #[test]
    fn resolve_target_episodes_from_lookup_keeps_explicit_standard_episode_season() {
        let collection = Collection {
            id: "collection-4".into(),
            title_id: "title-1".into(),
            collection_type: CollectionType::Season,
            collection_index: "4".into(),
            label: Some("Season 4".into()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("29".into()),
            last_episode_number: Some("30".into()),
            monitored: true,
            created_at: Utc::now(),
        };
        let episode = Episode {
            id: "episode-29".into(),
            title_id: "title-1".into(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("29".into()),
            season_number: Some("4".into()),
            episode_label: Some("S04E29".into()),
            title: Some("The Final Chapters Special 1".into()),
            air_date: None,
            duration_seconds: None,
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
        };

        let lookup = build_title_episode_lookup(
            std::slice::from_ref(&collection),
            std::slice::from_ref(&episode),
        );
        let ep_meta = crate::ParsedEpisodeMetadata {
            season: Some(4),
            episode_numbers: vec![29],
            special_kind: Some(crate::ParsedSpecialKind::Special),
            special_absolute_episode_numbers: vec![1],
            release_type: crate::ParsedEpisodeReleaseType::SingleEpisode,
            ..Default::default()
        };

        let episodes = resolve_target_episodes_from_lookup(&ep_meta, "4", &lookup);

        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].id, episode.id);
    }
}
