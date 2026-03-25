use std::path::Path;

use super::provider::{
    SubtitleMatch, SubtitleMediaKind, SubtitleProvider, SubtitleQuery, compute_opensubtitles_hash,
};
use crate::AppResult;

/// Orchestrates subtitle searching: tries hash-based lookup first,
/// falls back to metadata-based search if no good matches found.
pub struct SubtitleSearchOrchestrator {
    min_score: i32,
}

impl SubtitleSearchOrchestrator {
    pub fn new(min_score: i32) -> Self {
        Self { min_score }
    }

    /// Search for subtitles for a media file.
    ///
    /// Strategy:
    /// 1. Compute file hash and search with it (highest confidence matches).
    /// 2. If no results above min_score, fall back to metadata search (IMDB ID, title+year).
    /// 3. Return all results sorted by score descending.
    pub async fn search(
        &self,
        provider: &dyn SubtitleProvider,
        file_path: &Path,
        media_kind: SubtitleMediaKind,
        title: &str,
        title_aliases: &[String],
        year: Option<i32>,
        imdb_id: Option<&str>,
        series_imdb_id: Option<&str>,
        season: Option<i32>,
        episode: Option<i32>,
        languages: &[String],
        release_group: Option<&str>,
        source: Option<&str>,
        video_codec: Option<&str>,
        audio_codec: Option<&str>,
        resolution: Option<&str>,
        hearing_impaired: Option<bool>,
        include_ai_translated: bool,
        include_machine_translated: bool,
    ) -> AppResult<Vec<SubtitleMatch>> {
        // Try hash-based search first
        let file_hash = compute_opensubtitles_hash(file_path).ok();

        if file_hash.is_some() {
            let query = SubtitleQuery {
                media_kind,
                file_hash: file_hash.clone(),
                imdb_id: imdb_id.map(|s| s.to_string()),
                series_imdb_id: series_imdb_id.map(|s| s.to_string()),
                title: title.to_string(),
                title_aliases: title_aliases.to_vec(),
                year,
                season,
                episode,
                languages: languages.to_vec(),
                release_group: release_group.map(|s| s.to_string()),
                source: source.map(|s| s.to_string()),
                video_codec: video_codec.map(|s| s.to_string()),
                audio_codec: audio_codec.map(|s| s.to_string()),
                resolution: resolution.map(|s| s.to_string()),
                hearing_impaired,
                include_ai_translated,
                include_machine_translated,
            };

            match provider.search(&query).await {
                Ok(results) if results.iter().any(|r| r.score >= self.min_score) => {
                    return Ok(results);
                }
                Ok(results) => {
                    tracing::debug!(
                        provider = provider.name(),
                        hash = ?file_hash,
                        results = results.len(),
                        "hash search returned results below min_score, trying metadata fallback"
                    );
                    // Keep hash results, we'll merge with metadata results
                    if !results.is_empty() {
                        // If we have hash results, use them even if below threshold
                        // (the metadata search might not find anything better)
                        let metadata_results = self
                            .search_by_metadata(
                                provider,
                                media_kind,
                                title,
                                title_aliases,
                                year,
                                imdb_id,
                                series_imdb_id,
                                season,
                                episode,
                                languages,
                                release_group,
                                source,
                                video_codec,
                                audio_codec,
                                resolution,
                                hearing_impaired,
                                include_ai_translated,
                                include_machine_translated,
                            )
                            .await
                            .unwrap_or_default();

                        return Ok(merge_results(results, metadata_results));
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "hash-based subtitle search failed, trying metadata");
                }
            }
        }

        // Metadata-based fallback
        self.search_by_metadata(
            provider,
            media_kind,
            title,
            title_aliases,
            year,
            imdb_id,
            series_imdb_id,
            season,
            episode,
            languages,
            release_group,
            source,
            video_codec,
            audio_codec,
            resolution,
            hearing_impaired,
            include_ai_translated,
            include_machine_translated,
        )
        .await
    }

    async fn search_by_metadata(
        &self,
        provider: &dyn SubtitleProvider,
        media_kind: SubtitleMediaKind,
        title: &str,
        title_aliases: &[String],
        year: Option<i32>,
        imdb_id: Option<&str>,
        series_imdb_id: Option<&str>,
        season: Option<i32>,
        episode: Option<i32>,
        languages: &[String],
        release_group: Option<&str>,
        source: Option<&str>,
        video_codec: Option<&str>,
        audio_codec: Option<&str>,
        resolution: Option<&str>,
        hearing_impaired: Option<bool>,
        include_ai_translated: bool,
        include_machine_translated: bool,
    ) -> AppResult<Vec<SubtitleMatch>> {
        let query = SubtitleQuery {
            media_kind,
            file_hash: None,
            imdb_id: imdb_id.map(|s| s.to_string()),
            series_imdb_id: series_imdb_id.map(|s| s.to_string()),
            title: title.to_string(),
            title_aliases: title_aliases.to_vec(),
            year,
            season,
            episode,
            languages: languages.to_vec(),
            release_group: release_group.map(|s| s.to_string()),
            source: source.map(|s| s.to_string()),
            video_codec: video_codec.map(|s| s.to_string()),
            audio_codec: audio_codec.map(|s| s.to_string()),
            resolution: resolution.map(|s| s.to_string()),
            hearing_impaired,
            include_ai_translated,
            include_machine_translated,
        };

        provider.search(&query).await
    }
}

/// Merge two result sets, deduplicating by provider_file_id, keeping higher scores.
fn merge_results(primary: Vec<SubtitleMatch>, secondary: Vec<SubtitleMatch>) -> Vec<SubtitleMatch> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();

    for r in primary {
        if seen.insert(r.provider_file_id.clone()) {
            merged.push(r);
        }
    }
    for r in secondary {
        if seen.insert(r.provider_file_id.clone()) {
            merged.push(r);
        }
    }

    merged.sort_by(|a, b| b.score.cmp(&a.score));
    merged
}
