//! Host-owned account authentication transport. Tokens never cross public APIs.
use crate::{AppError, AppResult};
use async_trait::async_trait;
use scryer_domain::ListAccountCredential;

const AUTH_FAILURE_PREFIX: &str = "list account authentication failed: ";

/// The provider or relay answered that the stored grant is no longer usable.
pub const RECONNECT_REQUIRED: &str = "reconnect_required";

/// The provider refused the request without a recognised error code.
pub const PROVIDER_REJECTED: &str = "provider_rejected";

/// The provider or relay asked Scryer to slow down.
pub const RATE_LIMITED: &str = "rate_limited";

/// The provider refused the user's consent or the grant's use.
pub const ACCESS_DENIED: &str = "access_denied";

/// Every code an account authentication failure can carry. Adding or renaming
/// one changes what callers parse from the message, so it must be deliberate.
pub const AUTH_FAILURE_CODES: &[&str] = &[
    "access_denied",
    "authorization_failed",
    "busy",
    "instance_auth_required",
    "instance_auth_unavailable",
    "invalid_provider_app",
    "invalid_provider_response",
    "invalid_request",
    "provider_not_configured",
    "provider_rejected",
    "provider_unavailable",
    "rate_limited",
    "reconnect_required",
    "relay_unavailable",
    "transport_unavailable",
    "unsupported_exchange",
    "unsupported_poll",
    "unsupported_provider",
    "unsupported_provider_app",
];

/// How a caller should treat an account authentication failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthFailureClass {
    /// The provider or the path to it is briefly unavailable; retry.
    Transient,
    /// Retry after the pause carried by the error, if any.
    RateLimited,
    /// Retrying the same request will not succeed.
    Final,
}

/// A failed account authentication carrying a bounded, server-owned code.
pub fn auth_failure(code: &str) -> AppError {
    AppError::Validation(format!("{AUTH_FAILURE_PREFIX}{code}"))
}

/// A `rate_limited` failure. It keeps the same message as every other
/// `auth_failure` but carries the provider's requested pause.
pub fn rate_limited_failure(retry_after: Option<std::time::Duration>) -> AppError {
    AppError::temporary_unavailable(format!("{AUTH_FAILURE_PREFIX}{RATE_LIMITED}"), retry_after)
}

/// The code of an `auth_failure` or `rate_limited_failure`, or `None` for any
/// other error.
pub fn auth_failure_code(error: &AppError) -> Option<&str> {
    match error {
        AppError::Validation(message) | AppError::TemporaryUnavailable { message, .. } => {
            message.strip_prefix(AUTH_FAILURE_PREFIX)
        }
        _ => None,
    }
}

/// The class of an account authentication failure code.
pub fn auth_failure_class(code: &str) -> AuthFailureClass {
    match code {
        "provider_unavailable" | "relay_unavailable" | "busy" | "transport_unavailable" => {
            AuthFailureClass::Transient
        }
        RATE_LIMITED => AuthFailureClass::RateLimited,
        _ => AuthFailureClass::Final,
    }
}

/// Whether a failed renew means the stored grant is no longer usable: the
/// provider refused it, or refused the renew request outright. Relay and
/// configuration faults are excluded, since a reconnect would not fix them.
pub fn renew_requires_reconnect(code: &str) -> bool {
    matches!(code, RECONNECT_REQUIRED | PROVIDER_REJECTED | ACCESS_DENIED)
}

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
