//! Host-owned account authentication transport. Tokens never cross public APIs.
use crate::{AppError, AppResult};
use async_trait::async_trait;
use scryer_domain::ListAccountCredential;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ListProviderAppConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub access_token: Option<String>,
}

#[derive(Clone)]
pub struct ListAccountStartRequest {
    pub provider: String,
    pub origin: String,
    pub state: String,
    pub code_challenge: String,
    pub app: Option<ListProviderAppConfig>,
}

#[derive(Clone)]
pub struct ListAccountStartResponse {
    pub authorize_url: String,
    /// Opaque provider request token or PIN; retained only in the host session.
    pub poll_token: Option<String>,
    pub expires_in: u64,
}

#[derive(Clone)]
pub struct ListAccountCompleteRequest {
    pub provider: String,
    pub code: String,
    pub code_verifier: String,
    pub origin: String,
    pub app: Option<ListProviderAppConfig>,
    pub poll_token: Option<String>,
}

#[async_trait]
pub trait ListAccountAuthGateway: Send + Sync {
    async fn start(&self, request: ListAccountStartRequest) -> AppResult<ListAccountStartResponse>;
    async fn poll(
        &self,
        provider: &str,
        poll_token: &str,
        app: Option<&ListProviderAppConfig>,
    ) -> AppResult<Option<ListAccountCredential>>;
    async fn complete(
        &self,
        request: ListAccountCompleteRequest,
    ) -> AppResult<ListAccountCredential>;
    async fn renew(
        &self,
        provider: &str,
        credential: &ListAccountCredential,
        app: Option<&ListProviderAppConfig>,
    ) -> AppResult<ListAccountCredential>;
    async fn revoke(
        &self,
        provider: &str,
        credential: &ListAccountCredential,
        app: Option<&ListProviderAppConfig>,
    ) -> AppResult<()>;
}

pub struct NullListAccountAuthGateway;
#[async_trait]
impl ListAccountAuthGateway for NullListAccountAuthGateway {
    async fn start(&self, _: ListAccountStartRequest) -> AppResult<ListAccountStartResponse> {
        Err(unavailable())
    }
    async fn poll(
        &self,
        _: &str,
        _: &str,
        _: Option<&ListProviderAppConfig>,
    ) -> AppResult<Option<ListAccountCredential>> {
        Err(unavailable())
    }
    async fn complete(&self, _: ListAccountCompleteRequest) -> AppResult<ListAccountCredential> {
        Err(unavailable())
    }
    async fn renew(
        &self,
        _: &str,
        _: &ListAccountCredential,
        _: Option<&ListProviderAppConfig>,
    ) -> AppResult<ListAccountCredential> {
        Err(unavailable())
    }
    async fn revoke(
        &self,
        _: &str,
        _: &ListAccountCredential,
        _: Option<&ListProviderAppConfig>,
    ) -> AppResult<()> {
        Err(unavailable())
    }
}
fn unavailable() -> AppError {
    AppError::Validation("list account authentication is unavailable".into())
}
