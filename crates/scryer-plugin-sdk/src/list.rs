//! List-provider plugin family.
//!
//! A list provider turns an external list — a chart, a curated list, a
//! member's watchlist or status list — into ranked items carrying external
//! ids. The host owns everything around that: scheduling, credential storage
//! and refresh, identity resolution through the metadata gateway, routing,
//! requests and persistence. A plugin is stateless and sees at most one member
//! credential per call, carried inside the request and never written to the
//! plugin's config storage.
//!
//! The descriptor doubles as the provider browser's manifest: groups of
//! followable items with their parameters, the auth each needs, caveat notes,
//! and URL patterns the host matches without invoking the plugin.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ConfigFieldDef;

/// Media kinds a list, or one item of it, may contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListMediaKind {
    Movie,
    Series,
    Anime,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListProviderDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blurb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<ListProviderTile>,
    /// Template for a link back to a followed list, with `{param}` placeholders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand_url_template: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<ListMediaKind>,
    #[serde(default)]
    pub auth: ListProviderAuth,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<ListProviderGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<ListProviderNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub url_patterns: Vec<ListUrlPattern>,
    #[serde(default)]
    pub capabilities: ListProviderCapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_seconds: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListProviderCapabilities {
    /// The plugin answers the account operation for a member credential.
    #[serde(default)]
    pub account: bool,
    /// The plugin answers the health operation for its server-wide config.
    #[serde(default)]
    pub health: bool,
    /// Every fetch must carry a member credential. The host refuses to invoke
    /// fetch without one, which is how member-only providers stay member-only.
    #[serde(default)]
    pub requires_member_credential: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListProviderTile {
    pub bg: String,
    pub ink: String,
    pub abbr: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ListProviderAuth {
    #[default]
    None,
    ServerApiKey {
        config_field: String,
    },
    MemberAccount {
        flow: ListAccountFlow,
        exchange: ListAccountExchange,
        #[serde(default)]
        byo_app: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        scopes: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ListAccountFlow {
    AuthorizationCode {
        #[serde(default)]
        pkce: bool,
    },
    Pin,
    PlexPin,
    TmdbApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListAccountExchange {
    Direct,
    SmgRelay,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListProviderGroup {
    pub label: String,
    pub auth_badge: ListAuthBadge,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ListProviderItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListAuthBadge {
    NoAccount,
    NoAccountNeedsValue,
    MemberAccount,
    ServerApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListProviderItem {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<ListMediaKind>,
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ListSourceParam>,
    #[serde(default)]
    pub personal: bool,
    pub default_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListSourceParam {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub param_type: ListSourceParamType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListSourceParamType {
    Text,
    Url,
    Enum,
    Season,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListProviderNote {
    pub tone: ListNoteTone,
    pub text_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ListNoteTone {
    Info,
    Warn,
    Bad,
}

/// A URL the host recognises as one of this provider's sources without
/// invoking the plugin. `pattern` is a Rust `regex` expression; each capture
/// maps a named group to a source parameter.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListUrlPattern {
    pub pattern: String,
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<ListUrlPatternCapture>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListUrlPatternCapture {
    pub group: String,
    pub param: String,
}

/// One member's credential for one call. The host decrypts it from its own
/// storage for the duration of the invocation; it never enters plugin config.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListCredential {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

impl std::fmt::Debug for ListCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListCredential")
            .field("access_token", &"<redacted>")
            .field("token_type", &self.token_type)
            .field("external_user_id", &self.external_user_id)
            .field("username", &self.username)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListPluginFetchRequest {
    pub source_type: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    #[serde(default)]
    pub credential: Option<ListCredential>,
    #[serde(default)]
    pub page_cursor: Option<String>,
    #[serde(default)]
    pub since_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListPluginFetchResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ListPluginItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_hint: Option<u32>,
    /// A fingerprint of the list's current content, echoed back as
    /// `since_fingerprint` on the next fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The list has not changed since `since_fingerprint`; `items` is empty.
    #[serde(default)]
    pub unchanged: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ListPluginItem {
    /// Provider-native id, stable across syncs.
    pub item_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_hint: Option<ListMediaKind>,
    /// A last-resort hint for providers that carry no ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_ids: Vec<ListExternalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_rating: Option<ListProviderRating>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ListExternalId {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ListProviderRating {
    pub scale: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListPluginAccountRequest {
    pub credential: ListCredential,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListPluginAccountResponse {
    pub external_user_id: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owned_lists: Vec<ListAccountList>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<ListAccountStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListAccountList {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<ListMediaKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListAccountStatus {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<ListMediaKind>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListPluginHealthRequest {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListPluginHealthResponse {
    pub healthy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_descriptor() -> ListProviderDescriptor {
        ListProviderDescriptor {
            provider_type: "fixture-lists".to_string(),
            provider_aliases: vec!["fixture".to_string()],
            summary: Some("Fixture list provider".to_string()),
            blurb: None,
            tile: Some(ListProviderTile {
                bg: "#101010".to_string(),
                ink: "#fafafa".to_string(),
                abbr: "FX".to_string(),
            }),
            brand_url_template: Some("https://lists.fixture.invalid/{list_id}".to_string()),
            coverage: vec![ListMediaKind::Movie, ListMediaKind::Series],
            auth: ListProviderAuth::MemberAccount {
                flow: ListAccountFlow::AuthorizationCode { pkce: true },
                exchange: ListAccountExchange::SmgRelay,
                byo_app: true,
                scopes: vec!["read".to_string()],
            },
            groups: vec![ListProviderGroup {
                label: "Public lists".to_string(),
                auth_badge: ListAuthBadge::NoAccountNeedsValue,
                items: vec![ListProviderItem {
                    id: "user-list".to_string(),
                    name: "Any public list".to_string(),
                    description: None,
                    kinds: vec![ListMediaKind::Movie],
                    source_type: "user_list".to_string(),
                    params: vec![ListSourceParam {
                        key: "list_id".to_string(),
                        label: "List".to_string(),
                        param_type: ListSourceParamType::Url,
                        options: Vec::new(),
                        required: true,
                    }],
                    personal: false,
                    default_interval_seconds: 12 * 3600,
                }],
            }],
            notes: vec![ListProviderNote {
                tone: ListNoteTone::Info,
                text_key: "lists.note.fixture".to_string(),
            }],
            url_patterns: vec![ListUrlPattern {
                pattern: r"^https://lists\.fixture\.invalid/(?P<list>[a-z0-9-]+)$".to_string(),
                source_type: "user_list".to_string(),
                captures: vec![ListUrlPatternCapture {
                    group: "list".to_string(),
                    param: "list_id".to_string(),
                }],
            }],
            capabilities: ListProviderCapabilities {
                account: true,
                health: false,
                requires_member_credential: false,
            },
            config_fields: Vec::new(),
            default_base_url: None,
            allowed_hosts: vec!["lists.fixture.invalid".to_string()],
            rate_limit_seconds: Some(1),
        }
    }

    #[test]
    fn list_provider_descriptor_round_trips_through_the_plugin_descriptor() {
        let descriptor = crate::PluginDescriptor {
            id: "fixture-lists".to_string(),
            name: "Fixture Lists".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: crate::SDK_VERSION.to_string(),
            sdk_constraint: crate::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider: crate::ProviderDescriptor::ListProvider(fixture_descriptor()),
        };
        let json = serde_json::to_value(&descriptor).expect("serialize descriptor");
        assert_eq!(json["provider"]["kind"], "list_provider");
        assert_eq!(json["provider"]["auth"]["type"], "member_account");
        assert_eq!(
            json["provider"]["auth"]["flow"]["type"],
            "authorization_code"
        );
        assert_eq!(
            json["provider"]["groups"][0]["items"][0]["params"][0]["type"],
            "url"
        );

        let decoded: crate::PluginDescriptor =
            serde_json::from_value(json).expect("deserialize descriptor");
        assert_eq!(decoded.kind(), crate::PluginKind::ListProvider);
        assert_eq!(decoded.plugin_type(), "list_provider");
        assert_eq!(decoded.provider_type(), "fixture-lists");
        assert_eq!(decoded.allowed_hosts(), ["lists.fixture.invalid"]);
        let provider = decoded.list_provider().expect("list provider descriptor");
        assert_eq!(provider.groups[0].items[0].default_interval_seconds, 43_200);
        assert!(provider.capabilities.account);
    }

    #[test]
    fn a_minimal_descriptor_defaults_to_no_auth() {
        let descriptor: ListProviderDescriptor =
            serde_json::from_str(r#"{"provider_type":"fixture-lists"}"#)
                .expect("minimal descriptor");
        assert_eq!(descriptor.auth, ListProviderAuth::None);
        assert!(!descriptor.capabilities.requires_member_credential);
        assert!(descriptor.groups.is_empty());
    }

    #[test]
    fn list_plugin_kind_parses_from_its_wire_name() {
        let kind: crate::PluginKind =
            serde_json::from_str("\"list_provider\"").expect("parse plugin kind");
        assert_eq!(kind, crate::PluginKind::ListProvider);
        assert_eq!(kind.as_str(), "list_provider");
    }

    #[test]
    fn a_credential_never_prints_its_token() {
        let credential = ListCredential {
            access_token: "fixture-secret-token".to_string(),
            token_type: Some("Bearer".to_string()),
            external_user_id: None,
            username: Some("fixture-member".to_string()),
        };
        let printed = format!("{credential:?}");
        assert!(!printed.contains("fixture-secret-token"), "{printed}");
        assert!(printed.contains("fixture-member"));
    }

    #[test]
    fn fetch_command_round_trips_with_its_credential() {
        use crate::command::{
            PluginCommand, PluginCommandRequest, PluginCommandResponse, PluginCommandResult,
            PluginListCommand, PluginListCommandResult,
        };

        let request = PluginCommandRequest::new(PluginCommand::List(PluginListCommand::Fetch(
            ListPluginFetchRequest {
                source_type: "watchlist".to_string(),
                params: BTreeMap::from([("list_id".to_string(), "fixture-list".to_string())]),
                credential: Some(ListCredential {
                    access_token: "fixture-token".to_string(),
                    token_type: None,
                    external_user_id: Some("member-1".to_string()),
                    username: None,
                }),
                page_cursor: None,
                since_fingerprint: Some("fp-1".to_string()),
            },
        )));
        let value = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(value["command"]["family"], "list");
        assert_eq!(value["command"]["command"]["operation"], "fetch");
        let decoded: PluginCommandRequest =
            serde_json::from_value(value).expect("deserialize request");
        let PluginCommand::List(PluginListCommand::Fetch(fetch)) = decoded.command else {
            panic!("expected a list fetch command");
        };
        assert_eq!(fetch.params["list_id"], "fixture-list");
        assert_eq!(
            fetch.credential.expect("credential").access_token,
            "fixture-token"
        );

        let response = PluginCommandResponse::new(PluginCommandResult::List(
            PluginListCommandResult::Fetch(crate::PluginResult::Ok(ListPluginFetchResponse {
                items: vec![ListPluginItem {
                    item_key: "item-1".to_string(),
                    rank: Some(1),
                    kind_hint: Some(ListMediaKind::Movie),
                    external_ids: vec![ListExternalId {
                        source: "tmdb".to_string(),
                        kind: Some("movie".to_string()),
                        id: "100".to_string(),
                    }],
                    ..ListPluginItem::default()
                }],
                fingerprint: Some("fp-2".to_string()),
                ..ListPluginFetchResponse::default()
            })),
        ));
        let json = serde_json::to_string(&response).expect("serialize response");
        let decoded: PluginCommandResponse =
            serde_json::from_str(&json).expect("deserialize response");
        let PluginCommandResult::List(PluginListCommandResult::Fetch(crate::PluginResult::Ok(
            fetch,
        ))) = decoded.response
        else {
            panic!("expected a list fetch result");
        };
        assert_eq!(fetch.items.len(), 1);
        assert_eq!(fetch.items[0].external_ids[0].id, "100");
        assert!(!fetch.unchanged);
    }
}
