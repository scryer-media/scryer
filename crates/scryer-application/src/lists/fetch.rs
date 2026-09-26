//! Fetch: one subscription's source → the items it lists right now.
//!
//! Three origins exist. A chart the metadata gateway already ingests, and a
//! public IMDb list the gateway proxies, are read from the gateway; everything
//! else is a list-provider plugin call, paged until the provider stops handing
//! back a cursor. All return the same item shape, so resolve and evaluate
//! never know which one ran.
//!
//! Every failure here is classified into a [`ListFailure`]: the sync records
//! it in plain words on the subscription and, as the rule this module exists
//! to serve, never goes on to the departure step. An empty or unreadable list
//! is not evidence that anything left it.

use std::collections::{BTreeMap, HashMap, HashSet};

use async_trait::async_trait;
use scryer_domain::{LIST_SOURCE_IMDB_LIST_ID_PARAM, ListSourceOrigin, ListSubscription};
use scryer_plugin_sdk::{
    ListCredential, ListPluginFetchRequest, ListPluginItem, PluginError, PluginErrorCode,
    PluginResult,
};

use super::gateway::{ChartItemKey, ListChartItem, chart_items_to_plugin_items};
use super::plugin::ListPluginProvider;
use crate::AppError;

/// A provider paging past this many pages is treated as a runaway cursor.
pub(crate) const MAX_FETCH_PAGES: usize = 100;

/// Why a fetch or resolve did not produce a usable list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListFailureClass {
    /// The provider no longer accepts the member's linked account.
    Unauthorized,
    /// The list does not exist or is private.
    NotFound,
    /// The provider asked us to slow down.
    RateLimited { retry_after_seconds: Option<u64> },
    /// The provider, the gateway, or the plugin host is unavailable.
    Unavailable,
    /// No installed plugin serves this provider.
    NotInstalled,
    /// A member-only source has no usable linked account.
    AccountRequired,
    /// Anything else: a malformed response, an invalid configuration.
    Failed,
}

impl ListFailureClass {
    /// A short label for job summaries and logs. Carries no list or item names.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::NotFound => "not_found",
            Self::RateLimited { .. } => "rate_limited",
            Self::Unavailable => "unavailable",
            Self::NotInstalled => "not_installed",
            Self::AccountRequired => "account_required",
            Self::Failed => "failed",
        }
    }
}

/// A classified failure with the sentence shown on the subscription.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListFailure {
    pub class: ListFailureClass,
    pub message: String,
}

impl ListFailure {
    pub fn new(class: ListFailureClass, provider: &str) -> Self {
        let message = match &class {
            ListFailureClass::Unauthorized => {
                format!("{provider} no longer accepts the linked account; reconnect it")
            }
            ListFailureClass::NotFound => "The list no longer exists or is private.".to_string(),
            ListFailureClass::RateLimited { .. } => format!(
                "{provider} returned 429 Too Many Requests. Scryer will retry at the next interval. Titles already added are unaffected."
            ),
            ListFailureClass::Unavailable => format!(
                "{provider} could not be reached. Scryer will retry at the next interval. Titles already added are unaffected."
            ),
            ListFailureClass::NotInstalled => {
                format!("No installed plugin provides {provider} lists.")
            }
            ListFailureClass::AccountRequired => {
                format!("This list needs a linked {provider} account.")
            }
            ListFailureClass::Failed => format!(
                "{provider} returned a list Scryer could not read. Titles already added are unaffected."
            ),
        };
        Self { class, message }
    }

    /// Classify a provider-reported plugin error.
    pub fn from_plugin_error(error: &PluginError, provider: &str) -> Self {
        let class = match error.code {
            PluginErrorCode::AuthFailed => ListFailureClass::Unauthorized,
            PluginErrorCode::RateLimited => ListFailureClass::RateLimited {
                retry_after_seconds: error
                    .retry_after_seconds
                    .and_then(|seconds| u64::try_from(seconds).ok()),
            },
            PluginErrorCode::UpstreamUnavailable | PluginErrorCode::Temporary => {
                ListFailureClass::Unavailable
            }
            PluginErrorCode::Permanent if is_not_found(error) => ListFailureClass::NotFound,
            _ => ListFailureClass::Failed,
        };
        Self::new(class, provider)
    }

    /// Classify a host-side failure (the exchange itself did not complete).
    pub fn from_app_error(error: &AppError, provider: &str) -> Self {
        let class = match error {
            AppError::Validation(_) => ListFailureClass::AccountRequired,
            AppError::NotFound(_) => ListFailureClass::NotFound,
            _ => ListFailureClass::Unavailable,
        };
        Self::new(class, provider)
    }
}

fn is_not_found(error: &PluginError) -> bool {
    let message = error.public_message.to_ascii_lowercase();
    message.contains("404") || message.contains("not found")
}

/// What one fetch produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FetchedList {
    pub items: Vec<ListPluginItem>,
    /// The provider's change marker for the whole list, when it has one.
    pub fingerprint: Option<String>,
    /// The provider said nothing changed since the stored fingerprint.
    pub unchanged: bool,
    /// Poster art the source supplied, by item key. Shown in previews only.
    pub posters: HashMap<String, String>,
}

impl FetchedList {
    /// Keep each item key's first appearance only. A list naming the same
    /// item twice is one item, and it must be acted on once.
    pub fn dedupe(&mut self) {
        let mut seen = HashSet::new();
        self.items.retain(|item| seen.insert(item.item_key.clone()));
    }

    fn from_gateway(items: Vec<ListChartItem>, key: ChartItemKey) -> Self {
        let mut fetched = Self::default();
        for (item, poster) in chart_items_to_plugin_items(items, key) {
            if let Some(poster) = poster {
                fetched.posters.insert(item.item_key.clone(), poster);
            }
            fetched.items.push(item);
        }
        fetched
    }
}

/// Lists the metadata gateway serves: ingested charts, read as resolved items
/// in chart order, and proxied public IMDb lists.
#[async_trait]
pub trait ListChartSource: Send + Sync {
    async fn chart_items(
        &self,
        provider: &str,
        chart_key: &str,
        scope: &str,
    ) -> Result<Vec<ListChartItem>, ListFailure>;

    async fn imdb_user_list(&self, list_id: &str) -> Result<Vec<ListChartItem>, ListFailure>;
}

/// Read every item of one subscription's source.
///
/// `credential` is the owner's credential for a personal source and `None`
/// otherwise; `config` is the provider's server-wide configuration.
pub async fn fetch_list(
    subscription: &ListSubscription,
    plugins: &dyn ListPluginProvider,
    charts: &dyn ListChartSource,
    credential: Option<ListCredential>,
    config: &BTreeMap<String, String>,
) -> Result<FetchedList, ListFailure> {
    let provider = subscription.source.provider.as_str();
    match &subscription.source.origin {
        ListSourceOrigin::SmgChart { chart_key, scope } => {
            let items = charts.chart_items(provider, chart_key, scope).await?;
            Ok(FetchedList::from_gateway(items, ChartItemKey::GatewayTitle))
        }
        ListSourceOrigin::SmgImdbList => {
            let list_id = subscription
                .source
                .params
                .get(LIST_SOURCE_IMDB_LIST_ID_PARAM)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ListFailure::new(ListFailureClass::NotFound, provider))?;
            let items = charts.imdb_user_list(list_id).await?;
            Ok(FetchedList::from_gateway(items, ChartItemKey::Imdb))
        }
        ListSourceOrigin::ProviderFetch => {
            let client = plugins
                .client_for_provider(provider, config)
                .ok_or_else(|| ListFailure::new(ListFailureClass::NotInstalled, provider))?;
            let mut fetched = FetchedList::default();
            let mut cursor = None;
            for page in 0..MAX_FETCH_PAGES {
                let request = ListPluginFetchRequest {
                    source_type: subscription.source.source_type.clone(),
                    params: subscription.source.params.clone(),
                    credential: credential.clone(),
                    page_cursor: cursor.take(),
                    // Only the first page can short-circuit the whole list.
                    since_fingerprint: if page == 0 {
                        subscription.sync.fetch_fingerprint.clone()
                    } else {
                        None
                    },
                };
                let response = match client.fetch(request).await {
                    Ok(PluginResult::Ok(response)) => response,
                    Ok(PluginResult::Err(error)) => {
                        return Err(ListFailure::from_plugin_error(&error, provider));
                    }
                    Err(error) => return Err(ListFailure::from_app_error(&error, provider)),
                };
                if page == 0 {
                    fetched.fingerprint = response.fingerprint.clone();
                    if response.unchanged {
                        fetched.unchanged = true;
                        return Ok(fetched);
                    }
                }
                fetched.items.extend(response.items);
                match response.next_cursor.filter(|next| !next.is_empty()) {
                    Some(next) => cursor = Some(next),
                    None => return Ok(fetched),
                }
            }
            Err(ListFailure::new(ListFailureClass::Failed, provider))
        }
    }
}
