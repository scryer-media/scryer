use super::*;
use crate::catalog::interactive_release_search::scope_filter::category_contains;
use crate::{
    DownloadSourceKind, IndexerArtifactResolutionRequest, IndexerSearchResult,
    ResolvedDownloadArtifact,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

/// Serialized only inside the authenticated confidential reference envelope.
/// No search/cache identifier or timestamp participates in this identity.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexerClientArtifactReference {
    client_id: String,
    key_generation: u64,
    protocol: IndexerClientProtocol,
    indexer_id: String,
    result_identity: String,
    source_url: String,
    source_kind: DownloadSourceKind,
    categories: Vec<String>,
    info_hash_hint: Option<String>,
}

impl std::fmt::Debug for IndexerClientArtifactReference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IndexerClientArtifactReference")
            .field("client_id", &self.client_id)
            .field("indexer_id", &self.indexer_id)
            .finish_non_exhaustive()
    }
}

impl IndexerClientArtifactReference {
    /// Called only for a result already delivered by the scoped raw core.
    pub fn from_result(
        authenticated: &AuthenticatedIndexerClient,
        protocol: IndexerClientProtocol,
        result: &IndexerSearchResult,
    ) -> AppResult<Self> {
        let indexer_id = result
            .indexer_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?;
        let (source_url, mut source_kind) =
            result.canonical_download_source().ok_or_else(invalid)?;
        // Legacy single-protocol sources may omit a normalized source kind.
        // Their shared raw authorization already established the protocol.
        if result.source_kind.is_none()
            && protocol == IndexerClientProtocol::Newznab
            && matches!(source_kind, DownloadSourceKind::TorrentFile)
        {
            source_kind = DownloadSourceKind::NzbUrl;
        }
        let mut categories = result.response_attributes.categories.clone();
        categories.sort();
        categories.dedup();
        let reference = Self {
            client_id: authenticated.client.id.clone(),
            key_generation: authenticated.client.key_generation,
            protocol,
            indexer_id: indexer_id.into(),
            result_identity: result
                .guid
                .as_deref()
                .filter(|guid| !guid.is_empty())
                .unwrap_or(&source_url)
                .into(),
            source_url,
            source_kind,
            categories,
            info_hash_hint: result
                .extra
                .get("info_hash")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        };
        reference.require_grant(&authenticated.client, protocol)?;
        Ok(reference)
    }

    pub fn external_guid(&self) -> String {
        crate::helpers::blake3_identity_hex(
            crate::helpers::HashDomain::IndexerClientRelease,
            serde_json::json!([self.indexer_id, self.result_identity]).to_string(),
        )
    }

    /// An optional full key accompanying a retrieval must name the same client
    /// and generation as the narrow reference, never a different principal.
    pub fn require_authenticated_client(
        &self,
        authenticated: &AuthenticatedIndexerClient,
        protocol: IndexerClientProtocol,
    ) -> AppResult<()> {
        self.require_grant(&authenticated.client, protocol)
    }

    fn require_grant(
        &self,
        client: &IndexerClientRecord,
        protocol: IndexerClientProtocol,
    ) -> AppResult<()> {
        if client.id != self.client_id
            || client.key_generation != self.key_generation
            || !client.configuration.enabled
            || client.secret_hash.is_none()
            || protocol != self.protocol
            || !client.configuration.protocols.contains(&protocol)
            || !matches!(
                (protocol, self.source_kind),
                (
                    IndexerClientProtocol::Newznab,
                    DownloadSourceKind::NzbUrl | DownloadSourceKind::NzbFile
                ) | (
                    IndexerClientProtocol::Torznab,
                    DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri
                )
            )
        {
            return Err(invalid());
        }
        let grant = client
            .configuration
            .grants
            .iter()
            .find(|grant| grant.indexer_id == self.indexer_id)
            .ok_or_else(invalid)?;
        if grant.categories.as_ref().is_some_and(|allowed| {
            !self.categories.iter().any(|category| {
                allowed
                    .iter()
                    .any(|grant| category_contains(grant, category))
            })
        }) {
            return Err(invalid());
        }
        Ok(())
    }
}

impl AppUseCase {
    /// The HTTP boundary must authenticate/decrypt the envelope before calling
    /// this method. The enclosed identity authorizes one artifact, never a User.
    pub async fn fetch_indexer_client_artifact(
        &self,
        reference: &IndexerClientArtifactReference,
        protocol: IndexerClientProtocol,
    ) -> AppResult<ResolvedDownloadArtifact> {
        let client = self
            .indexer_client_repository()?
            .get_by_id(&reference.client_id)
            .await?
            .ok_or_else(invalid)?;
        reference.require_grant(&client, protocol)?;
        let authenticated = AuthenticatedIndexerClient { client };
        let result = async {
            let source = self
                .services
                .integrations
                .indexer_configs
                .get_by_id(&reference.indexer_id)
                .await?
                .ok_or_else(invalid)?;
            if !source.is_enabled {
                return Err(invalid());
            }
            let artifact = self
                .services
                .integrations
                .download_client
                .fetch_release_artifact(&IndexerArtifactResolutionRequest {
                    indexer_id: Some(reference.indexer_id.clone()),
                    source_url: reference.source_url.clone(),
                    source_kind: Some(reference.source_kind),
                    info_hash_hint: reference.info_hash_hint.clone(),
                    title_id: None,
                    search_facet: None,
                    cancellation: CancellationToken::new(),
                })
                .await?;
            // A physical successful grab is counted even if authority is revoked
            // before delivery. Cache reads or reference minting never count grabs.
            self.record_indexer_grab(Some(&source.id), Some(&source.name));
            self.revalidate_indexer_client(&authenticated).await?;
            Ok(artifact)
        }
        .await;
        self.record_indexer_client_use(
            &authenticated,
            match &result {
                Ok(_) => IndexerClientUseOutcome::ArtifactReady,
                Err(error) => IndexerClientUseOutcome::from_error(error),
            },
        )
        .await;
        result
    }
}

fn invalid() -> AppError {
    AppError::unauthorized_reason(reasons::REFERENCE_INVALID, "Invalid artifact reference")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (IndexerClientArtifactReference, IndexerClientRecord) {
        let now = chrono::Utc::now();
        let reference = IndexerClientArtifactReference {
            client_id: "client".into(),
            key_generation: 1,
            protocol: IndexerClientProtocol::Newznab,
            indexer_id: "indexer".into(),
            result_identity: "https://indexer.example/guid?apikey=secret".into(),
            source_url: "https://indexer.example/get?apikey=secret".into(),
            source_kind: DownloadSourceKind::NzbUrl,
            categories: vec!["4010".into()],
            info_hash_hint: None,
        };
        let client = IndexerClientRecord {
            id: "client".into(),
            configuration: IndexerClientConfiguration {
                name: "Fixture".into(),
                enabled: true,
                protocols: vec![IndexerClientProtocol::Newznab],
                grants: vec![IndexerClientGrant {
                    indexer_id: "indexer".into(),
                    categories: Some(vec!["4000".into()]),
                }],
            },
            key_generation: 1,
            revision: 1,
            lookup_id: Some("lookup".into()),
            secret_hash: Some("hash".into()),
            last_used_at: None,
            last_outcome_code: None,
            created_at: now,
            updated_at: now,
        };
        (reference, client)
    }

    #[test]
    fn retrieval_rechecks_current_key_protocol_source_and_category_grants() {
        let (reference, client) = fixture();
        reference
            .require_grant(&client, IndexerClientProtocol::Newznab)
            .unwrap();
        assert!(
            reference
                .require_grant(&client, IndexerClientProtocol::Torznab)
                .is_err()
        );
        let mut cases = vec![client.clone(); 6];
        cases[0].key_generation += 1;
        cases[1].configuration.enabled = false;
        cases[2].configuration.grants.clear();
        cases[3].configuration.grants[0].categories = Some(Vec::new());
        cases[4].configuration.grants[0].categories = Some(vec!["100123".into()]);
        cases[5].secret_hash = None;
        for client in cases {
            assert_eq!(
                reference
                    .require_grant(&client, IndexerClientProtocol::Newznab)
                    .unwrap_err()
                    .reason(),
                Some(reasons::REFERENCE_INVALID)
            );
        }
        let mut changed = client;
        changed.revision += 1;
        changed.configuration.name = "Renamed".into();
        reference
            .require_grant(&changed, IndexerClientProtocol::Newznab)
            .unwrap();
    }

    #[test]
    fn public_guid_is_source_qualified_opaque_and_independent_of_client_and_search_generation() {
        let (reference, _) = fixture();
        let guid = reference.external_guid();
        assert!(!guid.contains("secret"));
        let mut changed = reference.clone();
        changed.key_generation += 1;
        changed.client_id = "another-client".into();
        assert_eq!(guid, changed.external_guid());
        changed.indexer_id = "another-source".into();
        assert_ne!(guid, changed.external_guid());
        assert!(!format!("{reference:?}").contains("secret"));
    }
}
