use async_graphql::{Context, Enum, ID, InputObject, Object, Result as GqlResult, SimpleObject};
use scryer_application::indexer_clients::{
    IndexerClientConfiguration, IndexerClientGrant, IndexerClientProtocol, IndexerClientRecord,
    IndexerClientUseOutcome,
};
use scryer_interface_core::{
    AuthlessDefaultSession, InteractiveSession, actor_from_ctx, app_from_ctx,
    auth_runtime_from_ctx, mfa_verification_from_ctx, to_gql_error,
};
use scryer_interface_media::types::Long;

/// Compatibility protocol an indexer client may use.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
enum IndexerClientProtocolValue {
    /// Newznab feeds for Usenet releases.
    Newznab,
    /// Torznab feeds for torrent releases.
    Torznab,
}

impl From<IndexerClientProtocolValue> for IndexerClientProtocol {
    fn from(value: IndexerClientProtocolValue) -> Self {
        match value {
            IndexerClientProtocolValue::Newznab => Self::Newznab,
            IndexerClientProtocolValue::Torznab => Self::Torznab,
        }
    }
}

impl From<IndexerClientProtocol> for IndexerClientProtocolValue {
    fn from(value: IndexerClientProtocol) -> Self {
        match value {
            IndexerClientProtocol::Newznab => Self::Newznab,
            IndexerClientProtocol::Torznab => Self::Torznab,
        }
    }
}

/// How Scryer finished preparing the most recent request from an indexer client.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
enum IndexerClientRequestOutcomeValue {
    /// A capabilities response was prepared.
    CapsReady,
    /// A search page was prepared with every granted source answering.
    SearchReady,
    /// A search page was prepared while some sources were pending or unavailable.
    SearchPartial,
    /// An authorized download was fetched and revalidated for delivery.
    ArtifactReady,
    /// The request was refused by the client's current authorization or input.
    Refused,
    /// The request could not be served for now.
    Unavailable,
    /// The request failed before a response was prepared.
    Failed,
}

impl From<IndexerClientUseOutcome> for IndexerClientRequestOutcomeValue {
    fn from(value: IndexerClientUseOutcome) -> Self {
        match value {
            IndexerClientUseOutcome::CapsReady => Self::CapsReady,
            IndexerClientUseOutcome::SearchReady => Self::SearchReady,
            IndexerClientUseOutcome::SearchPartial => Self::SearchPartial,
            IndexerClientUseOutcome::ArtifactReady => Self::ArtifactReady,
            IndexerClientUseOutcome::Refused => Self::Refused,
            IndexerClientUseOutcome::Unavailable => Self::Unavailable,
            IndexerClientUseOutcome::Failed => Self::Failed,
        }
    }
}

/// Reads a persisted outcome code back into its outcome. A code this build
/// does not write maps to null rather than failing the whole client list.
fn stored_outcome(code: &str) -> Option<IndexerClientRequestOutcomeValue> {
    [
        IndexerClientUseOutcome::CapsReady,
        IndexerClientUseOutcome::SearchReady,
        IndexerClientUseOutcome::SearchPartial,
        IndexerClientUseOutcome::ArtifactReady,
        IndexerClientUseOutcome::Refused,
        IndexerClientUseOutcome::Unavailable,
        IndexerClientUseOutcome::Failed,
    ]
    .into_iter()
    .find(|outcome| outcome.code() == code)
    .map(Into::into)
}

#[derive(InputObject)]
/// Explicit access to one existing source, including a managed child.
struct IndexerClientGrantInput {
    /// Existing indexer configuration identity.
    indexer_id: ID,
    /// Null permits all source categories; an empty list permits none.
    categories: Option<Vec<String>>,
}

#[derive(InputObject)]
/// Operator configuration for a client sharing Scryer's existing indexers.
struct IndexerClientConfigurationInput {
    /// Operator-visible client name.
    name: String,
    /// Whether this client may authenticate.
    enabled: bool,
    /// Compatibility protocols this key is allowed to access.
    protocols: Vec<IndexerClientProtocolValue>,
    /// Explicit source and category grants; an empty list grants no sources.
    grants: Vec<IndexerClientGrantInput>,
}

impl From<IndexerClientConfigurationInput> for IndexerClientConfiguration {
    fn from(value: IndexerClientConfigurationInput) -> Self {
        Self {
            name: value.name,
            enabled: value.enabled,
            protocols: value.protocols.into_iter().map(Into::into).collect(),
            grants: value
                .grants
                .into_iter()
                .map(|grant| IndexerClientGrant {
                    indexer_id: grant.indexer_id.to_string(),
                    categories: grant.categories,
                })
                .collect(),
        }
    }
}

#[derive(SimpleObject)]
/// Effective access to one existing indexer.
struct IndexerClientGrantPayload {
    /// Existing indexer configuration identity.
    indexer_id: ID,
    /// Null permits all categories; an empty list permits none.
    categories: Option<Vec<String>>,
}

/// Management record for an indexer client. This projection intentionally has
/// no lookup identifier or key verification material.
#[derive(SimpleObject)]
struct IndexerClientPayload {
    /// Stable client identity.
    id: ID,
    /// Operator-visible name.
    name: String,
    /// Whether this client may authenticate.
    enabled: bool,
    /// Allowed compatibility protocols.
    protocols: Vec<IndexerClientProtocolValue>,
    /// Explicit source and category grants.
    grants: Vec<IndexerClientGrantPayload>,
    /// Whether the client has a key that has not been revoked.
    key_active: bool,
    /// Current client revision required by update, rotate and revoke.
    revision: Long,
    /// When the most recent request from this client was recorded, or null if never.
    last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Outcome of the most recently recorded request, or null if none was recorded.
    last_outcome: Option<IndexerClientRequestOutcomeValue>,
    /// When this client was created.
    created_at: chrono::DateTime<chrono::Utc>,
    /// When its configuration or key last changed.
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<IndexerClientRecord> for IndexerClientPayload {
    fn from(value: IndexerClientRecord) -> Self {
        Self {
            id: value.id.into(),
            name: value.configuration.name,
            enabled: value.configuration.enabled,
            protocols: value
                .configuration
                .protocols
                .into_iter()
                .map(Into::into)
                .collect(),
            grants: value
                .configuration
                .grants
                .into_iter()
                .map(|grant| IndexerClientGrantPayload {
                    indexer_id: grant.indexer_id.into(),
                    categories: grant.categories,
                })
                .collect(),
            key_active: value.secret_hash.is_some(),
            revision: Long::from_u64_saturating(value.revision),
            last_used_at: value.last_used_at,
            last_outcome: value.last_outcome_code.as_deref().and_then(stored_outcome),
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(SimpleObject)]
/// A newly issued client key and its public management record.
struct IssuedIndexerClientKeyPayload {
    /// Current client configuration without key verification material.
    client: IndexerClientPayload,
    /// Returned once. Scryer does not persist this secret.
    api_key: String,
}

/// A signed-in session, or the default actor while form login is off for the
/// whole instance. While login is on, the default actor is the local-IP login
/// bypass and is refused. Login is read live so a change applies to the next
/// request.
fn management_session(ctx: &Context<'_>) -> bool {
    if ctx.data_opt::<AuthlessDefaultSession>().is_some() {
        return !auth_runtime_from_ctx(ctx)
            .snapshot()
            .effective_form_login_enabled;
    }
    ctx.data_opt::<InteractiveSession>().is_some()
}

fn revision(value: Long) -> GqlResult<u64> {
    u64::try_from(value.0)
        .ok()
        .filter(|revision| *revision > 0)
        .ok_or_else(|| {
            to_gql_error(scryer_application::AppError::validation_refused(
                scryer_application::error_reasons::indexer_clients::CONFIGURATION_CHANGED,
                "Indexer client revision is invalid; reload it",
            ))
        })
}

#[derive(Default)]
pub struct IndexerClientQueries;

#[Object]
impl IndexerClientQueries {
    /// List indexer clients. Client administration is available only to fully
    /// privileged interactive sessions.
    async fn indexer_clients(&self, ctx: &Context<'_>) -> GqlResult<Vec<IndexerClientPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(app
            .list_indexer_clients(&actor, management_session(ctx))
            .await
            .map_err(to_gql_error)?
            .into_iter()
            .map(Into::into)
            .collect())
    }
}

#[derive(Default)]
pub struct IndexerClientMutations;

#[Object]
impl IndexerClientMutations {
    /// Create an indexer client and reveal its key once; requires fresh MFA.
    async fn create_indexer_client(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Name, protocols and explicit source/category grants.")]
        input: IndexerClientConfigurationInput,
    ) -> GqlResult<IssuedIndexerClientKeyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let issued = app
            .create_indexer_client(
                &actor,
                management_session(ctx),
                mfa_verification_from_ctx(ctx).step_up_verified_until,
                input.into(),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(IssuedIndexerClientKeyPayload {
            client: issued.client.into(),
            api_key: issued.raw_key,
        })
    }

    /// Replace a client's configuration if its revision is still current; requires fresh MFA.
    async fn update_indexer_client(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Client identity to update.")] id: ID,
        #[graphql(desc = "Revision observed when opening the client for editing.")]
        expected_revision: Long,
        #[graphql(desc = "Complete replacement configuration and grants.")]
        input: IndexerClientConfigurationInput,
    ) -> GqlResult<IndexerClientPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.update_indexer_client(
            &actor,
            management_session(ctx),
            mfa_verification_from_ctx(ctx).step_up_verified_until,
            id.as_str(),
            revision(expected_revision)?,
            input.into(),
        )
        .await
        .map(Into::into)
        .map_err(to_gql_error)
    }

    /// Invalidate the previous key and reveal a new key once; requires fresh MFA.
    async fn rotate_indexer_client_key(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Client whose key is rotated.")] id: ID,
        #[graphql(desc = "Current client revision.")] expected_revision: Long,
    ) -> GqlResult<IssuedIndexerClientKeyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let issued = app
            .rotate_indexer_client_key(
                &actor,
                management_session(ctx),
                mfa_verification_from_ctx(ctx).step_up_verified_until,
                id.as_str(),
                revision(expected_revision)?,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(IssuedIndexerClientKeyPayload {
            client: issued.client.into(),
            api_key: issued.raw_key,
        })
    }

    /// Revoke a client's key and derived retrieval access; requires fresh MFA.
    async fn revoke_indexer_client_key(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Client whose key is revoked.")] id: ID,
        #[graphql(desc = "Current client revision.")] expected_revision: Long,
    ) -> GqlResult<IndexerClientPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.revoke_indexer_client_key(
            &actor,
            management_session(ctx),
            mfa_verification_from_ctx(ctx).step_up_verified_until,
            id.as_str(),
            revision(expected_revision)?,
        )
        .await
        .map(Into::into)
        .map_err(to_gql_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recorded_outcome_code_reads_back_as_its_outcome() {
        for code in scryer_application::indexer_clients::usage::ALL {
            let outcome = stored_outcome(code).expect("recorded code maps to an outcome");
            let expected = match *code {
                scryer_application::indexer_clients::usage::CAPS_READY => {
                    IndexerClientRequestOutcomeValue::CapsReady
                }
                scryer_application::indexer_clients::usage::SEARCH_READY => {
                    IndexerClientRequestOutcomeValue::SearchReady
                }
                scryer_application::indexer_clients::usage::SEARCH_PARTIAL => {
                    IndexerClientRequestOutcomeValue::SearchPartial
                }
                scryer_application::indexer_clients::usage::ARTIFACT_READY => {
                    IndexerClientRequestOutcomeValue::ArtifactReady
                }
                scryer_application::indexer_clients::usage::REFUSED => {
                    IndexerClientRequestOutcomeValue::Refused
                }
                scryer_application::indexer_clients::usage::UNAVAILABLE => {
                    IndexerClientRequestOutcomeValue::Unavailable
                }
                scryer_application::indexer_clients::usage::FAILED => {
                    IndexerClientRequestOutcomeValue::Failed
                }
                other => panic!("unmapped recorded outcome code {other}"),
            };
            assert!(outcome == expected, "{code} read back as the wrong outcome");
        }
    }

    #[test]
    fn an_unrecognized_stored_outcome_code_reads_as_no_outcome() {
        assert!(stored_outcome("SYNTHETIC_UNKNOWN_OUTCOME").is_none());
    }
}
