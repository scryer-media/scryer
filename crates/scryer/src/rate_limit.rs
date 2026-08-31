use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_graphql::{
    BatchRequest, BatchResponse, ErrorExtensionValues, Response, ServerError, Value,
    parser::{
        parse_query,
        types::{ExecutableDocument, Field, OperationType, Selection, SelectionSet},
    },
};
use governor::clock::Clock;
use governor::{DefaultKeyedRateLimiter, Quota};

#[derive(Clone)]
pub(crate) struct ScryerRateLimiter {
    inner: Arc<RateLimitBuckets>,
}

struct RateLimitBuckets {
    bypass: Vec<IpMatcher>,
    login: Bucket,
    auth_start: Bucket,
    auth_peer: Bucket,
    principal_failures: PrincipalFailureBucket,
    search: Bucket,
    mutation: Bucket,
    api: Bucket,
    authless_client: Bucket,
    oauth: Bucket,
}

struct Bucket {
    limiter: DefaultKeyedRateLimiter<String>,
}

struct PrincipalFailureBucket {
    requests: usize,
    window: Duration,
    failures: Mutex<HashMap<String, VecDeque<Instant>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GraphqlRateLimitClass {
    Login,
    AuthStart,
    Search,
    Mutation,
    Api,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HttpRateLimitClass {
    Api,
    AuthlessClient,
    OAuth,
}

#[derive(Clone, Debug)]
pub(crate) struct RateLimitKey {
    value: String,
    peer_value: String,
    peer_ip: IpAddr,
}

impl RateLimitKey {
    #[cfg(test)]
    pub(crate) fn new(client_ip: IpAddr, user_id: Option<&str>) -> Self {
        Self::for_client_and_peer(client_ip, client_ip, user_id)
    }

    pub(crate) fn for_client_and_peer(
        client_ip: IpAddr,
        peer_ip: IpAddr,
        user_id: Option<&str>,
    ) -> Self {
        let value = match user_id {
            Some(user_id) => format!("user:{user_id}:ip:{client_ip}"),
            None => format!("ip:{client_ip}"),
        };
        let peer_value = match user_id {
            Some(user_id) => format!("user:{user_id}:peer:{peer_ip}"),
            None => format!("peer:{peer_ip}"),
        };
        Self {
            value,
            peer_value,
            peer_ip,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RateLimitDecision {
    pub(crate) message: String,
    pub(crate) retry_after: Option<Duration>,
}

#[derive(Clone, Copy)]
enum IpMatcher {
    Exact(IpAddr),
    Cidr(IpAddr, u8),
}

impl ScryerRateLimiter {
    pub(crate) fn from_env() -> Self {
        Self {
            inner: Arc::new(RateLimitBuckets {
                bypass: parse_ip_matchers(
                    std::env::var("SCRYER_RATE_LIMIT_BYPASS_IPS")
                        .unwrap_or_default()
                        .as_str(),
                ),
                login: Bucket::from_env(
                    "SCRYER_LOGIN_RATE_LIMIT_ATTEMPTS",
                    "SCRYER_LOGIN_RATE_LIMIT_WINDOW_SECS",
                    5,
                    60,
                ),
                auth_start: Bucket::from_env(
                    "SCRYER_AUTH_START_RATE_LIMIT_REQUESTS",
                    "SCRYER_AUTH_START_RATE_LIMIT_WINDOW_SECS",
                    30,
                    60,
                ),
                auth_peer: Bucket::from_env(
                    "SCRYER_AUTH_PEER_RATE_LIMIT_REQUESTS",
                    "SCRYER_AUTH_PEER_RATE_LIMIT_WINDOW_SECS",
                    60,
                    60,
                ),
                principal_failures: PrincipalFailureBucket::from_env(
                    "SCRYER_LOGIN_PRINCIPAL_FAILURES",
                    "SCRYER_LOGIN_PRINCIPAL_FAILURE_WINDOW_SECS",
                    20,
                    300,
                ),
                search: Bucket::from_env(
                    "SCRYER_SEARCH_RATE_LIMIT_REQUESTS",
                    "SCRYER_SEARCH_RATE_LIMIT_WINDOW_SECS",
                    30,
                    60,
                ),
                mutation: Bucket::from_env(
                    "SCRYER_MUTATION_RATE_LIMIT_REQUESTS",
                    "SCRYER_MUTATION_RATE_LIMIT_WINDOW_SECS",
                    120,
                    60,
                ),
                api: Bucket::from_env(
                    "SCRYER_API_RATE_LIMIT_REQUESTS",
                    "SCRYER_API_RATE_LIMIT_WINDOW_SECS",
                    300,
                    60,
                ),
                authless_client: Bucket::from_env(
                    "SCRYER_AUTHLESS_CLIENT_RATE_LIMIT_REQUESTS",
                    "SCRYER_AUTHLESS_CLIENT_RATE_LIMIT_WINDOW_SECS",
                    120,
                    60,
                ),
                oauth: Bucket::from_env(
                    "SCRYER_OAUTH_RATE_LIMIT_REQUESTS",
                    "SCRYER_OAUTH_RATE_LIMIT_WINDOW_SECS",
                    30,
                    60,
                ),
            }),
        }
    }

    pub(crate) fn check_graphql(
        &self,
        class: GraphqlRateLimitClass,
        key: &RateLimitKey,
    ) -> Result<(), RateLimitDecision> {
        if self.is_bypassed(key) {
            return Ok(());
        }

        let bucket = match class {
            GraphqlRateLimitClass::Login => &self.inner.login,
            GraphqlRateLimitClass::AuthStart => &self.inner.auth_start,
            GraphqlRateLimitClass::Search => &self.inner.search,
            GraphqlRateLimitClass::Mutation => &self.inner.mutation,
            GraphqlRateLimitClass::Api => &self.inner.api,
        };
        bucket.check(&key.value, class)?;
        if matches!(
            class,
            GraphqlRateLimitClass::Login | GraphqlRateLimitClass::AuthStart
        ) {
            self.inner.auth_peer.check(&key.peer_value, class)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn record_failed_login(&self, key: &RateLimitKey) -> Result<(), RateLimitDecision> {
        self.check_graphql(GraphqlRateLimitClass::Login, key)
    }

    pub(crate) fn check_login_principal(
        &self,
        key: &RateLimitKey,
        principal: &str,
    ) -> Result<(), RateLimitDecision> {
        if self.is_bypassed(key) {
            return Ok(());
        }
        self.inner.principal_failures.check(principal)
    }

    pub(crate) fn record_login_principal_failure(&self, key: &RateLimitKey, principal: &str) {
        if !self.is_bypassed(key) {
            self.inner.principal_failures.record(principal);
        }
    }

    pub(crate) fn clear_login_principal_failures(&self, principal: &str) {
        self.inner.principal_failures.clear(principal);
    }

    pub(crate) fn check_http(
        &self,
        class: HttpRateLimitClass,
        key: &RateLimitKey,
    ) -> Result<(), RateLimitDecision> {
        if self.is_bypassed(key) {
            return Ok(());
        }
        let bucket = match class {
            HttpRateLimitClass::Api => &self.inner.api,
            HttpRateLimitClass::AuthlessClient => &self.inner.authless_client,
            HttpRateLimitClass::OAuth => &self.inner.oauth,
        };
        bucket.check(&key.value, class)
    }

    fn is_bypassed(&self, key: &RateLimitKey) -> bool {
        self.inner
            .bypass
            .iter()
            .any(|matcher| matcher.matches(key.peer_ip))
    }
}

impl Bucket {
    fn from_env(
        requests_env: &str,
        window_env: &str,
        default_requests: u32,
        default_window: u64,
    ) -> Self {
        let requests = read_env_u32(requests_env)
            .unwrap_or(default_requests)
            .max(1);
        let window_secs = read_env_u64(window_env).unwrap_or(default_window).max(1);
        let quota = quota_for_window(requests, Duration::from_secs(window_secs));
        Self {
            limiter: DefaultKeyedRateLimiter::keyed(quota),
        }
    }

    fn check(
        &self,
        key: &str,
        class: impl Into<RateLimitMessageClass>,
    ) -> Result<(), RateLimitDecision> {
        let key = key.to_string();
        let class = class.into();
        self.limiter.check_key(&key).map_err(|blocked| {
            let retry_after = blocked.wait_time_from(self.limiter.clock().now());
            RateLimitDecision {
                message: rate_limited_message(class),
                retry_after: Some(retry_after),
            }
        })
    }
}

impl PrincipalFailureBucket {
    fn from_env(
        requests_env: &str,
        window_env: &str,
        default_requests: u32,
        default_window: u64,
    ) -> Self {
        Self {
            requests: read_env_u32(requests_env)
                .unwrap_or(default_requests)
                .max(1) as usize,
            window: Duration::from_secs(read_env_u64(window_env).unwrap_or(default_window).max(1)),
            failures: Mutex::new(HashMap::new()),
        }
    }

    fn check(&self, principal: &str) -> Result<(), RateLimitDecision> {
        let now = Instant::now();
        let mut failures = self
            .failures
            .lock()
            .expect("principal login failure lock must not be poisoned");
        let entries = failures.entry(principal.to_string()).or_default();
        Self::prune(entries, now, self.window);
        if entries.len() < self.requests {
            return Ok(());
        }
        let retry_after = entries.front().map(|attempt| {
            self.window
                .saturating_sub(now.saturating_duration_since(*attempt))
        });
        Err(RateLimitDecision {
            message: rate_limited_message(GraphqlRateLimitClass::Login),
            retry_after,
        })
    }

    fn record(&self, principal: &str) {
        let now = Instant::now();
        let mut failures = self
            .failures
            .lock()
            .expect("principal login failure lock must not be poisoned");
        let entries = failures.entry(principal.to_string()).or_default();
        Self::prune(entries, now, self.window);
        entries.push_back(now);
    }

    fn clear(&self, principal: &str) {
        self.failures
            .lock()
            .expect("principal login failure lock must not be poisoned")
            .remove(principal);
    }

    fn prune(entries: &mut VecDeque<Instant>, now: Instant, window: Duration) {
        while entries
            .front()
            .is_some_and(|attempt| now.saturating_duration_since(*attempt) >= window)
        {
            entries.pop_front();
        }
    }
}

pub(crate) fn classify_graphql(batch: &BatchRequest) -> GraphqlRateLimitClass {
    let mut class = GraphqlRateLimitClass::Api;
    for request in batch.iter() {
        let next = classify_graphql_request(request);
        class = stricter_class(class, next);
        if class == GraphqlRateLimitClass::Login {
            break;
        }
    }
    class
}

pub(crate) fn classify_graphql_request(request: &async_graphql::Request) -> GraphqlRateLimitClass {
    analyze_authentication_request(request)
        .class
        .unwrap_or_else(|| classify_query(&request.query))
}

#[cfg(test)]
pub(crate) fn should_precheck_graphql_login(batch: &BatchRequest) -> bool {
    batch
        .iter()
        .any(|request| analyze_authentication_request(request).class.is_some())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AuthenticationRequestAnalysis {
    pub(crate) class: Option<GraphqlRateLimitClass>,
    pub(crate) principal: Option<String>,
    pub(crate) rejected: bool,
}

pub(crate) fn analyze_authentication_request(
    request: &async_graphql::Request,
) -> AuthenticationRequestAnalysis {
    let Ok(document) = parse_query(&request.query) else {
        return AuthenticationRequestAnalysis::default();
    };
    let Some(operation) = selected_operation(&document, request.operation_name.as_deref()) else {
        return AuthenticationRequestAnalysis::default();
    };
    if operation.node.ty != OperationType::Mutation {
        return AuthenticationRequestAnalysis::default();
    }
    let mut fields = Vec::new();
    if !collect_top_level_fields(
        &document,
        &operation.node.selection_set.node,
        0,
        &mut fields,
    ) {
        return AuthenticationRequestAnalysis::default();
    }
    let authentication_fields = fields
        .iter()
        .filter_map(|field| {
            authentication_rate_limit_class(field.name.node.as_str()).map(|class| (*field, class))
        })
        .collect::<Vec<_>>();
    if authentication_fields.is_empty() {
        return AuthenticationRequestAnalysis::default();
    }
    if fields.len() != 1 || authentication_fields.len() != 1 {
        return AuthenticationRequestAnalysis {
            rejected: true,
            ..AuthenticationRequestAnalysis::default()
        };
    }
    let (field, class) = authentication_fields[0];
    AuthenticationRequestAnalysis {
        class: Some(class),
        principal: login_principal_for_field(request, field),
        rejected: false,
    }
}

pub(crate) fn rate_limited_graphql_response(decision: &RateLimitDecision) -> BatchResponse {
    BatchResponse::Single(rate_limited_graphql_single_response(decision))
}

pub(crate) fn rate_limited_graphql_single_response(decision: &RateLimitDecision) -> Response {
    let mut extensions = ErrorExtensionValues::default();
    extensions.set("code", "RATE_LIMITED");
    if let Some(retry_after) = decision.retry_after {
        extensions.set("retryAfterSeconds", retry_after.as_secs());
    }

    let mut error = ServerError::new(decision.message.clone(), None);
    error.extensions = Some(extensions);
    Response::from_errors(vec![error])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RateLimitMessageClass {
    Login,
    AuthStart,
    Search,
    Mutation,
    Api,
    AuthlessClient,
    OAuth,
}

impl From<GraphqlRateLimitClass> for RateLimitMessageClass {
    fn from(class: GraphqlRateLimitClass) -> Self {
        match class {
            GraphqlRateLimitClass::Login => Self::Login,
            GraphqlRateLimitClass::AuthStart => Self::AuthStart,
            GraphqlRateLimitClass::Search => Self::Search,
            GraphqlRateLimitClass::Mutation => Self::Mutation,
            GraphqlRateLimitClass::Api => Self::Api,
        }
    }
}

impl From<HttpRateLimitClass> for RateLimitMessageClass {
    fn from(class: HttpRateLimitClass) -> Self {
        match class {
            HttpRateLimitClass::Api => Self::Api,
            HttpRateLimitClass::AuthlessClient => Self::AuthlessClient,
            HttpRateLimitClass::OAuth => Self::OAuth,
        }
    }
}

pub(crate) fn rate_limited_message(class: impl Into<RateLimitMessageClass>) -> String {
    let class = class.into();
    match class {
        RateLimitMessageClass::Login => "too many login attempts".to_string(),
        RateLimitMessageClass::AuthStart => "too many authentication starts".to_string(),
        RateLimitMessageClass::Search => "search rate limit exceeded".to_string(),
        RateLimitMessageClass::Mutation => "mutation rate limit exceeded".to_string(),
        RateLimitMessageClass::Api => "API rate limit exceeded".to_string(),
        RateLimitMessageClass::AuthlessClient => "authless client rate limit exceeded".to_string(),
        RateLimitMessageClass::OAuth => "OAuth rate limit exceeded".to_string(),
    }
}

fn classify_query(query: &str) -> GraphqlRateLimitClass {
    let compact = query.split_whitespace().collect::<String>();
    let document = parse_query(query).ok();
    if document
        .as_ref()
        .is_some_and(document_contains_login_mutation_field)
    {
        return GraphqlRateLimitClass::Login;
    }
    if contains_expensive_field(&compact) {
        return GraphqlRateLimitClass::Search;
    }
    if document.as_ref().map_or_else(
        || compact.to_ascii_lowercase().contains("mutation"),
        document_contains_mutation,
    ) {
        return GraphqlRateLimitClass::Mutation;
    }
    GraphqlRateLimitClass::Api
}

fn document_contains_login_mutation_field(document: &ExecutableDocument) -> bool {
    document.operations.iter().any(|(_, operation)| {
        operation.node.ty == OperationType::Mutation
            && selection_set_contains_login_mutation_field(
                document,
                &operation.node.selection_set.node,
                0,
            )
    })
}

fn document_contains_mutation(document: &ExecutableDocument) -> bool {
    document
        .operations
        .iter()
        .any(|(_, operation)| operation.node.ty == OperationType::Mutation)
}

fn selection_set_contains_login_mutation_field(
    document: &ExecutableDocument,
    selection_set: &SelectionSet,
    depth: usize,
) -> bool {
    if depth > 16 {
        return false;
    }

    selection_set
        .items
        .iter()
        .any(|selection| match &selection.node {
            Selection::Field(field) => is_login_mutation_field(field.node.name.node.as_str()),
            Selection::FragmentSpread(spread) => document
                .fragments
                .get(&spread.node.fragment_name.node)
                .is_some_and(|fragment| {
                    selection_set_contains_login_mutation_field(
                        document,
                        &fragment.node.selection_set.node,
                        depth + 1,
                    )
                }),
            Selection::InlineFragment(fragment) => selection_set_contains_login_mutation_field(
                document,
                &fragment.node.selection_set.node,
                depth + 1,
            ),
        })
}

fn authentication_rate_limit_class(name: &str) -> Option<GraphqlRateLimitClass> {
    if is_authentication_start_mutation_field(name) {
        Some(GraphqlRateLimitClass::AuthStart)
    } else if is_login_mutation_field(name) {
        Some(GraphqlRateLimitClass::Login)
    } else {
        None
    }
}

fn is_authentication_start_mutation_field(name: &str) -> bool {
    matches!(
        name,
        "webauthnAuthenticateStart"
            | "loginVerificationPasskeyStart"
            | "accountSecurityPasskeyStart"
            | "webauthnLoginEnrollmentStart"
    )
}

fn is_login_mutation_field(name: &str) -> bool {
    matches!(
        name,
        "login"
            | "loginWithJellyfin"
            | "loginWithEmby"
            | "loginWithPlex"
            | "completeLoginMfaEnrollment"
            | "webauthnAuthenticateComplete"
            | "loginVerificationPasskeyComplete"
            | "loginVerificationTotpComplete"
            | "webauthnLoginEnrollmentComplete"
            | "completeRequiredPasswordChange"
            | "accountSecurityPasswordVerify"
            | "accountSecurityPasskeyStart"
            | "accountSecurityPasskeyComplete"
            | "mfaVerifyStepUp"
            | "totpDisable"
            | "totpRegenerateRecoveryCodes"
    )
}

fn selected_operation<'a>(
    document: &'a ExecutableDocument,
    operation_name: Option<&str>,
) -> Option<&'a async_graphql::Positioned<async_graphql::parser::types::OperationDefinition>> {
    match operation_name {
        Some(operation_name) => document.operations.iter().find_map(|(name, operation)| {
            (name
                .as_ref()
                .is_some_and(|name| name.as_str() == operation_name))
            .then_some(operation)
        }),
        None => {
            let mut operations = document.operations.iter();
            let (_, operation) = operations.next()?;
            operations.next().is_none().then_some(operation)
        }
    }
}

fn collect_top_level_fields<'a>(
    document: &'a ExecutableDocument,
    selection_set: &'a SelectionSet,
    depth: usize,
    fields: &mut Vec<&'a Field>,
) -> bool {
    if depth > 16 {
        return false;
    }
    for selection in &selection_set.items {
        match &selection.node {
            Selection::Field(field) => fields.push(&field.node),
            Selection::FragmentSpread(spread) => {
                let Some(fragment) = document.fragments.get(&spread.node.fragment_name.node) else {
                    return false;
                };
                if !collect_top_level_fields(
                    document,
                    &fragment.node.selection_set.node,
                    depth + 1,
                    fields,
                ) {
                    return false;
                }
            }
            Selection::InlineFragment(fragment) => {
                if !collect_top_level_fields(
                    document,
                    &fragment.node.selection_set.node,
                    depth + 1,
                    fields,
                ) {
                    return false;
                }
            }
        }
    }
    true
}

fn login_principal_for_field(request: &async_graphql::Request, field: &Field) -> Option<String> {
    let provider = match field.name.node.as_str() {
        "login" => "local",
        "loginWithJellyfin" => "jellyfin",
        "loginWithEmby" => "emby",
        _ => return None,
    };
    let input = field
        .get_argument("input")?
        .node
        .clone()
        .into_const_with(|name| request.variables.get(&name).cloned().ok_or(()))
        .ok()?;
    let Value::Object(input) = input else {
        return None;
    };
    let Value::String(username) = input.get("username")? else {
        return None;
    };
    let username = username.trim();
    if username.is_empty() {
        return None;
    }
    if provider == "local" {
        return Some(format!("local:{username}"));
    }
    let Value::String(connection_id) = input.get("connectionId")? else {
        return None;
    };
    let connection_id = connection_id.trim();
    (!connection_id.is_empty()).then(|| format!("{provider}:{connection_id}:{username}"))
}

fn contains_expensive_field(query: &str) -> bool {
    [
        "searchReleases",
        "searchMetadata",
        "searchMetadataMulti",
        "externalSubtitles",
        "subtitleSearch",
        "subtitleDownload",
        "releaseDecisions",
        "beginManualImportSelection",
        "pendingImportBindingPreview",
        "titleAcquisitionDiagnostics",
    ]
    .iter()
    .any(|field| query.contains(field))
        || query.contains("titles(") && query.contains("query:")
}

fn stricter_class(
    current: GraphqlRateLimitClass,
    next: GraphqlRateLimitClass,
) -> GraphqlRateLimitClass {
    if class_rank(next) < class_rank(current) {
        next
    } else {
        current
    }
}

fn class_rank(class: GraphqlRateLimitClass) -> u8 {
    match class {
        GraphqlRateLimitClass::Login => 0,
        GraphqlRateLimitClass::AuthStart => 1,
        GraphqlRateLimitClass::Search => 2,
        GraphqlRateLimitClass::Mutation => 3,
        GraphqlRateLimitClass::Api => 4,
    }
}

fn quota_for_window(requests: u32, window: Duration) -> Quota {
    let requests = NonZeroU32::new(requests.max(1)).expect("requests are clamped above one");
    let nanos_per_cell = (window.as_nanos() / u128::from(requests.get())).max(1);
    let nanos = u64::try_from(nanos_per_cell).unwrap_or(u64::MAX);
    Quota::with_period(Duration::from_nanos(nanos))
        .expect("non-zero period is guaranteed")
        .allow_burst(requests)
}

fn read_env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.trim().parse().ok()
}

fn read_env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.trim().parse().ok()
}

fn parse_ip_matchers(raw: &str) -> Vec<IpMatcher> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(parse_ip_matcher)
        .collect()
}

fn parse_ip_matcher(raw: &str) -> Option<IpMatcher> {
    let Some((ip, prefix)) = raw.split_once('/') else {
        return raw.parse::<IpAddr>().ok().map(IpMatcher::Exact);
    };
    let ip = ip.trim().parse::<IpAddr>().ok()?;
    let prefix = prefix.trim().parse::<u8>().ok()?;
    match ip {
        IpAddr::V4(_) if prefix <= 32 => Some(IpMatcher::Cidr(ip, prefix)),
        IpAddr::V6(_) if prefix <= 128 => Some(IpMatcher::Cidr(ip, prefix)),
        _ => None,
    }
}

impl IpMatcher {
    fn matches(self, ip: IpAddr) -> bool {
        match self {
            Self::Exact(exact) => exact == ip,
            Self::Cidr(base, prefix) => cidr_contains(base, prefix, ip),
        }
    }
}

fn cidr_contains(base: IpAddr, prefix: u8, ip: IpAddr) -> bool {
    match (base, ip) {
        (IpAddr::V4(base), IpAddr::V4(ip)) => {
            let mask = ipv4_mask(prefix);
            u32::from(base) & mask == u32::from(ip) & mask
        }
        (IpAddr::V6(base), IpAddr::V6(ip)) => {
            let mask = ipv6_mask(prefix);
            u128::from(base) & mask == u128::from(ip) & mask
        }
        _ => false,
    }
}

fn ipv4_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    }
}

fn ipv6_mask(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn cidr_bypass_matches_ipv4_prefix() {
        let matcher = parse_ip_matcher("192.168.1.0/24").expect("valid cidr");
        assert!(matcher.matches(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42))));
        assert!(!matcher.matches(IpAddr::V4(Ipv4Addr::new(192, 168, 2, 42))));
    }

    #[test]
    fn cidr_bypass_matches_ipv6_prefix() {
        let matcher = parse_ip_matcher("fd00::/8").expect("valid cidr");
        assert!(matcher.matches(IpAddr::V6("fd00::1".parse::<Ipv6Addr>().unwrap())));
        assert!(!matcher.matches(IpAddr::V6("fe80::1".parse::<Ipv6Addr>().unwrap())));
    }

    #[test]
    fn login_mutation_is_strictest_graphql_bucket() {
        let batch = BatchRequest::Single(async_graphql::Request::new(
            "mutation Login { login(input: { username: \"a\", password: \"b\" }) { token } }",
        ));
        assert_eq!(classify_graphql(&batch), GraphqlRateLimitClass::Login);
    }

    #[test]
    fn passkey_authentication_starts_use_the_start_bucket() {
        let start_batch = BatchRequest::Single(async_graphql::Request::new(
            "mutation PasskeyStart { webauthnAuthenticateStart(username: \"a\") { challengeId } }",
        ));
        assert_eq!(
            classify_graphql(&start_batch),
            GraphqlRateLimitClass::AuthStart
        );
        assert!(should_precheck_graphql_login(&start_batch));

        let complete_batch = BatchRequest::Single(async_graphql::Request::new(
            "mutation PasskeyComplete { webauthnAuthenticateComplete(input: { challengeId: \"c\", responseJson: \"{}\" }) { token } }",
        ));
        assert_eq!(
            classify_graphql(&complete_batch),
            GraphqlRateLimitClass::Login
        );
        assert!(should_precheck_graphql_login(&complete_batch));
    }

    #[test]
    fn account_security_verification_mutations_use_login_bucket() {
        for mutation in [
            "accountSecurityPasswordVerify(currentPassword: \"password\") { token }",
            "accountSecurityPasskeyComplete(input: { challengeId: \"c\", responseJson: \"{}\" }) { token }",
            "mfaVerifyStepUp(input: { code: \"123456\" }) { token }",
            "totpDisable(input: { code: \"123456\" }) { enabled }",
            "totpRegenerateRecoveryCodes(input: { code: \"123456\" }) { recoveryCodes }",
        ] {
            let batch = BatchRequest::Single(async_graphql::Request::new(format!(
                "mutation SecurityVerification {{ {mutation} }}"
            )));
            assert_eq!(classify_graphql(&batch), GraphqlRateLimitClass::Login);
            assert!(should_precheck_graphql_login(&batch));
        }
    }

    #[test]
    fn authentication_mutations_are_rejected_when_batched_in_one_operation() {
        let aliased = async_graphql::Request::new(
            "mutation { password: accountSecurityPasswordVerify(currentPassword: \"password\") { token } totp: mfaVerifyStepUp(input: { code: \"123456\" }) { token } }",
        );
        assert!(analyze_authentication_request(&aliased).rejected);

        let fragment = async_graphql::Request::new(
            "mutation { ...Authentication } fragment Authentication on Mutation { login(input: { username: \"a\", password: \"b\" }) { token } updateSecuritySettings(input: {}) { formLoginEnabled } }",
        );
        assert!(analyze_authentication_request(&fragment).rejected);
    }

    #[test]
    fn authentication_mutation_analysis_uses_the_selected_operation() {
        let request = async_graphql::Request::new(
            "mutation Authenticate { login(input: { username: \"a\", password: \"b\" }) { token } } mutation Update { updateSecuritySettings(input: {}) { formLoginEnabled } }",
        )
        .operation_name("Update");
        assert_eq!(
            analyze_authentication_request(&request),
            AuthenticationRequestAnalysis::default()
        );
    }

    #[test]
    fn authentication_mutation_analysis_uses_a_single_named_operation_without_an_explicit_name() {
        let request = async_graphql::Request::new(
            "mutation Authenticate { login(input: { username: \"a\", password: \"b\" }) { token } mfaVerifyStepUp(input: { code: \"123456\" }) { token } }",
        );
        assert!(analyze_authentication_request(&request).rejected);
    }

    #[test]
    fn principal_failures_are_cleared_after_a_non_failure_result() {
        let bucket = PrincipalFailureBucket {
            requests: 1,
            window: Duration::from_secs(60),
            failures: Mutex::new(HashMap::new()),
        };
        bucket.record("local:alice");
        assert!(bucket.check("local:alice").is_err());
        bucket.clear("local:alice");
        assert!(bucket.check("local:alice").is_ok());
    }

    #[test]
    fn security_settings_login_named_fields_use_mutation_bucket() {
        let batch = BatchRequest::Single(async_graphql::Request::new(
            r#"mutation UpdateSecuritySettings($input: SecuritySettingsInput!) {
              updateSecuritySettings(input: $input) {
                formLoginEnabled
                mfaRequirePasswordLogin
                totpRequireJellyfinLogin
                effectiveFormLoginEnabled
              }
            }"#,
        ));
        assert_eq!(classify_graphql(&batch), GraphqlRateLimitClass::Mutation);
        assert!(!should_precheck_graphql_login(&batch));
    }

    #[test]
    fn media_server_connection_login_named_fields_use_mutation_bucket() {
        let batch = BatchRequest::Single(async_graphql::Request::new(
            r#"mutation CreateMediaServerConnection($input: CreateMediaServerConnectionInput!) {
              createMediaServerConnection(input: $input) {
                id
                loginEnabled
                lastLoginAt
              }
            }"#,
        ));
        assert_eq!(classify_graphql(&batch), GraphqlRateLimitClass::Mutation);
        assert!(!should_precheck_graphql_login(&batch));
    }

    #[test]
    fn operation_name_login_does_not_make_mutation_a_login_attempt() {
        let batch = BatchRequest::Single(async_graphql::Request::new(
            r#"mutation Login($input: SecuritySettingsInput!) {
              updateSecuritySettings(input: $input) {
                formLoginEnabled
              }
            }"#,
        ));
        assert_eq!(classify_graphql(&batch), GraphqlRateLimitClass::Mutation);
        assert!(!should_precheck_graphql_login(&batch));
    }

    #[test]
    fn query_title_search_uses_search_bucket() {
        let batch = BatchRequest::Single(async_graphql::Request::new(
            "query Titles($q: String!) { titles(query: $q) { id } }",
        ));
        assert_eq!(classify_graphql(&batch), GraphqlRateLimitClass::Search);
    }

    #[test]
    fn failed_login_bucket_blocks_after_five_failures() {
        let limiter = ScryerRateLimiter::from_env();
        let key = RateLimitKey::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)), None);

        for _ in 0..5 {
            assert!(limiter.record_failed_login(&key).is_ok());
        }
        assert!(limiter.record_failed_login(&key).is_err());
    }
}
