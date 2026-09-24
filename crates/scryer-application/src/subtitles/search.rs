use std::path::Path;

use super::provider::{SubtitleMatch, SubtitleProvider, SubtitleQuery, compute_subtitle_file_hash};
use crate::AppResult;

/// Orchestrates subtitle searching by enriching the provider query with a file hash
/// when one can be computed and leaving the provider to combine hash and metadata.
pub struct SubtitleSearchOrchestrator;

impl SubtitleSearchOrchestrator {
    pub fn new(_min_score: i32) -> Self {
        Self
    }

    /// Search for subtitles for a media file.
    ///
    /// Strategy:
    /// 1. Compute a host-side subtitle file hash when possible.
    /// 2. Send one provider query containing both the hash and metadata.
    /// 3. Let the provider decide how to combine those inputs.
    pub async fn search(
        &self,
        provider: &dyn SubtitleProvider,
        file_path: &Path,
        query: &SubtitleQuery,
    ) -> AppResult<Vec<SubtitleMatch>> {
        let mut combined_query = query.clone();
        if combined_query.file_hash.is_none() {
            combined_query.file_hash = compute_subtitle_file_hash(file_path).ok();
        }

        provider.search(&combined_query).await
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::subtitles::provider::{SubtitleFile, SubtitleMediaKind};

    fn subtitle_match(provider_file_id: &str, score: i32) -> SubtitleMatch {
        SubtitleMatch {
            provider: "opensubtitles".to_string(),
            provider_file_id: provider_file_id.to_string(),
            language: "eng".to_string(),
            release_info: None,
            score,
            score_percent: score,
            hearing_impaired: false,
            forced: false,
            ai_translated: false,
            machine_translated: false,
            uploader: None,
            download_count: None,
            hash_matched: false,
        }
    }

    #[derive(Default)]
    struct RecordingProvider {
        queries: Mutex<Vec<SubtitleQuery>>,
    }

    #[async_trait::async_trait]
    impl SubtitleProvider for RecordingProvider {
        async fn search(&self, query: &SubtitleQuery) -> AppResult<Vec<SubtitleMatch>> {
            self.queries
                .lock()
                .expect("recording provider mutex poisoned")
                .push(query.clone());
            Ok(vec![subtitle_match("file-1", 90)])
        }

        async fn download(&self, _provider_file_id: &str) -> AppResult<SubtitleFile> {
            unreachable!("download is not used in these tests")
        }

        fn name(&self) -> &str {
            "opensubtitles"
        }
    }

    fn base_query() -> SubtitleQuery {
        SubtitleQuery {
            media_kind: SubtitleMediaKind::Movie,
            facet: Some("movie".to_string()),
            file_hash: None,
            imdb_id: Some("tt1234567".to_string()),
            series_imdb_id: None,
            title: "Example Movie".to_string(),
            title_aliases: vec!["Example Alt".to_string()],
            title_candidates: vec!["Example Candidate".to_string()],
            year: Some(2024),
            season: None,
            episode: None,
            absolute_episode: None,
            community_entry: None,
            external_ids: Default::default(),
            languages: vec!["eng".to_string()],
            release_group: Some("GROUP".to_string()),
            source: Some("web".to_string()),
            video_codec: Some("h264".to_string()),
            audio_codec: Some("aac".to_string()),
            resolution: Some("1080p".to_string()),
            hearing_impaired: Some(false),
            include_ai_translated: false,
            include_machine_translated: false,
        }
    }

    #[tokio::test]
    async fn search_calls_provider_once_with_combined_hash_and_metadata_query() {
        let mut file = NamedTempFile::new().expect("temp subtitle search file");
        file.write_all(&vec![0u8; 131_072])
            .expect("write hashable file");

        let provider = RecordingProvider::default();
        let orchestrator = SubtitleSearchOrchestrator::new(120);
        let query = base_query();

        let results = orchestrator
            .search(&provider, file.path(), &query)
            .await
            .expect("combined search succeeds");

        assert_eq!(results.len(), 1);

        let recorded = provider
            .queries
            .lock()
            .expect("recording provider mutex poisoned");
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].file_hash.is_some());
        assert_eq!(recorded[0].imdb_id.as_deref(), Some("tt1234567"));
        assert_eq!(recorded[0].title, "Example Movie");
    }

    #[tokio::test]
    async fn search_calls_provider_once_without_hash_when_file_is_not_hashable() {
        let provider = RecordingProvider::default();
        let orchestrator = SubtitleSearchOrchestrator::new(120);
        let query = base_query();
        let missing_path = Path::new("/tmp/definitely-missing-subtitle-search-file");

        orchestrator
            .search(&provider, missing_path, &query)
            .await
            .expect("metadata-only search succeeds");

        let recorded = provider
            .queries
            .lock()
            .expect("recording provider mutex poisoned");
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].file_hash.is_none());
    }
}
