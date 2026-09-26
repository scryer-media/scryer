//! The list-provider plugin port.
//!
//! A list provider is a WASM plugin that reads one external list (a watchlist,
//! a curated list, a status list) and returns its items with their external
//! ids. The host owns everything else: scheduling, resolution, routing,
//! requests, persistence and the credential. The application talks to the
//! plugins through these two traits; `scryer-plugins` implements them.
//!
//! Each call answers in two layers. The outer `AppResult` fails when the host
//! could not complete the exchange at all (the component would not start, the
//! response was not a document). The inner `PluginResult` carries a failure
//! the provider itself classified (an expired token, a missing list, a rate
//! limit) with the code the sync engine maps to a plain-words message and a
//! pause.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use scryer_plugin_sdk::{
    ListCredential, ListPluginAccountResponse, ListPluginFetchRequest, ListPluginFetchResponse,
    ListPluginHealthResponse, PluginDescriptor, PluginResult,
};

use crate::{AppResult, RuntimePluginLoad};

/// One list provider bound to its server-wide configuration.
#[async_trait]
pub trait ListProviderClient: Send + Sync {
    /// The provider's descriptor, as the host loaded it.
    fn descriptor(&self) -> &PluginDescriptor;

    /// Read one page of one list. A member credential travels inside the
    /// request for this call only.
    async fn fetch(
        &self,
        request: ListPluginFetchRequest,
    ) -> AppResult<PluginResult<ListPluginFetchResponse>>;

    /// Read the identity behind a member credential, and the lists and
    /// statuses that member can follow.
    async fn account(
        &self,
        credential: ListCredential,
    ) -> AppResult<PluginResult<ListPluginAccountResponse>>;

    /// Check the server-wide configuration, such as an instance API key.
    async fn health(&self) -> AppResult<PluginResult<ListPluginHealthResponse>>;
}

/// The installed list providers.
pub trait ListPluginProvider: Send + Sync {
    /// A client for `provider_type` (or one of its aliases) with the
    /// server-wide `config` values. `None` when no such provider is installed
    /// or the artifact could not be prepared.
    fn client_for_provider(
        &self,
        provider_type: &str,
        config: &BTreeMap<String, String>,
    ) -> Option<Arc<dyn ListProviderClient>>;

    /// Every installed provider's descriptor, sorted by provider type.
    fn descriptors(&self) -> Vec<PluginDescriptor>;

    fn available_provider_types(&self) -> Vec<String>;

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support runtime-load upsert".to_string())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support runtime-load removal".to_string())
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (runtime_plugins, disabled_builtins);
        Err("this provider does not support runtime-load reload".to_string())
    }
}

/// The provider an assembly without list plugins uses: nothing is installed.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullListPluginProvider;

impl ListPluginProvider for NullListPluginProvider {
    fn client_for_provider(
        &self,
        _provider_type: &str,
        _config: &BTreeMap<String, String>,
    ) -> Option<Arc<dyn ListProviderClient>> {
        None
    }

    fn descriptors(&self) -> Vec<PluginDescriptor> {
        Vec::new()
    }

    fn available_provider_types(&self) -> Vec<String> {
        Vec::new()
    }

    fn reload_runtime_plugins(
        &self,
        _runtime_plugins: &[RuntimePluginLoad],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Ok(())
    }
}
