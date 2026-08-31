#[cfg(test)]
use super::catalog::{CATALOG_V3_RUNTIME_WASIP1, ChildCatalog, ChildCatalogRelease};
use super::catalog::{
    CatalogV3, CatalogV3CommunitySource, CatalogV3DistributionArtifact, CatalogV3PluginArtifact,
    CatalogV3PluginEntry, CatalogV3PluginRelease, CatalogV3Redirect, CatalogV3RulePackEntry,
    CatalogV3RulePackRelease, GitHubRepo, MANUAL_PLUGIN_WASM_OUTPUT_LIMIT,
    PLUGIN_CATALOG_JSON_OUTPUT_LIMIT, PLUGIN_CATALOG_REDIRECT_OUTPUT_LIMIT,
    PLUGIN_SIGNATURE_BUNDLE_OUTPUT_LIMIT, PluginLifecycleStatus,
    RULE_PACK_MANIFEST_FALLBACK_OUTPUT_LIMIT, RequiredSigner, artifact_encoding_from_url,
    blake3_digest, bound_uncompressed_bytes, catalog_v3_runtime_is_supported, compress_zstd,
    decompress_brotli, decompress_zstd, parse_and_validate_catalog_v3,
    parse_and_validate_catalog_v3_redirect, parse_digest_string, redirect_bundle_url_for,
    verify_digest_set, verify_signed_blob, verify_split_digest,
};
use super::*;
use crate::ProviderCatalogFamily;
use crate::RateLimitCooldownAction;
use crate::ports::RuntimePluginLoad;
use base64::Engine as _;
use chrono::Utc;
use scryer_domain::{
    PersistedPluginWasmPayload, PluginSourceKind, PluginSupportTier, PluginWasmEncoding,
};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy,
    no_redirect_reqwest_client,
};
use scryer_plugin_sdk::{
    PluginDescriptor, ProviderDescriptor, SDK_VERSION, effective_host_sdk_constraint,
    host_version_matches_constraint, plugin_descriptor_sdk_constraint, sdk_constraint_or_legacy,
    validate_plugin_descriptor_host_permissions, validate_plugin_descriptor_sdk_contract,
    validate_sdk_contract,
};
use serde::{Deserialize, Serialize};
use std::{sync::LazyLock, time::Duration};
use tracing::{debug, warn};

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("types.rs");
include!("http.rs");
include!("progress.rs");
include!("runtime_registry.rs");
include!("builtins.rs");
include!("availability.rs");
include!("catalog_status.rs");
include!("catalog_fetch.rs");
include!("verification.rs");
include!("install.rs");
include!("manual_install.rs");
include!("restore.rs");
include!("auto_update.rs");
include!("tests.rs");
