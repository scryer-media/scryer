use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{
    AppResult, IndexerArtifactLease, IndexerArtifactResolutionRequest, IndexerArtifactResolver,
    IndexerConfigRepository, IndexerProxyConfigRepository, IndexerStatsTracker,
    PreparedIndexerArtifact, ResolvedDownloadArtifact, StagedNzbStore, extract_magnet_info_hash,
    is_valid_magnet_uri,
};
use tokio::sync::Semaphore;

use crate::downloads::clients::StagedNzbLease;
use crate::indexers::artifact_staging::{
    BufferedOrStagedNzb, stage_nzb_from_bytes, stage_or_buffer_nzb_response,
};
use crate::indexers::artifact_transport::{
    ArtifactFetchResponse, ArtifactHttpResponse, IndexerArtifactTransport, classify,
};

/// Resolves artifacts at the indexer boundary before a download client is
/// chosen. The host owns artifact HTTP, solver handling, and staging.
pub struct AcquisitionIndexerArtifactResolver {
    artifact_transport: IndexerArtifactTransport,
    staged_nzb_store: Arc<dyn StagedNzbStore>,
    staged_nzb_pipeline_limit: Arc<Semaphore>,
}

impl AcquisitionIndexerArtifactResolver {
    pub fn new(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        indexer_proxy_configs: Arc<dyn IndexerProxyConfigRepository>,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
    ) -> Self {
        Self {
            artifact_transport: IndexerArtifactTransport::new(
                Arc::clone(&indexer_configs),
                Arc::clone(&indexer_proxy_configs),
            ),
            staged_nzb_store,
            staged_nzb_pipeline_limit,
        }
    }

    pub fn with_indexer_stats_tracker(
        mut self,
        indexer_stats: Arc<dyn IndexerStatsTracker>,
    ) -> Self {
        self.artifact_transport = self
            .artifact_transport
            .with_indexer_stats_tracker(indexer_stats);
        self
    }

    async fn prepare_http_artifact(
        &self,
        response: ArtifactHttpResponse,
        source: &str,
        request: &IndexerArtifactResolutionRequest,
    ) -> AppResult<PreparedIndexerArtifact> {
        let ArtifactHttpResponse {
            response,
            final_url,
            headers,
        } = response;
        let final_url = final_url.unwrap_or_else(|| source.to_string());
        match stage_or_buffer_nzb_response(
            response,
            &self.staged_nzb_store,
            &self.staged_nzb_pipeline_limit,
            &final_url,
            request.title_id.as_deref(),
            request.search_facet.as_ref(),
            &request.cancellation,
        )
        .await?
        {
            BufferedOrStagedNzb::Staged(lease) => {
                Ok(PreparedIndexerArtifact::StagedNzb(Box::new(lease)))
            }
            BufferedOrStagedNzb::Buffered(bytes) => {
                Ok(PreparedIndexerArtifact::Resolved(classify(
                    "indexer",
                    Some(&final_url),
                    headers.as_ref(),
                    bytes,
                    request.info_hash_hint.clone(),
                )?))
            }
        }
    }
}

impl IndexerArtifactLease for StagedNzbLease {
    fn staged_nzb(&self) -> &scryer_application::StagedNzbRef {
        &self.staged_nzb
    }
}

#[async_trait]
impl IndexerArtifactResolver for AcquisitionIndexerArtifactResolver {
    async fn resolve_artifact(
        &self,
        request: &IndexerArtifactResolutionRequest,
    ) -> AppResult<PreparedIndexerArtifact> {
        let source = request.source_url.trim();
        if source.is_empty() {
            return Err(scryer_application::AppError::Validation(
                "an indexer artifact source is required".to_string(),
            ));
        }
        if is_valid_magnet_uri(source) {
            return Ok(PreparedIndexerArtifact::Resolved(
                ResolvedDownloadArtifact::Magnet {
                    uri: source.to_string(),
                    info_hash_hint: extract_magnet_info_hash(source)
                        .or(request.info_hash_hint.clone()),
                },
            ));
        }

        let indexer_id = request
            .indexer_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty());
        let artifact = match self
            .artifact_transport
            .fetch_response_with_cancellation(indexer_id, source, &request.cancellation)
            .await?
        {
            Some(ArtifactFetchResponse::Resolved(artifact)) => artifact,
            Some(ArtifactFetchResponse::Http(response)) => {
                return self.prepare_http_artifact(response, source, request).await;
            }
            None => {
                self.artifact_transport
                    .resolve_with_cancellation(
                        indexer_id,
                        source,
                        request.info_hash_hint.clone(),
                        &request.cancellation,
                    )
                    .await?
            }
        };
        match artifact {
            ResolvedDownloadArtifact::Nzb { bytes, .. } => {
                let lease = stage_nzb_from_bytes(
                    &self.staged_nzb_store,
                    &self.staged_nzb_pipeline_limit,
                    "indexer_artifact",
                    request.title_id.as_deref(),
                    request.search_facet.as_ref(),
                    &request.cancellation,
                    bytes,
                )
                .await?;
                Ok(PreparedIndexerArtifact::StagedNzb(Box::new(lease)))
            }
            ResolvedDownloadArtifact::TorrentFile {
                bytes,
                file_name,
                content_type,
                info_hash_hint,
            } => {
                let mut headers = serde_json::Map::new();
                if let Some(content_type) = content_type {
                    headers.insert("content-type".into(), content_type.into());
                }
                if let Some(file_name) = file_name {
                    headers.insert(
                        "content-disposition".into(),
                        format!("attachment; filename=\"{file_name}\"").into(),
                    );
                }
                let headers = (!headers.is_empty()).then_some(serde_json::Value::Object(headers));
                Ok(PreparedIndexerArtifact::Resolved(classify(
                    "indexer",
                    Some(source),
                    headers.as_ref(),
                    bytes,
                    info_hash_hint.or(request.info_hash_hint.clone()),
                )?))
            }
            artifact => Ok(PreparedIndexerArtifact::Resolved(artifact)),
        }
    }
}
