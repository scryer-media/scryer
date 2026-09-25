use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{AppError, AppResult};

/// Query parameters for searching subtitles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleMediaKind {
    Movie,
    Episode,
}

#[derive(Debug, Clone)]
pub struct SubtitleQuery {
    /// Whether this search is for a movie or an episode.
    pub media_kind: SubtitleMediaKind,
    /// Content facet (movie, series, anime) owned by Scryer for provider routing.
    pub facet: Option<String>,
    /// Provider-neutral file hash hint computed from the media file.
    pub file_hash: Option<String>,
    /// IMDb ID for the movie itself.
    pub imdb_id: Option<String>,
    /// IMDb ID for the parent series.
    pub series_imdb_id: Option<String>,
    /// Primary title name for feature lookups and text fallback.
    pub title: String,
    /// Alternate title names (aliases) for feature lookups.
    pub title_aliases: Vec<String>,
    /// Refined title candidates derived from release metadata.
    pub title_candidates: Vec<String>,
    /// Release year.
    pub year: Option<i32>,
    /// Season number (series only).
    pub season: Option<i32>,
    /// Episode number (series only).
    pub episode: Option<i32>,
    /// Absolute episode number when available.
    pub absolute_episode: Option<i32>,
    /// The anime community entry (usually one cour) the episode belongs to,
    /// with the episode's number inside it.
    pub community_entry: Option<SubtitleCommunityEntry>,
    /// Provider-specific external identifiers grouped by normalized source key.
    pub external_ids: BTreeMap<String, Vec<String>>,
    /// Internal subtitle language codes to search for.
    pub languages: Vec<String>,
    /// Release group from the filename.
    pub release_group: Option<String>,
    /// Source (BluRay, WEB-DL, etc.).
    pub source: Option<String>,
    /// Video codec.
    pub video_codec: Option<String>,
    /// Audio codec.
    pub audio_codec: Option<String>,
    /// Resolution (e.g., "1080p").
    pub resolution: Option<String>,
    /// Whether hearing-impaired subtitles are preferred.
    pub hearing_impaired: Option<bool>,
    /// Whether to include AI-translated results.
    pub include_ai_translated: bool,
    /// Whether to include machine-translated results.
    pub include_machine_translated: bool,
}

/// One AniList/AniDB/MAL entry an episode belongs to, numbered on its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubtitleCommunityEntry {
    /// 1-based community season, in air order.
    pub season: i32,
    /// The episode's number within this entry.
    pub episode: i32,
    pub anilist_id: Option<i64>,
    pub anidb_id: Option<i64>,
    pub mal_id: Option<i64>,
    /// The entry's titles, most canonical first.
    pub titles: Vec<String>,
}

/// A single subtitle search result from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleMatch {
    /// Provider name.
    pub provider: String,
    /// Provider-specific file identifier for downloading/blocklisting.
    pub provider_file_id: String,
    /// Stable internal subtitle language code.
    pub language: String,
    /// Release info / filename from the provider.
    pub release_info: Option<String>,
    /// Computed match score.
    pub score: i32,
    /// Computed match score normalized to a 0-100 percentage for display.
    pub score_percent: i32,
    /// Whether this subtitle is hearing-impaired.
    pub hearing_impaired: bool,
    /// Whether this subtitle is forced (foreign parts only).
    pub forced: bool,
    /// Whether this was flagged as AI-translated.
    pub ai_translated: bool,
    /// Whether this was flagged as machine-translated.
    pub machine_translated: bool,
    /// Uploader name.
    pub uploader: Option<String>,
    /// Download count on the provider.
    pub download_count: Option<i64>,
    /// Whether the file hash matched.
    pub hash_matched: bool,
}

/// Downloaded subtitle file content.
#[derive(Debug)]
pub struct SubtitleFile {
    /// Downloaded artifact or normalized subtitle content bytes.
    pub content: Vec<u8>,
    /// Legacy/provider file extension hint. After host-side normalization this
    /// is the final subtitle extension (e.g., "srt", "ass").
    pub format: String,
    /// Provider artifact filename when known. This may describe a compressed
    /// artifact such as `release.ass.xz` before normalization.
    pub filename: Option<String>,
    /// Provider artifact content type when known.
    pub content_type: Option<String>,
}

#[async_trait]
pub trait SubtitleProvider: Send + Sync {
    /// Search for subtitles matching the query.
    async fn search(&self, query: &SubtitleQuery) -> AppResult<Vec<SubtitleMatch>>;

    /// Download a specific subtitle by provider file ID.
    async fn download(&self, provider_file_id: &str) -> AppResult<SubtitleFile>;

    /// Provider name.
    fn name(&self) -> &str;
}

/// Compute the subtitle file hash hint used by subtitle plugins.
///
/// The host owns this because plugins do not get direct access to the media file.
pub fn compute_subtitle_file_hash(path: &std::path::Path) -> AppResult<String> {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    const CHUNK_SIZE: usize = 65_536;

    let mut file = File::open(path)
        .map_err(|e| AppError::Repository(format!("cannot open file for hashing: {e}")))?;
    let file_size = file
        .metadata()
        .map_err(|e| AppError::Repository(format!("cannot stat file: {e}")))?
        .len();

    if file_size < CHUNK_SIZE as u64 * 2 {
        return Err(AppError::Validation(
            "file too small for subtitle file hash".into(),
        ));
    }

    let mut hash: u64 = file_size;
    let mut buf = [0u8; 8];

    for _ in 0..(CHUNK_SIZE / 8) {
        file.read_exact(&mut buf)
            .map_err(|e| AppError::Repository(format!("hash read error: {e}")))?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    file.seek(SeekFrom::End(-(CHUNK_SIZE as i64)))
        .map_err(|e| AppError::Repository(format!("hash seek error: {e}")))?;
    for _ in 0..(CHUNK_SIZE / 8) {
        file.read_exact(&mut buf)
            .map_err(|e| AppError::Repository(format!("hash read error: {e}")))?;
        hash = hash.wrapping_add(u64::from_le_bytes(buf));
    }

    Ok(format!("{hash:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn subtitle_file_hash_known_shape() {
        let hash_str = format!("{:016x}", 0u64);
        assert_eq!(hash_str.len(), 16);
    }

    #[test]
    fn hash_rejects_file_smaller_than_128kb() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 65_535]).unwrap();
        tmp.flush().unwrap();
        let result = compute_subtitle_file_hash(tmp.path());
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("too small"));
    }

    #[test]
    fn hash_rejects_file_exactly_128kb_minus_one() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 131_071]).unwrap();
        tmp.flush().unwrap();
        assert!(compute_subtitle_file_hash(tmp.path()).is_err());
    }

    #[test]
    fn hash_accepts_file_exactly_128kb() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 131_072]).unwrap();
        tmp.flush().unwrap();
        assert!(compute_subtitle_file_hash(tmp.path()).is_ok());
    }

    #[test]
    fn hash_output_is_16_hex_chars() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; 131_072]).unwrap();
        tmp.flush().unwrap();
        let hash = compute_subtitle_file_hash(tmp.path()).unwrap();
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_of_all_zeros_equals_file_size() {
        let size: u64 = 131_072;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&vec![0u8; size as usize]).unwrap();
        tmp.flush().unwrap();
        let hash = compute_subtitle_file_hash(tmp.path()).unwrap();
        assert_eq!(hash, format!("{size:016x}"));
    }

    #[test]
    fn hash_changes_with_different_content() {
        let size = 131_072usize;

        let mut tmp1 = tempfile::NamedTempFile::new().unwrap();
        tmp1.write_all(&vec![0u8; size]).unwrap();
        tmp1.flush().unwrap();
        let hash1 = compute_subtitle_file_hash(tmp1.path()).unwrap();

        let mut tmp2 = tempfile::NamedTempFile::new().unwrap();
        tmp2.write_all(&vec![1u8; size]).unwrap();
        tmp2.flush().unwrap();
        let hash2 = compute_subtitle_file_hash(tmp2.path()).unwrap();

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn hash_with_large_file_reads_first_and_last_64kb() {
        let chunk = 65_536usize;
        let mut data = Vec::with_capacity(chunk * 4);
        data.extend(vec![1u8; chunk]);
        data.extend(vec![0u8; chunk * 2]);
        data.extend(vec![2u8; chunk]);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&data).unwrap();
        tmp.flush().unwrap();

        let hash = compute_subtitle_file_hash(tmp.path()).unwrap();
        assert_eq!(hash.len(), 16);

        let mut data2 = data.clone();
        for byte in &mut data2[chunk..chunk * 3] {
            *byte = 0xFF;
        }
        let mut tmp2 = tempfile::NamedTempFile::new().unwrap();
        tmp2.write_all(&data2).unwrap();
        tmp2.flush().unwrap();

        let hash2 = compute_subtitle_file_hash(tmp2.path()).unwrap();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn subtitle_query_fields_set_correctly() {
        let q = SubtitleQuery {
            media_kind: SubtitleMediaKind::Episode,
            facet: Some("series".into()),
            file_hash: Some("abc123".into()),
            imdb_id: None,
            series_imdb_id: Some("tt1234567".into()),
            title: "Cinder Line".into(),
            title_aliases: vec!["Faultline".into()],
            title_candidates: vec!["Cinder Line".into()],
            year: Some(2008),
            season: Some(1),
            episode: Some(3),
            absolute_episode: None,
            community_entry: None,
            external_ids: Default::default(),
            languages: vec!["eng".into(), "spa".into()],
            release_group: Some("NTb".into()),
            source: Some("WEB-DL".into()),
            video_codec: Some("x264".into()),
            audio_codec: Some("DDP".into()),
            resolution: Some("1080p".into()),
            hearing_impaired: Some(false),
            include_ai_translated: false,
            include_machine_translated: false,
        };

        assert_eq!(q.media_kind, SubtitleMediaKind::Episode);
        assert_eq!(q.series_imdb_id.as_deref(), Some("tt1234567"));
        assert_eq!(q.title_aliases, vec!["Faultline"]);
        assert_eq!(q.hearing_impaired, Some(false));
    }

    #[test]
    fn subtitle_query_optional_fields_default_none() {
        let q = SubtitleQuery {
            media_kind: SubtitleMediaKind::Movie,
            facet: Some("movie".into()),
            file_hash: None,
            imdb_id: None,
            series_imdb_id: None,
            title: "Test".into(),
            title_aliases: vec![],
            title_candidates: vec![],
            year: None,
            season: None,
            episode: None,
            absolute_episode: None,
            community_entry: None,
            external_ids: Default::default(),
            languages: vec![],
            release_group: None,
            source: None,
            video_codec: None,
            audio_codec: None,
            resolution: None,
            hearing_impaired: None,
            include_ai_translated: true,
            include_machine_translated: true,
        };

        assert!(q.imdb_id.is_none());
        assert!(q.series_imdb_id.is_none());
        assert!(q.title_aliases.is_empty());
        assert!(q.hearing_impaired.is_none());
    }

    #[test]
    fn subtitle_match_ordering_higher_score_first() {
        let mut matches = [
            SubtitleMatch {
                provider: "test-provider".into(),
                provider_file_id: "1".into(),
                language: "eng".into(),
                release_info: None,
                score: 100,
                score_percent: 28,
                hearing_impaired: false,
                forced: false,
                ai_translated: false,
                machine_translated: false,
                uploader: None,
                download_count: None,
                hash_matched: false,
            },
            SubtitleMatch {
                provider: "test-provider".into(),
                provider_file_id: "2".into(),
                language: "eng".into(),
                release_info: None,
                score: 300,
                score_percent: 83,
                hearing_impaired: false,
                forced: false,
                ai_translated: false,
                machine_translated: false,
                uploader: None,
                download_count: None,
                hash_matched: true,
            },
        ];

        matches.sort_by_key(|entry| std::cmp::Reverse(entry.score));
        assert_eq!(matches[0].provider_file_id, "2");
    }
}
