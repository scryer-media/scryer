//! A typed client for TorBox's HTTP API (`/v1/api`).
//!
//! Every call is sent once. An answer comes back as a typed value, a refusal
//! as a [`TorboxError`] the caller schedules around; nothing here retries,
//! waits, or remembers anything between calls.
//!
//! TorBox documents its payloads only by example, so decoding is lenient:
//! unknown fields are ignored, most fields are optional, and numbers or
//! booleans sent as strings are still read. Only the id an item is addressed
//! by is required.
//!
//! The API token is a secret. It never appears in `Debug` output, in an error,
//! or in a logged URL; the one call that carries it in the query string
//! (`requestdl`) never reports its URL. The download link that call returns is
//! a bearer credential too and is wrapped in [`TorboxDownloadUrl`].

use std::fmt;
use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use reqwest::{RequestBuilder, StatusCode, multipart};
use scryer_application::{AppError, PluginFailureReason, torbox_refusal};
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

/// TorBox's public API host.
pub const TORBOX_API_BASE_URL: &str = "https://api.torbox.app";
const TORBOX_API_PREFIX: &str = "/v1/api";
const TORBOX_REQUEST_TIMEOUT: Duration = scryer_outbound_http::DOWNLOAD_CLIENT_HTTP_TIMEOUT;
/// The wait suggested after a 429 that names none. TorBox counts its
/// per-token limit per minute.
pub const TORBOX_DEFAULT_RATE_LIMIT_RETRY: Duration = Duration::from_secs(60);
/// The wait suggested after a TorBox-side fault that names none.
pub const TORBOX_DEFAULT_SERVER_FAULT_RETRY: Duration = Duration::from_secs(60);
const REDACTED: &str = "<redacted>";
/// The largest answer read from TorBox. Generous for the list of a large
/// account; anything bigger is not read.
pub const TORBOX_MAX_ANSWER_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
struct TorboxToken(String);

impl fmt::Debug for TorboxToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

/// One TorBox account, reached through an injected HTTP client.
#[derive(Clone)]
pub struct TorboxApi {
    base_url: String,
    token: TorboxToken,
    http: reqwest::Client,
    max_answer_bytes: usize,
    /// Local mock servers hand out plain `http` links.
    #[cfg(test)]
    plain_http_links: bool,
}

impl fmt::Debug for TorboxApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TorboxApi")
            .field("base_url", &self.base_url)
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

/// Which of TorBox's two download families an item belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TorboxItemKind {
    Torrent,
    Usenet,
}

impl TorboxItemKind {
    fn list_path(self) -> &'static str {
        match self {
            Self::Torrent => "torrents/mylist",
            Self::Usenet => "usenet/mylist",
        }
    }

    fn request_download_path(self) -> &'static str {
        match self {
            Self::Torrent => "torrents/requestdl",
            Self::Usenet => "usenet/requestdl",
        }
    }

    fn control_path(self) -> &'static str {
        match self {
            Self::Torrent => "torrents/controltorrent",
            Self::Usenet => "usenet/controlusenetdownload",
        }
    }

    fn id_param(self) -> &'static str {
        match self {
            Self::Torrent => "torrent_id",
            Self::Usenet => "usenet_id",
        }
    }
}

/// The call an outcome belongs to, named in its sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorboxOperation {
    UserMe,
    CreateTorrent,
    CreateUsenet,
    List(TorboxItemKind),
    RequestDownload(TorboxItemKind),
    Control(TorboxItemKind),
}

impl fmt::Display for TorboxOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = |kind: &TorboxItemKind| match kind {
            TorboxItemKind::Torrent => "torrent",
            TorboxItemKind::Usenet => "usenet",
        };
        match self {
            Self::UserMe => f.write_str("account lookup"),
            Self::CreateTorrent => f.write_str("torrent create"),
            Self::CreateUsenet => f.write_str("usenet create"),
            Self::List(item) => write!(f, "{} list", kind(item)),
            Self::RequestDownload(item) => write!(f, "{} download link request", kind(item)),
            Self::Control(item) => write!(f, "{} control", kind(item)),
        }
    }
}

/// TorBox's `error` field, read once at the boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TorboxErrorCode {
    DatabaseError,
    UnknownError,
    NoAuth,
    BadToken,
    AuthError,
    InvalidOption,
    EndpointNotFound,
    ItemNotFound,
    PlanRestrictedFeature,
    DuplicateItem,
    TooMuchData,
    DownloadTooLarge,
    MissingRequiredOption,
    TooManyOptions,
    BozoTorrent,
    NoServersAvailableError,
    MonthlyLimit,
    CooldownLimit,
    ActiveLimit,
    DownloadServerError,
    BozoNzb,
    LinkOffline,
    VendorDisabled,
    /// A code this client does not know, as TorBox sent it.
    Other(String),
}

impl TorboxErrorCode {
    pub fn parse(code: &str) -> Self {
        match code.trim() {
            "DATABASE_ERROR" => Self::DatabaseError,
            "UNKNOWN_ERROR" => Self::UnknownError,
            "NO_AUTH" => Self::NoAuth,
            "BAD_TOKEN" => Self::BadToken,
            "AUTH_ERROR" => Self::AuthError,
            "INVALID_OPTION" => Self::InvalidOption,
            "ENDPOINT_NOT_FOUND" => Self::EndpointNotFound,
            "ITEM_NOT_FOUND" => Self::ItemNotFound,
            "PLAN_RESTRICTED_FEATURE" => Self::PlanRestrictedFeature,
            "DUPLICATE_ITEM" => Self::DuplicateItem,
            "TOO_MUCH_DATA" => Self::TooMuchData,
            "DOWNLOAD_TOO_LARGE" => Self::DownloadTooLarge,
            "MISSING_REQUIRED_OPTION" => Self::MissingRequiredOption,
            "TOO_MANY_OPTIONS" => Self::TooManyOptions,
            "BOZO_TORRENT" => Self::BozoTorrent,
            "NO_SERVERS_AVAILABLE_ERROR" => Self::NoServersAvailableError,
            "MONTHLY_LIMIT" => Self::MonthlyLimit,
            "COOLDOWN_LIMIT" => Self::CooldownLimit,
            "ACTIVE_LIMIT" => Self::ActiveLimit,
            "DOWNLOAD_SERVER_ERROR" => Self::DownloadServerError,
            "BOZO_NZB" => Self::BozoNzb,
            "LINK_OFFLINE" => Self::LinkOffline,
            "VENDOR_DISABLED" => Self::VendorDisabled,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::DatabaseError => "DATABASE_ERROR",
            Self::UnknownError => "UNKNOWN_ERROR",
            Self::NoAuth => "NO_AUTH",
            Self::BadToken => "BAD_TOKEN",
            Self::AuthError => "AUTH_ERROR",
            Self::InvalidOption => "INVALID_OPTION",
            Self::EndpointNotFound => "ENDPOINT_NOT_FOUND",
            Self::ItemNotFound => "ITEM_NOT_FOUND",
            Self::PlanRestrictedFeature => "PLAN_RESTRICTED_FEATURE",
            Self::DuplicateItem => "DUPLICATE_ITEM",
            Self::TooMuchData => "TOO_MUCH_DATA",
            Self::DownloadTooLarge => "DOWNLOAD_TOO_LARGE",
            Self::MissingRequiredOption => "MISSING_REQUIRED_OPTION",
            Self::TooManyOptions => "TOO_MANY_OPTIONS",
            Self::BozoTorrent => "BOZO_TORRENT",
            Self::NoServersAvailableError => "NO_SERVERS_AVAILABLE_ERROR",
            Self::MonthlyLimit => "MONTHLY_LIMIT",
            Self::CooldownLimit => "COOLDOWN_LIMIT",
            Self::ActiveLimit => "ACTIVE_LIMIT",
            Self::DownloadServerError => "DOWNLOAD_SERVER_ERROR",
            Self::BozoNzb => "BOZO_NZB",
            Self::LinkOffline => "LINK_OFFLINE",
            Self::VendorDisabled => "VENDOR_DISABLED",
            Self::Other(code) => code,
        }
    }

    /// A fault on TorBox's side. TorBox's own convention is that a code ending
    /// in `ERROR` is one; `AUTH_ERROR` is the exception, since it reports a
    /// token TorBox could not verify.
    pub fn is_server_fault(&self) -> bool {
        match self {
            Self::DatabaseError
            | Self::UnknownError
            | Self::NoServersAvailableError
            | Self::DownloadServerError => true,
            Self::AuthError => false,
            Self::Other(code) => code.ends_with("ERROR"),
            _ => false,
        }
    }
}

impl fmt::Display for TorboxErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a request failed to get an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorboxTransportKind {
    /// The request was never sent: it could not be built, or no connection
    /// was made.
    NotSent,
    /// No answer arrived in time; TorBox may have acted on the request.
    TimedOut,
    /// The exchange broke after the request may have reached TorBox.
    Interrupted,
}

/// Every way a TorBox call can end without the value asked for.
#[derive(Debug)]
pub enum TorboxError {
    /// TorBox answered and refused. `code` is its stable `error` field, and
    /// `detail` its sentence, relayed only for people.
    Refused {
        operation: TorboxOperation,
        status: u16,
        code: Option<TorboxErrorCode>,
        detail: Option<String>,
        retry_after: Option<Duration>,
    },
    /// TorBox answered 429.
    RateLimited {
        operation: TorboxOperation,
        retry_after: Option<Duration>,
    },
    /// The create asked TorBox to add the item only if already cached, and
    /// TorBox did not add it.
    NotCached { operation: TorboxOperation },
    /// No answer was received.
    Transport {
        operation: TorboxOperation,
        kind: TorboxTransportKind,
        message: String,
    },
    /// An answer arrived but could not be read.
    Decode {
        operation: TorboxOperation,
        status: u16,
        message: String,
    },
}

impl fmt::Display for TorboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused {
                operation,
                status,
                code,
                detail,
                ..
            } => {
                write!(f, "TorBox {operation} was refused")?;
                if let Some(code) = code {
                    write!(f, " with {code}")?;
                }
                write!(f, " (HTTP {status})")?;
                if let Some(detail) = detail.as_deref().filter(|d| !d.trim().is_empty()) {
                    write!(f, ": {detail}")?;
                }
                Ok(())
            }
            Self::RateLimited {
                operation,
                retry_after,
            } => match retry_after {
                Some(delay) => write!(
                    f,
                    "TorBox {operation} was rate limited; retry after {}s",
                    delay.as_secs()
                ),
                None => write!(f, "TorBox {operation} was rate limited"),
            },
            Self::NotCached { operation } => write!(
                f,
                "TorBox {operation} was declined because TorBox does not have it cached"
            ),
            Self::Transport {
                operation, message, ..
            } => write!(f, "TorBox {operation} failed: {message}"),
            Self::Decode {
                operation, message, ..
            } => write!(f, "TorBox {operation} answer could not be read: {message}"),
        }
    }
}

impl std::error::Error for TorboxError {}

impl TorboxError {
    /// TorBox's code for a refusal.
    pub fn code(&self) -> Option<&TorboxErrorCode> {
        match self {
            Self::Refused { code, .. } => code.as_ref(),
            _ => None,
        }
    }

    pub fn is_item_not_found(&self) -> bool {
        self.code() == Some(&TorboxErrorCode::ItemNotFound)
    }

    /// The request may have taken effect even though no usable answer came
    /// back, so a create must be reconciled rather than sent again blindly.
    pub fn outcome_unknown(&self) -> bool {
        match self {
            Self::Transport { kind, .. } => *kind != TorboxTransportKind::NotSent,
            Self::Decode { status, .. } => (200..300).contains(status),
            _ => false,
        }
    }

    /// [`Self::outcome_unknown`] for a create, which also counts a server
    /// error that carries no TorBox code: a gateway in front of TorBox may
    /// answer 5xx after TorBox made the item. A 5xx with TorBox's own code
    /// is TorBox refusing.
    pub fn create_outcome_unknown(&self) -> bool {
        self.outcome_unknown()
            || matches!(
                self,
                Self::Refused {
                    status: 500..,
                    code: None,
                    ..
                }
            )
    }

    /// The Scryer error for this outcome, carrying a `torbox_refusal` reason
    /// wherever the condition has one.
    pub fn into_app_error(self) -> AppError {
        let message = self.to_string();
        match self {
            Self::RateLimited { retry_after, .. } => AppError::temporary_unavailable_reason(
                torbox_refusal::RATE_LIMITED,
                message,
                Some(retry_after.unwrap_or(TORBOX_DEFAULT_RATE_LIMIT_RETRY)),
            ),
            Self::NotCached { .. } => {
                AppError::validation_refused(torbox_refusal::NOT_CACHED, message)
            }
            Self::Transport { kind, .. } => AppError::native_client_failed(
                match kind {
                    TorboxTransportKind::TimedOut => PluginFailureReason::TimedOut,
                    TorboxTransportKind::NotSent | TorboxTransportKind::Interrupted => {
                        PluginFailureReason::UpstreamUnavailable
                    }
                },
                message,
            ),
            Self::Decode { .. } => {
                AppError::native_client_failed(PluginFailureReason::UpstreamUnavailable, message)
            }
            Self::Refused {
                status,
                code: Some(code),
                retry_after,
                ..
            } => refusal_for_code(&code, status, message, retry_after),
            Self::Refused {
                status,
                code: None,
                retry_after,
                ..
            } => match status {
                401 | 403 => AppError::validation_refused(torbox_refusal::TOKEN_REJECTED, message),
                500.. => server_fault(message, retry_after),
                _ => AppError::native_client_failed(PluginFailureReason::Permanent, message),
            },
        }
    }
}

impl From<TorboxError> for AppError {
    fn from(error: TorboxError) -> Self {
        error.into_app_error()
    }
}

fn refusal_for_code(
    code: &TorboxErrorCode,
    status: u16,
    message: String,
    retry_after: Option<Duration>,
) -> AppError {
    use TorboxErrorCode as C;
    let refused = |reason| AppError::validation_refused(reason, message.clone());
    match code {
        C::NoAuth | C::BadToken | C::AuthError => refused(torbox_refusal::TOKEN_REJECTED),
        C::PlanRestrictedFeature => refused(torbox_refusal::PLAN_RESTRICTED),
        C::MonthlyLimit => refused(torbox_refusal::MONTHLY_LIMIT),
        C::DownloadTooLarge => refused(torbox_refusal::DOWNLOAD_TOO_LARGE),
        C::BozoTorrent => refused(torbox_refusal::INVALID_TORRENT),
        C::BozoNzb => refused(torbox_refusal::INVALID_NZB),
        C::DuplicateItem => refused(torbox_refusal::DUPLICATE_ITEM),
        C::LinkOffline => refused(torbox_refusal::LINK_OFFLINE),
        C::ItemNotFound => AppError::not_found_reason(torbox_refusal::ITEM_NOT_FOUND, message),
        // Both clear on their own: a slot frees up, or the cooldown passes.
        C::ActiveLimit => AppError::temporary_unavailable_reason(
            torbox_refusal::ACTIVE_LIMIT,
            message,
            retry_after,
        ),
        C::CooldownLimit => AppError::temporary_unavailable_reason(
            torbox_refusal::COOLDOWN_LIMIT,
            message,
            retry_after,
        ),
        code if code.is_server_fault() || status >= 500 => server_fault(message, retry_after),
        C::VendorDisabled => {
            AppError::native_client_failed(PluginFailureReason::UpstreamUnavailable, message)
        }
        // A request this client built wrong, or a code it does not know.
        _ => AppError::native_client_failed(PluginFailureReason::Permanent, message),
    }
}

fn server_fault(message: String, retry_after: Option<Duration>) -> AppError {
    AppError::temporary_unavailable_reason(
        torbox_refusal::UNAVAILABLE,
        message,
        Some(retry_after.unwrap_or(TORBOX_DEFAULT_SERVER_FAULT_RETRY)),
    )
}

/// The account behind the token.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct TorboxUser {
    #[serde(default, deserialize_with = "lenient_plan")]
    pub plan: Option<TorboxPlan>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub email: Option<String>,
}

impl fmt::Debug for TorboxUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TorboxUser")
            .field("plan", &self.plan)
            .field("email", &self.email.as_ref().map(|_| REDACTED))
            .finish()
    }
}

/// A TorBox subscription plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorboxPlan {
    Free,
    Essential,
    Pro,
    Standard,
    /// A plan number this client does not know.
    Unknown(i64),
}

impl TorboxPlan {
    pub fn from_number(plan: i64) -> Self {
        match plan {
            0 => Self::Free,
            1 => Self::Essential,
            2 => Self::Pro,
            3 => Self::Standard,
            other => Self::Unknown(other),
        }
    }
}

/// A torrent or usenet download TorBox holds for the account.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct TorboxItem {
    #[serde(deserialize_with = "required_i64")]
    pub id: i64,
    #[serde(default, deserialize_with = "lenient_string")]
    pub hash: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub size: Option<i64>,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub active: bool,
    /// TorBox's state string (qBittorrent's vocabulary for torrents). It is
    /// not a completion signal: see [`TorboxItem::is_downloaded`].
    #[serde(default, deserialize_with = "lenient_string")]
    pub download_state: Option<String>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub seeds: Option<i64>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub peers: Option<i64>,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub ratio: Option<f64>,
    /// Fraction done, 0 to 1.
    #[serde(default, deserialize_with = "lenient_f64")]
    pub progress: Option<f64>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub download_speed: Option<i64>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub upload_speed: Option<i64>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub eta: Option<i64>,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub download_present: bool,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub download_finished: bool,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub cached: bool,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub seed_torrent: bool,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub long_term_seeding: bool,
    /// `None` when the answer did not say.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub private: Option<bool>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub tracker_message: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub expires_at: Option<String>,
    #[serde(default, deserialize_with = "lenient_files")]
    pub files: Vec<TorboxFile>,
}

impl TorboxItem {
    /// TorBox has the whole download and it can be fetched. TorBox's docs
    /// name this pair, not `download_state == "completed"`, as the signal.
    pub fn is_downloaded(&self) -> bool {
        self.download_finished && self.download_present
    }
}

/// One file of a TorBox item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct TorboxFile {
    #[serde(deserialize_with = "required_i64")]
    pub id: i64,
    /// The path inside the item, folder included.
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub short_name: Option<String>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub size: Option<i64>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub mimetype: Option<String>,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub zipped: bool,
    #[serde(default, deserialize_with = "lenient_flag")]
    pub infected: bool,
    #[serde(default, deserialize_with = "lenient_string")]
    pub absolute_path: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub md5: Option<String>,
}

/// What a create answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorboxCreated {
    /// `torrent_id` or `usenetdownload_id`: the id every later call names.
    pub id: i64,
    pub hash: Option<String>,
    pub auth_id: Option<String>,
    /// Whether TorBox already had the item cached, when the answer says. The
    /// documented answers carry no such field, so this is usually `None`.
    pub cached: Option<bool>,
}

/// Where a torrent create's payload comes from.
#[derive(Clone)]
pub enum TorboxTorrentSource {
    File { file_name: String, bytes: Vec<u8> },
    Magnet(String),
}

impl fmt::Debug for TorboxTorrentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A torrent or magnet can carry a private tracker's passkey.
        match self {
            Self::File { file_name, bytes } => f
                .debug_struct("File")
                .field("file_name", file_name)
                .field("len", &bytes.len())
                .finish(),
            Self::Magnet(_) => f.debug_tuple("Magnet").field(&REDACTED).finish(),
        }
    }
}

/// TorBox's `seed` option for a torrent create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorboxSeedPreference {
    /// Follow the account's own setting.
    Auto,
    Seed,
    DoNotSeed,
}

impl TorboxSeedPreference {
    fn form_value(self) -> &'static str {
        match self {
            Self::Auto => "1",
            Self::Seed => "2",
            Self::DoNotSeed => "3",
        }
    }
}

/// A `createtorrent` request.
#[derive(Clone, Debug)]
pub struct TorboxTorrentCreate {
    pub source: TorboxTorrentSource,
    pub seed: Option<TorboxSeedPreference>,
    pub allow_zip: bool,
    pub name: Option<String>,
    pub as_queued: Option<bool>,
    pub add_only_if_cached: bool,
}

/// Where a usenet create's payload comes from.
#[derive(Clone)]
pub enum TorboxUsenetSource {
    File {
        file_name: String,
        bytes: Vec<u8>,
    },
    /// A link TorBox fetches the NZB from. Indexer links carry API keys.
    Link(String),
}

impl fmt::Debug for TorboxUsenetSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File { file_name, bytes } => f
                .debug_struct("File")
                .field("file_name", file_name)
                .field("len", &bytes.len())
                .finish(),
            Self::Link(_) => f.debug_tuple("Link").field(&REDACTED).finish(),
        }
    }
}

/// A `createusenetdownload` request.
#[derive(Clone)]
pub struct TorboxUsenetCreate {
    pub source: TorboxUsenetSource,
    pub name: Option<String>,
    pub password: Option<String>,
    /// TorBox's `post_processing` level; sent only when set.
    pub post_processing: Option<i64>,
    pub as_queued: Option<bool>,
    pub add_only_if_cached: bool,
}

impl fmt::Debug for TorboxUsenetCreate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TorboxUsenetCreate")
            .field("source", &self.source)
            .field("name", &self.name)
            .field("password", &self.password.as_ref().map(|_| REDACTED))
            .field("post_processing", &self.post_processing)
            .field("as_queued", &self.as_queued)
            .field("add_only_if_cached", &self.add_only_if_cached)
            .finish()
    }
}

/// Paging and freshness for a `mylist` read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TorboxListQuery {
    /// Ask TorBox for fresh state rather than its short-lived cache.
    pub bypass_cache: bool,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

/// What a `requestdl` link should fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorboxDownloadTarget {
    File(i64),
    /// The whole item as one zip.
    Zip,
}

/// A signed download link from `requestdl`. It is a bearer credential for
/// the file, so its `Debug` hides it and it has no `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct TorboxDownloadUrl(String);

impl TorboxDownloadUrl {
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for TorboxDownloadUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TorboxDownloadUrl").field(&REDACTED).finish()
    }
}

#[derive(Default, Deserialize)]
struct Envelope {
    #[serde(default, deserialize_with = "lenient_bool")]
    success: Option<bool>,
    #[serde(default, deserialize_with = "lenient_string")]
    error: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    detail: Option<String>,
    #[serde(default)]
    data: Value,
}

struct Answer {
    status: u16,
    data: Value,
}

impl TorboxApi {
    /// A client for TorBox's public API. `http` carries the traffic, so an
    /// operator-assigned proxy applies.
    pub fn new(http: reqwest::Client, token: impl Into<String>) -> Self {
        Self {
            base_url: TORBOX_API_BASE_URL.to_string(),
            token: TorboxToken(token.into().trim().to_string()),
            http,
            max_answer_bytes: TORBOX_MAX_ANSWER_BYTES,
            #[cfg(test)]
            plain_http_links: false,
        }
    }

    /// Point the client at another host, for tests.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    /// Accept the plain `http` download links a local mock server hands out.
    #[cfg(test)]
    pub(crate) fn allow_plain_http_links(mut self) -> Self {
        self.plain_http_links = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_max_answer_bytes(mut self, max: usize) -> Self {
        self.max_answer_bytes = max;
        self
    }

    fn accepts_link_scheme(&self, scheme: &str) -> bool {
        #[cfg(test)]
        if self.plain_http_links && scheme == "http" {
            return true;
        }
        scheme == "https"
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `GET /user/me`.
    pub async fn user_me(&self) -> Result<TorboxUser, TorboxError> {
        let operation = TorboxOperation::UserMe;
        let request = self.authorized(self.http.get(self.url("user/me")));
        let answer = self.execute(operation, request).await?;
        decode_value(operation, answer.status, answer.data)
    }

    /// `POST /torrents/createtorrent`.
    pub async fn create_torrent(
        &self,
        request: TorboxTorrentCreate,
    ) -> Result<TorboxCreated, TorboxError> {
        let operation = TorboxOperation::CreateTorrent;
        let add_only_if_cached = request.add_only_if_cached;
        let mut form = multipart::Form::new();
        form = match request.source {
            TorboxTorrentSource::File { file_name, bytes } => form.part(
                "file",
                file_part(operation, bytes, file_name, "application/x-bittorrent")?,
            ),
            TorboxTorrentSource::Magnet(magnet) => form.text("magnet", magnet),
        };
        if let Some(seed) = request.seed {
            form = form.text("seed", seed.form_value());
        }
        form = form.text("allow_zip", bool_text(request.allow_zip));
        if let Some(name) = request.name {
            form = form.text("name", name);
        }
        if let Some(as_queued) = request.as_queued {
            form = form.text("as_queued", bool_text(as_queued));
        }
        form = form.text("add_only_if_cached", bool_text(add_only_if_cached));

        let http_request = self.authorized(
            self.http
                .post(self.url("torrents/createtorrent"))
                .multipart(form),
        );
        self.create(
            operation,
            http_request,
            &["torrent_id", "id"],
            add_only_if_cached,
        )
        .await
    }

    /// `POST /usenet/createusenetdownload`.
    pub async fn create_usenet(
        &self,
        request: TorboxUsenetCreate,
    ) -> Result<TorboxCreated, TorboxError> {
        let operation = TorboxOperation::CreateUsenet;
        let add_only_if_cached = request.add_only_if_cached;
        let mut form = multipart::Form::new();
        form = match request.source {
            TorboxUsenetSource::File { file_name, bytes } => form.part(
                "file",
                file_part(operation, bytes, file_name, "application/x-nzb")?,
            ),
            TorboxUsenetSource::Link(link) => form.text("link", link),
        };
        if let Some(name) = request.name {
            form = form.text("name", name);
        }
        if let Some(password) = request.password {
            form = form.text("password", password);
        }
        if let Some(post_processing) = request.post_processing {
            form = form.text("post_processing", post_processing.to_string());
        }
        if let Some(as_queued) = request.as_queued {
            form = form.text("as_queued", bool_text(as_queued));
        }
        form = form.text("add_only_if_cached", bool_text(add_only_if_cached));

        let http_request = self.authorized(
            self.http
                .post(self.url("usenet/createusenetdownload"))
                .multipart(form),
        );
        // TorBox documents no success answer for this call. The id is read
        // from `usenetdownload_id`, the name its other endpoints use, and
        // from `usenet_id` or `id` in case the answer names it otherwise.
        self.create(
            operation,
            http_request,
            &["usenetdownload_id", "usenet_id", "id"],
            add_only_if_cached,
        )
        .await
    }

    /// `GET /{kind}/mylist`: one page of the account's items.
    pub async fn list(
        &self,
        kind: TorboxItemKind,
        query: TorboxListQuery,
    ) -> Result<Vec<TorboxItem>, TorboxError> {
        let operation = TorboxOperation::List(kind);
        let mut params = vec![("bypass_cache", bool_text(query.bypass_cache).to_string())];
        if let Some(offset) = query.offset {
            params.push(("offset", offset.to_string()));
        }
        if let Some(limit) = query.limit {
            params.push(("limit", limit.to_string()));
        }
        let request = self.authorized(self.http.get(self.url(kind.list_path())).query(&params));
        let answer = self.execute(operation, request).await?;
        match answer.data {
            Value::Null => Ok(Vec::new()),
            // One unreadable item is skipped rather than failing the whole
            // read: a job whose item is missing from a list is confirmed by
            // a single-item read before anything is concluded about it.
            Value::Array(items) => Ok(items
                .into_iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(value_i64);
                    match decode_value::<TorboxItem>(operation, answer.status, item) {
                        Ok(item) => Some(item),
                        Err(_) => {
                            tracing::debug!(
                                item_id = ?id,
                                "skipping a TorBox list item that could not be read"
                            );
                            None
                        }
                    }
                })
                .collect()),
            // A single object where a list was expected is read as a list
            // of one.
            item @ Value::Object(_) => Ok(vec![decode_value(operation, answer.status, item)?]),
            _ => Err(TorboxError::Decode {
                operation,
                status: answer.status,
                message: "the item list is neither a list nor an object".to_string(),
            }),
        }
    }

    /// `GET /{kind}/mylist?id=`: one item, or `None` when TorBox holds no
    /// item with that id.
    pub async fn item(
        &self,
        kind: TorboxItemKind,
        id: i64,
        bypass_cache: bool,
    ) -> Result<Option<TorboxItem>, TorboxError> {
        let operation = TorboxOperation::List(kind);
        let params = [
            ("bypass_cache", bool_text(bypass_cache).to_string()),
            ("id", id.to_string()),
        ];
        let request = self.authorized(self.http.get(self.url(kind.list_path())).query(&params));
        let answer = match self.execute(operation, request).await {
            Ok(answer) => answer,
            Err(error) if error.is_item_not_found() => return Ok(None),
            Err(error) => return Err(error),
        };
        match answer.data {
            Value::Null => Ok(None),
            item @ Value::Object(_) => decode_value(operation, answer.status, item).map(Some),
            // Documented as a single object for `id=`; a list is searched
            // for the id in case the filter was not applied.
            Value::Array(items) => {
                for item in items {
                    let item: TorboxItem = decode_value(operation, answer.status, item)?;
                    if item.id == id {
                        return Ok(Some(item));
                    }
                }
                Ok(None)
            }
            _ => Err(TorboxError::Decode {
                operation,
                status: answer.status,
                message: "the item is neither an object nor a list".to_string(),
            }),
        }
    }

    /// `GET /{kind}/requestdl`: a signed link to one file or the whole item.
    pub async fn request_download(
        &self,
        kind: TorboxItemKind,
        item_id: i64,
        target: TorboxDownloadTarget,
    ) -> Result<TorboxDownloadUrl, TorboxError> {
        let operation = TorboxOperation::RequestDownload(kind);
        // This endpoint takes the token in the query string, not a header.
        let mut params = vec![
            ("token", self.token.0.clone()),
            (kind.id_param(), item_id.to_string()),
        ];
        match target {
            TorboxDownloadTarget::File(file_id) => {
                params.push(("file_id", file_id.to_string()));
                params.push(("zip_link", "false".to_string()));
            }
            TorboxDownloadTarget::Zip => params.push(("zip_link", "true".to_string())),
        }
        params.push(("redirect", "false".to_string()));
        let request = self
            .http
            .get(self.url(kind.request_download_path()))
            .query(&params);
        let answer = self.execute(operation, request).await?;
        let link = match answer.data {
            Value::String(link) => link,
            _ => {
                return Err(TorboxError::Decode {
                    operation,
                    status: answer.status,
                    message: "the answer carries no download link".to_string(),
                });
            }
        };
        match reqwest::Url::parse(link.trim()) {
            Ok(url) if self.accepts_link_scheme(url.scheme()) => {
                Ok(TorboxDownloadUrl(url.to_string()))
            }
            _ => Err(TorboxError::Decode {
                operation,
                status: answer.status,
                message: "the download link is not an https URL".to_string(),
            }),
        }
    }

    /// Delete one item. TorBox's bulk form is never sent.
    pub async fn delete(&self, kind: TorboxItemKind, id: i64) -> Result<(), TorboxError> {
        self.control(kind, id, "delete").await
    }

    /// Resume one item.
    pub async fn resume(&self, kind: TorboxItemKind, id: i64) -> Result<(), TorboxError> {
        self.control(kind, id, "resume").await
    }

    /// Pause one usenet download. TorBox offers no pause for torrents.
    pub async fn pause_usenet(&self, id: i64) -> Result<(), TorboxError> {
        self.control(TorboxItemKind::Usenet, id, "pause").await
    }

    /// Reannounce one torrent to its trackers.
    pub async fn reannounce_torrent(&self, id: i64) -> Result<(), TorboxError> {
        self.control(TorboxItemKind::Torrent, id, "reannounce")
            .await
    }

    async fn control(
        &self,
        kind: TorboxItemKind,
        id: i64,
        operation_name: &'static str,
    ) -> Result<(), TorboxError> {
        let operation = TorboxOperation::Control(kind);
        // Exactly the id and the operation: TorBox's `all` flag, which acts
        // on every item of the account, is never part of the body.
        let mut body = Map::new();
        body.insert(kind.id_param().to_string(), Value::from(id));
        body.insert("operation".to_string(), Value::from(operation_name));
        let request = self.authorized(
            self.http
                .post(self.url(kind.control_path()))
                .json(&Value::Object(body)),
        );
        self.execute(operation, request).await.map(|_| ())
    }

    async fn create(
        &self,
        operation: TorboxOperation,
        request: RequestBuilder,
        id_fields: &[&str],
        add_only_if_cached: bool,
    ) -> Result<TorboxCreated, TorboxError> {
        let answer = match self.execute(operation, request).await {
            Ok(answer) => answer,
            // How TorBox declines a cached-only create is undocumented. A
            // refusal with no code on an otherwise successful status is read
            // as that decline; a coded refusal keeps its code.
            Err(TorboxError::Refused {
                status, code: None, ..
            }) if add_only_if_cached && (200..300).contains(&status) => {
                return Err(TorboxError::NotCached { operation });
            }
            Err(error) => return Err(error),
        };
        let data = answer.data.as_object();
        let id = data.and_then(|data| {
            id_fields
                .iter()
                .find_map(|field| data.get(*field).and_then(value_i64))
        });
        let Some(id) = id else {
            // The same undocumented decline may instead be a success with no
            // item in it.
            if add_only_if_cached {
                return Err(TorboxError::NotCached { operation });
            }
            return Err(TorboxError::Decode {
                operation,
                status: answer.status,
                message: "the answer carries no item id".to_string(),
            });
        };
        let field = |name: &str| data.and_then(|data| data.get(name));
        Ok(TorboxCreated {
            id,
            hash: field("hash").and_then(value_string),
            auth_id: field("auth_id").and_then(value_string),
            cached: field("cached").and_then(value_bool),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{TORBOX_API_PREFIX}/{path}", self.base_url)
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        request.bearer_auth(&self.token.0)
    }

    /// Hide the token wherever it could appear in text this client returns.
    fn redact(&self, text: &str) -> String {
        if self.token.0.is_empty() {
            text.to_string()
        } else {
            text.replace(&self.token.0, REDACTED)
        }
    }

    async fn execute(
        &self,
        operation: TorboxOperation,
        request: RequestBuilder,
    ) -> Result<Answer, TorboxError> {
        let response = request
            .timeout(TORBOX_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| self.transport_error(operation, error))?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(scryer_outbound_http::parse_retry_after)
            .map(|(delay, _)| delay)
            .filter(|delay| !delay.is_zero());
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(TorboxError::RateLimited {
                operation,
                retry_after,
            });
        }
        let status = status.as_u16();
        let body = self.read_answer(operation, status, response).await?;
        let envelope = serde_json::from_slice::<Envelope>(&body).ok();
        match envelope {
            Some(envelope) if (200..300).contains(&status) && envelope.success == Some(true) => {
                Ok(Answer {
                    status,
                    data: envelope.data,
                })
            }
            Some(envelope) if envelope.success.is_some() || envelope.error.is_some() => {
                Err(TorboxError::Refused {
                    operation,
                    status,
                    code: envelope.error.as_deref().map(TorboxErrorCode::parse),
                    detail: envelope.detail.map(|detail| self.redact(&detail)),
                    retry_after,
                })
            }
            _ if (200..300).contains(&status) => Err(TorboxError::Decode {
                operation,
                status,
                message: "the answer is not TorBox's response envelope".to_string(),
            }),
            _ => Err(TorboxError::Refused {
                operation,
                status,
                code: None,
                detail: None,
                retry_after,
            }),
        }
    }

    /// The answer's body, read up to the cap. A body over the cap is not
    /// read further and is an unreadable answer.
    async fn read_answer(
        &self,
        operation: TorboxOperation,
        status: u16,
        mut response: reqwest::Response,
    ) -> Result<Vec<u8>, TorboxError> {
        let max = self.max_answer_bytes;
        let too_large = || TorboxError::Decode {
            operation,
            status,
            message: format!("the answer is larger than {max} bytes"),
        };
        if response
            .content_length()
            .is_some_and(|length| length > max as u64)
        {
            return Err(too_large());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| self.transport_error(operation, error))?
        {
            if body.len().saturating_add(chunk.len()) > max {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    fn transport_error(&self, operation: TorboxOperation, error: reqwest::Error) -> TorboxError {
        let kind = if error.is_builder() || error.is_connect() {
            TorboxTransportKind::NotSent
        } else if error.is_timeout() {
            TorboxTransportKind::TimedOut
        } else {
            TorboxTransportKind::Interrupted
        };
        // The URL can carry the token (`requestdl`), so it is dropped before
        // the error is put into words.
        let message = self.redact(&error.without_url().to_string());
        TorboxError::Transport {
            operation,
            kind,
            message,
        }
    }
}

fn file_part(
    operation: TorboxOperation,
    bytes: Vec<u8>,
    file_name: String,
    mime: &str,
) -> Result<multipart::Part, TorboxError> {
    multipart::Part::bytes(bytes)
        .file_name(file_name)
        .mime_str(mime)
        .map_err(|error| TorboxError::Transport {
            operation,
            kind: TorboxTransportKind::NotSent,
            message: format!("the upload could not be built: {}", error.without_url()),
        })
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn decode_value<T: DeserializeOwned>(
    operation: TorboxOperation,
    status: u16,
    value: Value,
) -> Result<T, TorboxError> {
    // serde's message can quote the offending value, so only its category
    // is reported.
    serde_json::from_value(value).map_err(|error| TorboxError::Decode {
        operation,
        status,
        message: format!("{:?} error in the answer's data", error.classify()),
    })
}

fn value_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|float| float.is_finite())
                .map(|float| float as i64)
        }),
        Value::String(text) => {
            let text = text.trim();
            text.parse::<i64>().ok().or_else(|| {
                text.parse::<f64>()
                    .ok()
                    .filter(|float| float.is_finite())
                    .map(|float| float as i64)
            })
        }
        _ => None,
    }
}

fn value_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|float| float.is_finite())
}

fn value_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::Number(number) => number.as_i64().map(|number| number != 0),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn lenient_i64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    Ok(value_i64(&Value::deserialize(deserializer)?))
}

fn required_i64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    value_i64(&Value::deserialize(deserializer)?)
        .ok_or_else(|| D::Error::custom("the id is not a number"))
}

fn lenient_f64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    Ok(value_f64(&Value::deserialize(deserializer)?))
}

fn lenient_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    Ok(value_bool(&Value::deserialize(deserializer)?))
}

fn lenient_flag<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(value_bool(&Value::deserialize(deserializer)?).unwrap_or(false))
}

fn lenient_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(value_string(&Value::deserialize(deserializer)?))
}

fn lenient_plan<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<TorboxPlan>, D::Error> {
    Ok(value_i64(&Value::deserialize(deserializer)?).map(TorboxPlan::from_number))
}

fn lenient_files<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<TorboxFile>, D::Error> {
    match Value::deserialize(deserializer)? {
        Value::Array(files) => files
            .into_iter()
            .map(|file| serde_json::from_value(file).map_err(D::Error::custom))
            .collect(),
        _ => Ok(Vec::new()),
    }
}
