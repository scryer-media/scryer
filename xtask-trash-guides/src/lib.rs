use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use regex_syntax::Parser as RegexParser;
use regex_syntax::hir::{Class, Hir, HirKind};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, RETRY_AFTER, USER_AGENT};
use scryer_outbound_http::{blocking_reqwest_client, send_blocking_reqwest_request};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;
use xtask_support::{TaskContext, ok, run_checked, step};

/// One upstream directory of custom formats, and the `app` its records carry.
///
/// `docs/json/guide-only/` sits outside the per-app trees: upstream publishes it
/// as a menu of language policies that no shipped app profile includes, so it
/// gets its own pseudo-app rather than being folded into `sonarr` or `radarr`.
struct GuideSource {
    app: &'static str,
    /// Path under `docs/json/`, both for the GitHub contents listing and the raw
    /// file fetch.
    dir: &'static str,
}

const GUIDE_ONLY_APP: &str = "guide-only";

const GUIDE_SOURCES: &[GuideSource] = &[
    GuideSource {
        app: "sonarr",
        dir: "sonarr/cf",
    },
    GuideSource {
        app: "radarr",
        dir: "radarr/cf",
    },
    GuideSource {
        app: GUIDE_ONLY_APP,
        dir: GUIDE_ONLY_APP,
    },
];

const FETCH_WORKERS: usize = 36;
const GITHUB_REPO_API_BASE: &str = "https://api.github.com/repos/TRaSH-Guides/Guides";
const GITHUB_API_BASE: &str = "https://api.github.com/repos/TRaSH-Guides/Guides/contents/docs/json";
const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com/TRaSH-Guides/Guides";
const REQUEST_USER_AGENT: &str = "scryer-xtask-trash-guides";
const SOURCE_REVISION_ENV: &str = "SCRYER_TRASH_GUIDES_REVISION";
const SOURCE_DIR_ENV: &str = "SCRYER_TRASH_GUIDES_SOURCE_DIR";
const ACCEPT_STEMS_ENV: &str = "SCRYER_TRASH_GUIDES_ACCEPT_STEMS";

const QUALITY_OUTPUT: &str =
    "crates/scryer-application/src/quality/trash_guides_release_groups.generated.rs";
const PARSER_OUTPUT: &str =
    "crates/scryer-release-parser/src/trash_guides_parser_knowledge.generated.rs";
const SUMMARY_OUTPUT: &str = "xtask-trash-guides/generated/latest-summary.txt";
const MANIFEST_OUTPUT: &str = "xtask-trash-guides/generated/stem-classification.json";

struct LegacySeedGroup {
    name: &'static str,
    tier: DistilledTier,
    context: DistilledContext,
}

const LEGACY_SEED_GROUPS: &[LegacySeedGroup] = &[
    LegacySeedGroup {
        name: "BDMV",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "BDVD",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "BakedFish",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "D3US",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "DaddySubs",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "DeadFish",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Deadmau RAWS",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Erai-raws",
        tier: DistilledTier::Bronze,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "GSK_kun",
        tier: DistilledTier::Silver,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Iznjie Biznjie",
        tier: DistilledTier::Silver,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Judgment",
        tier: DistilledTier::Bronze,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "M2TS",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Mr.Deadpool",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "NAN0",
        tier: DistilledTier::Gold,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "NoGrop",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "NoobSubs",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "PiRaTeS",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "PMR",
        tier: DistilledTier::Gold,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "QAS",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "SpaceFish",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "SubsPlus+",
        tier: DistilledTier::Silver,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "tenshi",
        tier: DistilledTier::Silver,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "VISIONPLUSHDR-X",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "VISIONPLUSHDR1000",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "WtF Anime",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "YTS.AG",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "YTS.LT",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "YTS.MX",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "jennaortega",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "mal lu zen",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
];

/// How much evidence one distilled alias token carries on its own.
///
/// Sonarr applies a custom format when no `required: true` specification fails
/// and at least one specification matches. A service format whose title regex is
/// required therefore treats the bare token as sufficient, while a format with no
/// required specification matches *every* WEB release once its optional source
/// specs are considered — so its tokens only mean anything next to a WEB marker.
/// Required-spec formats get standalone tokens; no-required-spec formats get
/// WEB-adjacent ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ServiceTokenPolicy {
    /// Upstream deems the bare token sufficient evidence of the service.
    Standalone,
    /// The token counts only when immediately followed by a WEB marker.
    WebAdjacent,
}

impl ServiceTokenPolicy {
    fn requires_web_adjacency(self) -> bool {
        matches!(self, Self::WebAdjacent)
    }
}

/// One upstream service stem admitted to the alias table.
struct ServiceAliasSpec {
    stem: &'static str,
    service: &'static str,
    /// Policy applied to every distilled token without an override.
    default_policy: ServiceTokenPolicy,
    /// Tokens whose policy differs from the stem default (§6a rule 1 exceptions).
    token_overrides: &'static [(&'static str, ServiceTokenPolicy)],
    /// Tokens the patterns yield that must never become aliases, with the §6a reason.
    excluded_tokens: &'static [(&'static str, &'static str)],
}

/// The binding service inclusion list.
///
/// The first fourteen entries predate the redesign. They keep their standalone
/// behavior because the parser's detection tests pin it.
const SERVICE_ALIAS_SPECS: &[ServiceAliasSpec] = &[
    // Grandfathered services. `hbo`, `hmax`, `max`, and `stan` carry no required
    // specification upstream, but their standalone detection is long-established
    // parser behavior and is deliberately left alone by §6a's table.
    standalone_service("amzn", "Amazon"),
    standalone_service("atvp", "Apple TV+"),
    standalone_service("cr", "Crunchyroll"),
    standalone_service("dsnp", "Disney+"),
    standalone_service("funi", "Funimation"),
    standalone_service("hbo", "HBO Max"),
    standalone_service("hidive", "HIDIVE"),
    standalone_service("hmax", "HBO Max"),
    standalone_service("hulu", "Hulu"),
    standalone_service("max", "HBO Max"),
    standalone_service("nf", "Netflix"),
    standalone_service("pcok", "Peacock"),
    standalone_service("pmtp", "Paramount+"),
    standalone_service("stan", "Stan"),
    // Newly admitted services.
    standalone_service("4od", "Channel 4"),
    standalone_service("abema", "ABEMA"),
    standalone_service("all4", "Channel 4"),
    standalone_service("atv", "ATV"),
    standalone_service("aubc", "ABC iview"),
    ServiceAliasSpec {
        stem: "bcore",
        service: "BCORE",
        default_policy: ServiceTokenPolicy::Standalone,
        token_overrides: &[],
        excluded_tokens: &[("CORE", "common_word")],
    },
    standalone_service("bglobal", "B-Global"),
    standalone_service("bilibili", "Bilibili"),
    standalone_service("cbc", "CBC Gem"),
    web_adjacent_service("cnlp", "CANAL+"),
    standalone_service("cpng", "Coupang Play"),
    web_adjacent_service("crav", "Crave"),
    ServiceAliasSpec {
        stem: "dcu",
        service: "DC Universe",
        default_policy: ServiceTokenPolicy::Standalone,
        token_overrides: &[],
        excluded_tokens: &[("DC", "brand_prefix_with_title_collisions")],
    },
    standalone_service("dmm-tv", "DMM TV"),
    ServiceAliasSpec {
        stem: "dscp",
        service: "Discovery+",
        default_policy: ServiceTokenPolicy::WebAdjacent,
        token_overrides: &[],
        excluded_tokens: &[("DISC", "common_word"), ("DCP", "cinema_package")],
    },
    standalone_service("fod", "FOD"),
    standalone_service("french-adn", "ADN"),
    standalone_service("french-salto", "Salto"),
    standalone_service("french-wkn", "Wakanim"),
    web_adjacent_service("friday", "friDay Video"),
    standalone_service("hami", "Hami Video"),
    ServiceAliasSpec {
        stem: "htsr",
        service: "Disney+ Hotstar",
        default_policy: ServiceTokenPolicy::Standalone,
        token_overrides: &[],
        excluded_tokens: &[("HS", "too_short")],
    },
    ServiceAliasSpec {
        stem: "ip",
        service: "BBC iPlayer",
        default_policy: ServiceTokenPolicy::WebAdjacent,
        token_overrides: &[("IPLAYER", ServiceTokenPolicy::Standalone)],
        excluded_tokens: &[],
    },
    ServiceAliasSpec {
        stem: "iqiy",
        service: "iQIYI",
        default_policy: ServiceTokenPolicy::WebAdjacent,
        token_overrides: &[],
        excluded_tokens: &[("IQ", "too_short")],
    },
    ServiceAliasSpec {
        stem: "it",
        service: "iTunes",
        default_policy: ServiceTokenPolicy::WebAdjacent,
        token_overrides: &[],
        excluded_tokens: &[("ITUNES", "curated_supplement_keeps_standalone_spelling")],
    },
    web_adjacent_service("itvx", "ITVX"),
    standalone_service("kcw", "KOCOWA"),
    standalone_service("kktv", "KKTV"),
    standalone_service("linetv", "LINE TV"),
    standalone_service("my5", "My5"),
    standalone_service("mytvsuper", "myTV SUPER"),
    standalone_service("nlz", "NLZiet"),
    web_adjacent_service("now", "NOW"),
    standalone_service("ovid", "OVID.tv"),
    standalone_service("pathe", "Pathé Thuis"),
    web_adjacent_service("play", "PLAY"),
    standalone_service("qibi", "Quibi"),
    ServiceAliasSpec {
        stem: "red",
        service: "YouTube Premium",
        default_policy: ServiceTokenPolicy::WebAdjacent,
        token_overrides: &[],
        excluded_tokens: &[("YOUTUBE", "curated_supplement_owns_youtube")],
    },
    standalone_service("roku", "The Roku Channel"),
    ServiceAliasSpec {
        stem: "sho",
        service: "Showtime",
        default_policy: ServiceTokenPolicy::WebAdjacent,
        token_overrides: &[("SHOWTIME", ServiceTokenPolicy::Standalone)],
        excluded_tokens: &[],
    },
    standalone_service("strp", "Star+"),
    standalone_service("syfy", "SYFY"),
    standalone_service("tver", "TVer"),
    web_adjacent_service("tving", "TVING"),
    standalone_service("vdl", "Videoland"),
    standalone_service("viki", "Viki"),
    web_adjacent_service("viu", "Viu"),
    standalone_service("vrv", "VRV"),
    standalone_service("wavve", "Wavve"),
    standalone_service("wetv", "WeTV"),
    standalone_service("youku", "Youku"),
];

/// Service-shaped stems excluded from the alias table outright, with reasons.
const EXCLUDED_SERVICE_ALIAS_STEMS: &[(&str, &str)] = &[
    ("sic", "service_alias_combo_format_not_a_service_tag"),
    (
        "hd-streaming-boost",
        "service_alias_duplicate_pattern_booster",
    ),
    ("ma", "service_alias_negative_lookaround_token_collision"),
    ("u-next", "service_alias_unrepresentable_compound_token"),
];

const fn standalone_service(stem: &'static str, service: &'static str) -> ServiceAliasSpec {
    ServiceAliasSpec {
        stem,
        service,
        default_policy: ServiceTokenPolicy::Standalone,
        token_overrides: &[],
        excluded_tokens: &[],
    }
}

const fn web_adjacent_service(stem: &'static str, service: &'static str) -> ServiceAliasSpec {
    ServiceAliasSpec {
        stem,
        service,
        default_policy: ServiceTokenPolicy::WebAdjacent,
        token_overrides: &[],
        excluded_tokens: &[],
    }
}

#[derive(Debug, Deserialize)]
struct GitHubEntry {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubCommit {
    sha: String,
}

#[derive(Debug, Clone)]
struct FetchTask {
    app: String,
    dir: String,
    filename: String,
}

impl FetchTask {
    fn source_path(&self) -> String {
        format!("docs/json/{}/{}", self.dir, self.filename)
    }
}

#[derive(Debug, Deserialize)]
struct UpstreamCf {
    name: String,
    #[serde(default)]
    trash_id: String,
    #[serde(default)]
    trash_scores: Option<Value>,
    #[serde(default)]
    specifications: Vec<UpstreamSpec>,
}

#[derive(Debug, Deserialize)]
struct UpstreamSpec {
    name: String,
    implementation: String,
    #[serde(default)]
    required: Value,
    #[serde(default)]
    negate: Value,
    #[serde(default)]
    fields: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct UpstreamRecord {
    app: String,
    stem: String,
    source_path: String,
    trash_id: String,
    cf_name: String,
    spec_name: String,
    implementation: String,
    value: String,
    required_json: String,
    negate_json: String,
    /// Upstream `trash_scores` for the custom format this specification came
    /// from, keyed by score set. Parsed once per file and carried on every
    /// record so distillation can join scores to the facts it emits.
    scores: BTreeMap<String, i64>,
}

#[derive(Debug)]
struct FetchedRecords {
    source_revision: String,
    records: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DistilledTier {
    Gold,
    Silver,
    Bronze,
    Banned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DistilledContext {
    Web,
    BluRay,
    UhdBluRay,
    Remux,
    Anime,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DistilledFacet {
    Movie,
    Series,
    Anime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GroupMatchKindSpec {
    Exact,
    Prefix,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupRuleIdentity {
    matcher: String,
    match_kind: GroupMatchKindSpec,
    facet: DistilledFacet,
    context: DistilledContext,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupRuleKey {
    matcher: String,
    match_kind: GroupMatchKindSpec,
    tier: DistilledTier,
    facet: DistilledFacet,
    context: DistilledContext,
}

#[derive(Debug, Clone)]
struct GroupRuleRecord {
    key: GroupRuleKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ParserSignalKindSpec {
    AiEnhanced,
    Proper,
    Repack,
    DubsOnly,
    HardcodedSubs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TokenPatternKindSpec {
    Sequence,
    RequiredTokens,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TokenPatternSpec {
    kind: TokenPatternKindSpec,
    tokens: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ServiceAliasKey {
    token: String,
    service: String,
    requires_web_adjacency: bool,
}

#[derive(Debug, Clone)]
struct ServiceAliasRecord {
    key: ServiceAliasKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SignalRuleKey {
    kind: ParserSignalKindSpec,
    pattern: TokenPatternSpec,
}

#[derive(Debug, Clone)]
struct SignalRuleRecord {
    key: SignalRuleKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CategoryScopeSpec {
    Any,
    Anime,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BlockedTitleKey {
    code: String,
    facet: DistilledFacet,
    category: CategoryScopeSpec,
    pattern: TokenPatternSpec,
}

#[derive(Debug, Clone)]
struct BlockedTitleRecord {
    key: BlockedTitleKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FactRuleKey {
    code: String,
    facet: DistilledFacet,
    category: CategoryScopeSpec,
    pattern: TokenPatternSpec,
}

#[derive(Debug, Clone)]
struct FactRuleRecord {
    key: FactRuleKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LocaleGroupFactRuleKey {
    code: String,
    matcher: String,
    match_kind: GroupMatchKindSpec,
    facet: DistilledFacet,
    source_context: DistilledContext,
}

#[derive(Debug, Clone)]
struct LocaleGroupFactRuleRecord {
    key: LocaleGroupFactRuleKey,
    provenance: Vec<UpstreamRecord>,
}

/// One `LanguageSpecification` value, resolved to something the rule input can
/// answer.
///
/// `Named` carries the canonical audio-language code
/// `crate::normalize_detected_audio_language_code` produces, which is exactly
/// what `user_rule_input` publishes in `release.languages_audio`. `Original` is
/// upstream's relative id `-2`, resolved against the title's own original
/// language rather than a fixed code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum LanguageConditionSpec {
    Named(String),
    Original,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LanguageConditionEntry {
    language: LanguageConditionSpec,
    negate: bool,
    required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LanguageRuleKey {
    code: String,
    app: String,
    stem: String,
    conditions: Vec<LanguageConditionEntry>,
}

#[derive(Debug, Clone)]
struct LanguageRuleRecord {
    key: LanguageRuleKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DetectionOwner {
    NativeFact,
    ExistingNative,
    ManagedRego,
    CustomRuleOnly,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EffectBinding {
    HardBlock,
    PersonaScore,
    LocaleScore,
    Informational,
    None,
}

#[derive(Debug, Clone, Copy)]
struct StemClassification {
    detection_owner: DetectionOwner,
    effect_binding: EffectBinding,
    reason: &'static str,
}

#[derive(Debug, Serialize, Deserialize)]
struct StemClassificationManifest {
    source_revision: String,
    /// Reviewed score range per upstream score set. It is an alarm only: the
    /// normalization never reads it, so a rescaled upstream trips the gate
    /// instead of quietly changing what vetoes.
    #[serde(default)]
    score_envelope: Vec<ScoreSetEnvelope>,
    stems: Vec<StemClassificationManifestRecord>,
    /// Per-specification provenance for everything the distillation did not turn
    /// into a runtime rule. This is audit data: it belongs in the manifest so it
    /// stays queryable without being compiled into the generated Rust.
    #[serde(default)]
    inactive_records: Vec<AuditRecord>,
    #[serde(default)]
    ignored_records: Vec<AuditRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuditRecord {
    app: String,
    stem: String,
    trash_id: String,
    cf_name: String,
    spec_name: String,
    implementation: String,
    reason: String,
    source_path: String,
}

impl From<&MetadataRuleRecord> for AuditRecord {
    fn from(record: &MetadataRuleRecord) -> Self {
        Self {
            app: record.app.clone(),
            stem: record.stem.clone(),
            trash_id: record.trash_id.clone(),
            cf_name: record.cf_name.clone(),
            spec_name: record.spec_name.clone(),
            implementation: record.implementation.clone(),
            reason: record.reason.clone(),
            source_path: record.source_path.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StemClassificationManifestRecord {
    app: String,
    stem: String,
    detection_owner: DetectionOwner,
    effect_binding: EffectBinding,
    reason: String,
    #[serde(default)]
    emitted_rule_count: usize,
    #[serde(default)]
    emitted_fact_codes: Vec<String>,
    #[serde(default)]
    emitted_rule_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MetadataRuleRecord {
    app: String,
    facet: DistilledFacet,
    stem: String,
    trash_id: String,
    cf_name: String,
    spec_name: String,
    implementation: String,
    value: String,
    reason: String,
    source_path: String,
}

#[derive(Debug, Default)]
struct DistilledCatalog {
    group_rules: Vec<GroupRuleRecord>,
    service_alias_rules: Vec<ServiceAliasRecord>,
    signal_rules: Vec<SignalRuleRecord>,
    blocked_title_rules: Vec<BlockedTitleRecord>,
    fact_rules: Vec<FactRuleRecord>,
    locale_group_fact_rules: Vec<LocaleGroupFactRuleRecord>,
    language_rules: Vec<LanguageRuleRecord>,
    no_release_group_facets: Vec<DistilledFacet>,
    inactive_records: Vec<MetadataRuleRecord>,
    ignored_records: Vec<MetadataRuleRecord>,
}

pub fn run_sync(ctx: &TaskContext) -> Result<()> {
    step("Fetching TRaSH Guides custom formats");
    let fetched = fetch_all_records()?;
    ok(format!(
        "Fetched {} custom-format specifications at {}",
        fetched.records.len(),
        fetched.source_revision
    ));
    let manifest_output = ctx.path(MANIFEST_OUTPUT);

    step("Distilling Scryer-native rule sets");
    let distilled = distill_records(&fetched.records)?;
    validate_distilled_catalog(&distilled, &fetched.records)?;
    enforce_stem_coverage(&manifest_output, &fetched.records, &distilled)?;
    ok(format!(
        "Distilled {} release-group rules, {} service aliases, {} parser signals, {} blocked title rules",
        distilled.group_rules.len(),
        distilled.service_alias_rules.len(),
        distilled.signal_rules.len(),
        distilled.blocked_title_rules.len()
    ));

    step("Writing generated outputs");
    let quality_output = ctx.path(QUALITY_OUTPUT);
    let parser_output = ctx.path(PARSER_OUTPUT);
    let summary_output = ctx.path(SUMMARY_OUTPUT);

    write_if_changed(
        &quality_output,
        &render_quality_output(&distilled, &fetched.records, &fetched.source_revision)?,
    )?;
    write_if_changed(
        &parser_output,
        &render_parser_output(&distilled, &fetched.source_revision)?,
    )?;
    write_if_changed(
        &summary_output,
        &render_summary(&distilled, &fetched.source_revision),
    )?;
    write_if_changed(
        &manifest_output,
        &render_stem_manifest(&fetched.records, &distilled, &fetched.source_revision)?,
    )?;
    format_generated_rust(ctx)?;
    ok("Generated TRaSH distillation artifacts refreshed");

    Ok(())
}

fn validate_distilled_catalog(
    catalog: &DistilledCatalog,
    records: &[UpstreamRecord],
) -> Result<()> {
    const REQUIRED_FACT_CODES: &[&str] = &[
        "trash.scene",
        "trash.obfuscated",
        "trash.retagged",
        "trash.locale.french.group.tier1",
        "trash.locale.french.group.tier2",
        "trash.locale.french.group.tier3",
        "trash.locale.french.lq",
        "trash.locale.french.scene",
        "trash.locale.french.marker.vostfr",
        "trash.locale.french.marker.vff",
        "trash.locale.french.marker.vfi",
        "trash.locale.french.marker.vof",
        "trash.locale.french.marker.voq",
        "trash.locale.french.marker.vq",
        "trash.locale.french.marker.vfq",
        "trash.locale.german.group.tier1",
        "trash.locale.german.group.tier2",
        "trash.locale.german.group.tier3",
        "trash.locale.german.lq",
        "trash.locale.german.scene",
        "trash.locale.german.marker.subbed",
        "trash.locale.asian.group.tier1",
        "trash.locale.asian.group.tier2",
        "trash.locale.asian.group.tier3",
        "trash.locale.asian.lq",
    ];

    let emitted = catalog
        .fact_rules
        .iter()
        .map(|rule| rule.key.code.as_str())
        .chain(
            catalog
                .locale_group_fact_rules
                .iter()
                .map(|rule| rule.key.code.as_str()),
        )
        .collect::<BTreeSet<_>>();
    let missing = REQUIRED_FACT_CODES
        .iter()
        .copied()
        .filter(|code| !emitted.contains(code))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "distilled TRaSH catalog is missing managed facts: {}",
            missing.join(", ")
        );
    }

    let unscored = unscored_fact_apps(catalog);
    if !unscored.is_empty() {
        bail!(
            "emitted TRaSH facts have no upstream score for the app that produced them: {}",
            unscored.join(", ")
        );
    }

    let unbound = build_stem_manifest(records, catalog, "")
        .stems
        .into_iter()
        .filter(|record| {
            matches!(
                record.effect_binding,
                EffectBinding::HardBlock | EffectBinding::PersonaScore | EffectBinding::LocaleScore
            ) && (matches!(
                record.detection_owner,
                DetectionOwner::NativeFact | DetectionOwner::ManagedRego
            ) || classify_active_group_stem(&record.stem).is_some())
                && record.emitted_rule_count == 0
        })
        .map(|record| format!("{}/{}", record.app, record.stem))
        .collect::<Vec<_>>();
    if !unbound.is_empty() {
        bail!(
            "effect-bound TRaSH stems emitted no runtime rules: {}",
            unbound.join(", ")
        );
    }

    validate_language_rules(&catalog.language_rules)?;
    validate_service_alias_rules(&catalog.service_alias_rules)
}

/// A language rule code is what the managed packs key off, so the apps that
/// publish the same code must agree on what it means.
///
/// Upstream ships the `language-*` family identically in both apps today. If
/// one of them ever diverges, the packs would silently follow whichever app
/// happened to resolve first, so the disagreement fails the sync instead.
fn validate_language_rules(rules: &[LanguageRuleRecord]) -> Result<()> {
    let mut by_code = BTreeMap::<&str, BTreeSet<&Vec<LanguageConditionEntry>>>::new();
    for rule in rules {
        by_code
            .entry(rule.key.code.as_str())
            .or_default()
            .insert(&rule.key.conditions);
    }

    let divergent = by_code
        .iter()
        .filter(|(_, conditions)| conditions.len() > 1)
        .map(|(code, _)| (*code).to_string())
        .collect::<Vec<_>>();
    if !divergent.is_empty() {
        bail!(
            "TRaSH language rules disagree across apps for: {}",
            divergent.join(", ")
        );
    }
    Ok(())
}

fn validate_service_alias_rules(rules: &[ServiceAliasRecord]) -> Result<()> {
    let mut services_by_token = BTreeMap::<&str, BTreeSet<&str>>::new();
    let mut policies_by_token = BTreeMap::<&str, BTreeSet<bool>>::new();
    for rule in rules {
        services_by_token
            .entry(rule.key.token.as_str())
            .or_default()
            .insert(rule.key.service.as_str());
        policies_by_token
            .entry(rule.key.token.as_str())
            .or_default()
            .insert(rule.key.requires_web_adjacency);
    }
    let unusable = services_by_token
        .iter()
        .filter(|(token, _)| sanitize_alias_token(token).as_deref() != Some(**token))
        .map(|(token, _)| (*token).to_string())
        .collect::<Vec<_>>();
    if !unusable.is_empty() {
        bail!(
            "service alias tokens match too broadly to be title tokens: {}",
            unusable.join(", ")
        );
    }
    let ambiguous = services_by_token
        .iter()
        .filter(|(_, services)| services.len() > 1)
        .map(|(token, services)| {
            format!(
                "{token} -> {}",
                services.iter().copied().collect::<Vec<_>>().join("/")
            )
        })
        .collect::<Vec<_>>();
    if !ambiguous.is_empty() {
        bail!(
            "service alias tokens resolve to more than one service: {}",
            ambiguous.join(", ")
        );
    }
    // One token, one policy. A token that is standalone for one service and
    // WEB-adjacent for another has no answer at the context-free call sites,
    // which consult standalone aliases only.
    let split_policy = policies_by_token
        .iter()
        .filter(|(_, policies)| policies.len() > 1)
        .map(|(token, _)| (*token).to_string())
        .collect::<Vec<_>>();
    if !split_policy.is_empty() {
        bail!(
            "service alias tokens claim both standalone and WEB-adjacent policies: {}",
            split_policy.join(", ")
        );
    }
    Ok(())
}

fn fetch_all_records() -> Result<FetchedRecords> {
    if let Ok(source_dir) = std::env::var(SOURCE_DIR_ENV)
        && !source_dir.trim().is_empty()
    {
        return read_local_records(Path::new(&source_dir));
    }
    let client = blocking_reqwest_client().context("failed to build HTTP client")?;
    let source_revision = resolve_source_revision(&client)?;
    let mut tasks = Vec::new();

    for source in GUIDE_SOURCES {
        let listing_url = format!("{GITHUB_API_BASE}/{}?ref={source_revision}", source.dir);
        let listing = get_json::<Vec<GitHubEntry>>(&client, &listing_url)
            .with_context(|| format!("failed to list {} custom formats", source.app))?;

        for entry in listing {
            if !entry.name.ends_with(".json") {
                continue;
            }
            tasks.push(FetchTask {
                app: source.app.to_string(),
                dir: source.dir.to_string(),
                filename: entry.name,
            });
        }
    }

    let worker_count = tasks.len().clamp(1, FETCH_WORKERS);
    let chunk_size = tasks.len().div_ceil(worker_count).max(1);
    let mut records = Vec::new();

    thread::scope(|scope| -> Result<()> {
        let mut workers = Vec::new();
        for chunk in tasks.chunks(chunk_size) {
            let client = client.clone();
            let source_revision = source_revision.clone();
            workers.push(scope.spawn(move || -> Result<Vec<UpstreamRecord>> {
                let mut chunk_records = Vec::new();
                for task in chunk {
                    chunk_records.extend(fetch_records_for_task(&client, task, &source_revision)?);
                }
                Ok(chunk_records)
            }));
        }

        for worker in workers {
            records.extend(
                worker
                    .join()
                    .map_err(|panic| anyhow!("TRaSH fetch worker panicked: {panic:?}"))??,
            );
        }
        Ok(())
    })?;

    records.sort();
    Ok(FetchedRecords {
        source_revision,
        records,
    })
}

fn read_local_records(source_dir: &Path) -> Result<FetchedRecords> {
    let source_revision = std::env::var(SOURCE_REVISION_ENV)
        .ok()
        .filter(|revision| !revision.trim().is_empty())
        .context(format!(
            "{SOURCE_REVISION_ENV} is required when {SOURCE_DIR_ENV} is set"
        ))?;
    let mut records = Vec::new();
    for source in GUIDE_SOURCES {
        let directory = source_dir.join("docs/json").join(source.dir);
        let mut paths = fs::read_dir(&directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.retain(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        });
        paths.sort();

        for path in paths {
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("TRaSH source path did not have a UTF-8 filename")?
                .to_string();
            let task = FetchTask {
                app: source.app.to_string(),
                dir: source.dir.to_string(),
                filename,
            };
            let parsed = serde_json::from_str::<UpstreamCf>(
                &fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?,
            )
            .with_context(|| format!("failed to parse {}", path.display()))?;
            let source_path = task.source_path();
            records.extend(records_from_custom_format(&task, parsed, source_path)?);
        }
    }
    records.sort();
    Ok(FetchedRecords {
        source_revision,
        records,
    })
}

fn resolve_source_revision(client: &Client) -> Result<String> {
    let requested = std::env::var(SOURCE_REVISION_ENV)
        .ok()
        .filter(|revision| !revision.trim().is_empty())
        .unwrap_or_else(|| "master".to_string());
    let commit_url = format!("{GITHUB_REPO_API_BASE}/commits/{requested}");
    let resolved = get_json::<GitHubCommit>(client, &commit_url)
        .with_context(|| format!("failed to resolve TRaSH Guides revision {requested}"))?;
    if resolved.sha.is_empty() {
        bail!("TRaSH Guides resolved an empty source revision for {requested}");
    }
    Ok(resolved.sha)
}

fn fetch_records_for_task(
    client: &Client,
    task: &FetchTask,
    source_revision: &str,
) -> Result<Vec<UpstreamRecord>> {
    let source_path = task.source_path();
    let raw_url = format!("{GITHUB_RAW_BASE}/{source_revision}/{source_path}");
    let parsed = get_json::<UpstreamCf>(client, &raw_url)
        .with_context(|| format!("failed to fetch {}", task.filename))?;
    records_from_custom_format(task, parsed, source_path)
}

fn records_from_custom_format(
    task: &FetchTask,
    parsed: UpstreamCf,
    source_path: String,
) -> Result<Vec<UpstreamRecord>> {
    let stem = task.filename.trim_end_matches(".json").to_string();
    let scores = parse_trash_scores(parsed.trash_scores.as_ref(), &source_path)?;
    let mut records = Vec::new();

    for spec in parsed.specifications {
        let value = spec
            .fields
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| spec.fields.to_string());
        records.push(UpstreamRecord {
            app: task.app.clone(),
            stem: stem.clone(),
            source_path: source_path.clone(),
            trash_id: parsed.trash_id.clone(),
            cf_name: parsed.name.clone(),
            spec_name: spec.name,
            implementation: spec.implementation,
            value,
            required_json: json_string(Some(&spec.required)),
            negate_json: json_string(Some(&spec.negate)),
            scores: scores.clone(),
        });
    }

    Ok(records)
}

/// Scores are the valuation half of the guide, so a shape Scryer cannot read is
/// a hard failure rather than a silently dropped entry.
fn parse_trash_scores(scores: Option<&Value>, source_path: &str) -> Result<BTreeMap<String, i64>> {
    let Some(scores) = scores else {
        return Ok(BTreeMap::new());
    };
    let Some(entries) = scores.as_object() else {
        bail!("trash_scores in {source_path} is not an object");
    };

    entries
        .iter()
        .map(|(score_set, value)| {
            let score = value.as_i64().ok_or_else(|| {
                anyhow!("non-integer trash score {value} for {score_set} in {source_path}")
            })?;
            Ok((score_set.clone(), score))
        })
        .collect()
}

fn json_string(value: Option<&Value>) -> String {
    serde_json::to_string(value.unwrap_or(&Value::Null))
        .expect("serializing a serde_json::Value cannot fail")
}

fn get_json<T: for<'de> Deserialize<'de>>(client: &Client, url: &str) -> Result<T> {
    let mut last_error = None;
    for attempt in 1..=4 {
        match send_blocking_reqwest_request(
            client
                .get(url)
                .timeout(Duration::from_secs(30))
                .header(USER_AGENT, REQUEST_USER_AGENT)
                .header(ACCEPT, "application/json"),
        ) {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return response
                        .json::<T>()
                        .with_context(|| format!("failed to decode JSON from {url}"));
                }
                last_error = Some(anyhow!("unsuccessful response for {url}: HTTP {status}"));
                if attempt < 4 {
                    thread::sleep(retry_delay_for_response(status, &response, attempt));
                }
                continue;
            }
            Err(error) => last_error = Some(anyhow!("request failed for {url}: {error}")),
        }

        if attempt < 4 {
            thread::sleep(retry_delay_for_attempt(attempt));
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("request failed for {url}")))
}

fn retry_delay_for_response(
    status: StatusCode,
    response: &reqwest::blocking::Response,
    attempt: usize,
) -> Duration {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return retry_after_delay(response)
            .unwrap_or_else(|| Duration::from_secs((attempt as u64).max(1)));
    }

    retry_delay_for_attempt(attempt)
}

fn retry_delay_for_attempt(attempt: usize) -> Duration {
    Duration::from_millis(250 * attempt as u64)
}

fn retry_after_delay(response: &reqwest::blocking::Response) -> Option<Duration> {
    let retry_after = response.headers().get(RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = retry_after.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let when = DateTime::parse_from_rfc2822(retry_after).ok()?;
    let when = when.with_timezone(&Utc);
    if when <= Utc::now() {
        return Some(Duration::from_secs(0));
    }

    (when - Utc::now()).to_std().ok()
}

fn distill_records(records: &[UpstreamRecord]) -> Result<DistilledCatalog> {
    let mut group_rules = BTreeMap::<GroupRuleKey, Vec<UpstreamRecord>>::new();
    let mut service_alias_rules = BTreeMap::<ServiceAliasKey, Vec<UpstreamRecord>>::new();
    let mut signal_rules = BTreeMap::<SignalRuleKey, Vec<UpstreamRecord>>::new();
    let mut blocked_title_rules = BTreeMap::<BlockedTitleKey, Vec<UpstreamRecord>>::new();
    let mut fact_rules = BTreeMap::<FactRuleKey, Vec<UpstreamRecord>>::new();
    let mut locale_group_fact_rules =
        BTreeMap::<LocaleGroupFactRuleKey, Vec<UpstreamRecord>>::new();
    let mut no_release_group_facets = BTreeSet::<DistilledFacet>::new();
    let mut language_format_records = BTreeMap::<(String, String), Vec<UpstreamRecord>>::new();
    let mut inactive_records = BTreeSet::<MetadataRuleRecord>::new();
    let mut ignored_records = BTreeSet::<MetadataRuleRecord>::new();

    for record in records {
        // Language formats are decided per *format*, not per specification —
        // `DidMatch` combines the whole specification list — so they are
        // collected here and distilled once the group is complete.
        if is_language_rule_stem(&record.app, &record.stem) {
            language_format_records
                .entry((record.app.clone(), record.stem.clone()))
                .or_default()
                .push(record.clone());
            continue;
        }

        // Alias capture runs first and additively: three of §6a's services are
        // also French locale markers (`french-adn`, `french-salto`,
        // `french-wkn`), and those markers still have to reach the locale branch
        // below. For every other service stem this arm owns the record outright.
        if let Some(spec) = service_alias_spec(&record.stem) {
            let locale_owned = is_localized_stem(&record.stem);
            collect_service_alias_tokens(
                record,
                spec,
                locale_owned,
                &mut service_alias_rules,
                &mut inactive_records,
            );
            if !locale_owned {
                continue;
            }
        } else if let Some(reason) = excluded_service_alias_reason(&record.stem) {
            ignored_records.insert(metadata_record(record, reason));
            continue;
        }

        if let Some(group_target) = classify_active_group_stem(&record.stem) {
            let facet = facet_for_record(record);
            let matchers = match record.implementation.as_str() {
                "ReleaseGroupSpecification" => distill_group_matchers(record)?,
                "ReleaseTitleSpecification" => {
                    if let Some(matcher) = distill_title_spec_group_matcher(record) {
                        vec![matcher]
                    } else {
                        ignored_records
                            .insert(metadata_record(record, "group_title_spec_not_lossless"));
                        continue;
                    }
                }
                _ => {
                    ignored_records.insert(metadata_record(record, "group_spec_not_supported"));
                    continue;
                }
            };

            for (matcher, match_kind) in matchers {
                let key = GroupRuleKey {
                    matcher,
                    match_kind,
                    tier: group_target.0,
                    facet,
                    context: group_target.1,
                };
                group_rules.entry(key).or_default().push(record.clone());
            }
            continue;
        }

        if is_localized_stem(&record.stem) {
            if let Some(source_context) = locale_group_source_context(&record.stem) {
                let matchers = match record.implementation.as_str() {
                    "ReleaseGroupSpecification" => distill_group_matchers(record)?,
                    "ReleaseTitleSpecification" if !record_is_negated(record) => {
                        if let Some(matcher) = distill_title_spec_group_matcher(record) {
                            vec![matcher]
                        } else {
                            inactive_records.insert(metadata_record(
                                record,
                                "locale_group_title_spec_not_lossless",
                            ));
                            continue;
                        }
                    }
                    "ReleaseTitleSpecification" => {
                        inactive_records
                            .insert(metadata_record(record, "locale_group_negated_title_spec"));
                        continue;
                    }
                    _ => {
                        inactive_records
                            .insert(metadata_record(record, "locale_group_spec_not_supported"));
                        continue;
                    }
                };
                for (matcher, match_kind) in matchers {
                    locale_group_fact_rules
                        .entry(LocaleGroupFactRuleKey {
                            code: locale_fact_code(&record.stem),
                            matcher,
                            match_kind,
                            facet: facet_for_record(record),
                            source_context,
                        })
                        .or_default()
                        .push(record.clone());
                }
                continue;
            }
            if record.implementation != "ReleaseTitleSpecification" || record_is_negated(record) {
                inactive_records.insert(metadata_record(
                    record,
                    "locale_marker_requires_positive_title_spec",
                ));
                continue;
            }
            if !record_is_sufficient_fact(records, record)
                && !matches!(
                    record.stem.as_str(),
                    "french-vff" | "french-vfq" | "german-subbed"
                )
            {
                inactive_records.insert(metadata_record(
                    record,
                    "locale_marker_is_only_one_required_component",
                ));
                continue;
            }
            match distill_named_patterns(record, ParserSignalKindSpec::Proper) {
                Ok(patterns) => {
                    for pattern in patterns {
                        fact_rules
                            .entry(FactRuleKey {
                                code: locale_fact_code(&record.stem),
                                facet: facet_for_record(record),
                                category: CategoryScopeSpec::Any,
                                pattern,
                            })
                            .or_default()
                            .push(record.clone());
                    }
                }
                Err(_) => {
                    inactive_records.insert(metadata_record(record, "locale_fact_not_lossless"));
                }
            }
            continue;
        }

        match record.stem.as_str() {
            "upscaled" => {
                for pattern in distill_named_patterns(record, ParserSignalKindSpec::AiEnhanced)? {
                    let key = SignalRuleKey {
                        kind: ParserSignalKindSpec::AiEnhanced,
                        pattern: pattern.clone(),
                    };
                    signal_rules.entry(key).or_default().push(record.clone());
                    fact_rules
                        .entry(FactRuleKey {
                            code: "trash.ai_enhanced".to_string(),
                            facet: facet_for_record(record),
                            category: CategoryScopeSpec::Any,
                            pattern,
                        })
                        .or_default()
                        .push(record.clone());
                }
            }
            "repack-proper" | "repack2" | "repack3" => {
                if record_is_negated(record) {
                    inactive_records.insert(metadata_record(record, "negated_repack_constraint"));
                    continue;
                }
                for (kind, pattern) in distill_repack_patterns(record)? {
                    let key = SignalRuleKey {
                        kind,
                        pattern: pattern.clone(),
                    };
                    signal_rules.entry(key).or_default().push(record.clone());
                    for code in fact_codes_for_signal(kind) {
                        fact_rules
                            .entry(FactRuleKey {
                                code: (*code).to_string(),
                                facet: facet_for_record(record),
                                category: CategoryScopeSpec::Any,
                                pattern: pattern.clone(),
                            })
                            .or_default()
                            .push(record.clone());
                    }
                }
            }
            "dubs-only" => {
                if record.implementation != "ReleaseTitleSpecification" || record_is_negated(record)
                {
                    inactive_records.insert(metadata_record(
                        record,
                        "dubs_only_requires_positive_title_spec",
                    ));
                    continue;
                }
                for pattern in distill_dubs_only_patterns(record)? {
                    let key = SignalRuleKey {
                        kind: ParserSignalKindSpec::DubsOnly,
                        pattern: pattern.clone(),
                    };
                    signal_rules.entry(key).or_default().push(record.clone());
                    fact_rules
                        .entry(FactRuleKey {
                            code: "trash.dubs_only".to_string(),
                            facet: facet_for_record(record),
                            category: CategoryScopeSpec::Any,
                            pattern,
                        })
                        .or_default()
                        .push(record.clone());
                }
            }
            "anime-raws" => {
                let Ok(patterns) =
                    distill_blocked_title_patterns(record, "trash_guides_anime_raws")
                else {
                    inactive_records.insert(metadata_record(
                        record,
                        "blocked_title_pattern_not_lossless",
                    ));
                    continue;
                };
                for pattern in patterns {
                    let key = BlockedTitleKey {
                        code: "trash_guides_anime_raws".to_string(),
                        facet: DistilledFacet::Anime,
                        category: CategoryScopeSpec::Anime,
                        pattern,
                    };
                    blocked_title_rules
                        .entry(key)
                        .or_default()
                        .push(record.clone());
                }
            }
            "lq-release-title" => {
                let Ok(patterns) =
                    distill_blocked_title_patterns(record, "trash_guides_lq_release_title")
                else {
                    inactive_records.insert(metadata_record(
                        record,
                        "blocked_title_pattern_not_lossless",
                    ));
                    continue;
                };
                for pattern in patterns {
                    let key = BlockedTitleKey {
                        code: "trash_guides_lq_release_title".to_string(),
                        facet: facet_for_record(record),
                        category: CategoryScopeSpec::Any,
                        pattern,
                    };
                    blocked_title_rules
                        .entry(key)
                        .or_default()
                        .push(record.clone());
                }
            }
            "fansub" => {
                for pattern in distill_named_patterns(record, ParserSignalKindSpec::HardcodedSubs)?
                {
                    let key = BlockedTitleKey {
                        code: "trash_guides_fansub".to_string(),
                        facet: DistilledFacet::Anime,
                        category: CategoryScopeSpec::Anime,
                        pattern: pattern.clone(),
                    };
                    blocked_title_rules
                        .entry(key)
                        .or_default()
                        .push(record.clone());
                    for code in ["trash.hardcoded_subs", "trash.fansub"] {
                        fact_rules
                            .entry(FactRuleKey {
                                code: code.to_string(),
                                facet: DistilledFacet::Anime,
                                category: CategoryScopeSpec::Anime,
                                pattern: pattern.clone(),
                            })
                            .or_default()
                            .push(record.clone());
                    }
                }
            }
            "fastsub" => {
                for pattern in distill_named_patterns(record, ParserSignalKindSpec::HardcodedSubs)?
                {
                    let key = BlockedTitleKey {
                        code: "trash_guides_fastsub".to_string(),
                        facet: DistilledFacet::Anime,
                        category: CategoryScopeSpec::Anime,
                        pattern: pattern.clone(),
                    };
                    blocked_title_rules
                        .entry(key)
                        .or_default()
                        .push(record.clone());
                    for code in ["trash.hardcoded_subs", "trash.fastsub"] {
                        fact_rules
                            .entry(FactRuleKey {
                                code: code.to_string(),
                                facet: DistilledFacet::Anime,
                                category: CategoryScopeSpec::Anime,
                                pattern: pattern.clone(),
                            })
                            .or_default()
                            .push(record.clone());
                    }
                }
            }
            "scene" | "obfuscated" | "retags" => {
                if record.implementation != "ReleaseTitleSpecification" || record_is_negated(record)
                {
                    inactive_records.insert(metadata_record(
                        record,
                        "native_fact_requires_positive_title_spec",
                    ));
                    continue;
                }
                if record.stem == "scene" {
                    let matchers = distill_scene_group_matchers(record)?;
                    if matchers.is_empty() {
                        inactive_records
                            .insert(metadata_record(record, "scene_group_pattern_not_lossless"));
                    } else {
                        for matcher in matchers {
                            locale_group_fact_rules
                                .entry(LocaleGroupFactRuleKey {
                                    code: native_fact_code(&record.stem).to_string(),
                                    matcher,
                                    match_kind: GroupMatchKindSpec::Exact,
                                    facet: facet_for_record(record),
                                    source_context: DistilledContext::Any,
                                })
                                .or_default()
                                .push(record.clone());
                        }
                    }
                    continue;
                }
                if record.stem == "obfuscated" {
                    if let Some(matcher) = distill_terminal_group_matcher(record) {
                        locale_group_fact_rules
                            .entry(LocaleGroupFactRuleKey {
                                code: native_fact_code(&record.stem).to_string(),
                                matcher,
                                match_kind: GroupMatchKindSpec::Exact,
                                facet: facet_for_record(record),
                                source_context: DistilledContext::Any,
                            })
                            .or_default()
                            .push(record.clone());
                    } else {
                        inactive_records.insert(metadata_record(
                            record,
                            "obfuscated_group_pattern_not_lossless",
                        ));
                    }
                    continue;
                }
                match distill_named_patterns(record, ParserSignalKindSpec::Proper) {
                    Ok(patterns) => {
                        for pattern in patterns {
                            fact_rules
                                .entry(FactRuleKey {
                                    code: native_fact_code(&record.stem).to_string(),
                                    facet: facet_for_record(record),
                                    category: CategoryScopeSpec::Any,
                                    pattern,
                                })
                                .or_default()
                                .push(record.clone());
                        }
                    }
                    Err(_) => {
                        inactive_records
                            .insert(metadata_record(record, "native_fact_not_lossless"));
                    }
                }
            }
            "no-rlsgroup" => {
                if record.implementation == "ReleaseGroupSpecification"
                    && record.value.trim() == "."
                {
                    no_release_group_facets.insert(facet_for_record(record));
                } else {
                    inactive_records.insert(metadata_record(
                        record,
                        "no_release_group_requires_wildcard_group_spec",
                    ));
                }
            }
            "hdr" | "hdr10plus-boost" | "dv-boost" | "dv-disk" | "dv-wo-hdr-fallback"
            | "hybrid" | "remaster" | "truehd-atmos" | "ddplus-atmos" | "10bit" | "10-mono"
            | "20-stereo" | "line-mic-dubbed" => {
                ignored_records.insert(metadata_record(
                    record,
                    "custom_rule_only_existing_parser_signal",
                ));
            }
            _ => {
                ignored_records.insert(metadata_record(record, "unsupported_unreviewed_stem"));
            }
        }
    }

    let mut language_rules = Vec::<LanguageRuleRecord>::new();
    for ((app, stem), format_records) in language_format_records {
        match distill_language_rule(&format_records)? {
            LanguageRuleDistillation::Rule(conditions) => {
                language_rules.push(LanguageRuleRecord {
                    key: LanguageRuleKey {
                        code: language_rule_code(&stem),
                        app,
                        stem,
                        conditions,
                    },
                    provenance: format_records,
                });
            }
            LanguageRuleDistillation::Inactive(reason) => {
                for record in &format_records {
                    inactive_records.insert(metadata_record(record, reason));
                }
            }
        }
    }
    language_rules.sort_by(|left, right| left.key.cmp(&right.key));

    seed_legacy_group_rules(&mut group_rules);

    let mut group_conflict_records = BTreeSet::<MetadataRuleRecord>::new();

    Ok(DistilledCatalog {
        group_rules: collapse_group_rules(group_rules, &mut group_conflict_records),
        service_alias_rules: service_alias_rules
            .into_iter()
            .map(|(key, provenance)| ServiceAliasRecord { key, provenance })
            .collect(),
        signal_rules: signal_rules
            .into_iter()
            .map(|(key, provenance)| SignalRuleRecord { key, provenance })
            .collect(),
        blocked_title_rules: blocked_title_rules
            .into_iter()
            .map(|(key, provenance)| BlockedTitleRecord { key, provenance })
            .collect(),
        fact_rules: fact_rules
            .into_iter()
            .map(|(key, provenance)| FactRuleRecord { key, provenance })
            .collect(),
        locale_group_fact_rules: locale_group_fact_rules
            .into_iter()
            .map(|(key, provenance)| LocaleGroupFactRuleRecord { key, provenance })
            .collect(),
        language_rules,
        no_release_group_facets: no_release_group_facets.into_iter().collect(),
        inactive_records: inactive_records.into_iter().collect(),
        ignored_records: ignored_records
            .into_iter()
            .chain(group_conflict_records)
            .collect(),
    })
}

/// One emitted fact code joined to a single upstream score entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FactScoreRow {
    code: String,
    app: String,
    score_set: String,
    score: i64,
}

/// Observed range of a score set, recorded so a wholesale change in upstream's
/// scale trips the coverage gate instead of being absorbed silently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ScoreSetEnvelope {
    score_set: String,
    min: i64,
    max: i64,
    vetoes: Vec<i64>,
}

/// Scores at or beyond this magnitude are vetoes rather than rankings; the
/// value matches `scryer.block_score()` and `BLOCK_SCORE`.
const VETO_SCORE_THRESHOLD: i64 = -10_000;

/// Cutoff assumed for a score set that publishes no veto at all.
const DEFAULT_VETO_MAGNITUDE: i64 = 10_000;

/// Fact codes upstream ships with no `trash_scores` at all. Their scores stay
/// Scryer-native, so the per-app guard treats them as reviewed; a fact that
/// arrives unscored without being listed here still fails the sync.
const UNSCORED_UPSTREAM_FACT_CODES: &[&str] = &[
    "trash.fansub",
    "trash.locale.french.marker.auvio",
    "trash.locale.french.marker.salto",
    "trash.locale.french.marker.vf2",
    "trash.locale.french.marker.vfb",
    "trash.locale.french.marker.vff",
    "trash.locale.french.marker.vfi",
    "trash.locale.french.marker.vfq",
    "trash.locale.french.marker.vof",
    "trash.locale.french.marker.voq",
    "trash.locale.french.marker.vq",
    "trash.locale.french.marker.wkn",
];

fn emitted_fact_provenance(catalog: &DistilledCatalog) -> Vec<(&str, &[UpstreamRecord])> {
    catalog
        .fact_rules
        .iter()
        .map(|rule| (rule.key.code.as_str(), rule.provenance.as_slice()))
        .chain(
            catalog
                .locale_group_fact_rules
                .iter()
                .map(|rule| (rule.key.code.as_str(), rule.provenance.as_slice())),
        )
        // Language rules are not parser facts, but they are scored the same way
        // and resolved through the same table, so they join here.
        .chain(
            catalog
                .language_rules
                .iter()
                .map(|rule| (rule.key.code.as_str(), rule.provenance.as_slice())),
        )
        .collect()
}

fn collect_fact_scores(catalog: &DistilledCatalog) -> Vec<FactScoreRow> {
    let mut rows = BTreeSet::new();
    for (code, provenance) in emitted_fact_provenance(catalog) {
        for record in provenance {
            for (score_set, score) in &record.scores {
                rows.insert(FactScoreRow {
                    code: code.to_string(),
                    app: record.app.clone(),
                    score_set: score_set.clone(),
                    score: *score,
                });
            }
        }
    }
    rows.into_iter().collect()
}

/// A score set's cutoff is the *smallest* veto it uses: sets carry vetoes
/// harder than their own cutoff, so `default` would otherwise read as 35000
/// because of its twelve -35000 entries.
fn score_set_veto_magnitudes(records: &[UpstreamRecord]) -> BTreeMap<String, i64> {
    let mut observed = BTreeMap::<String, Option<i64>>::new();
    for record in records {
        for (score_set, score) in &record.scores {
            let smallest = observed.entry(score_set.clone()).or_default();
            if *score <= VETO_SCORE_THRESHOLD {
                *smallest = Some(smallest.map_or(score.abs(), |current| current.min(score.abs())));
            }
        }
    }

    observed
        .into_iter()
        .map(|(score_set, smallest)| (score_set, smallest.unwrap_or(DEFAULT_VETO_MAGNITUDE)))
        .collect()
}

fn score_set_envelopes(records: &[UpstreamRecord]) -> Vec<ScoreSetEnvelope> {
    let mut observed = BTreeMap::<&str, (i64, i64, BTreeSet<i64>)>::new();
    for record in records {
        for (score_set, score) in &record.scores {
            let entry = observed
                .entry(score_set.as_str())
                .or_insert_with(|| (*score, *score, BTreeSet::new()));
            entry.0 = entry.0.min(*score);
            entry.1 = entry.1.max(*score);
            if *score <= VETO_SCORE_THRESHOLD {
                entry.2.insert(*score);
            }
        }
    }

    observed
        .into_iter()
        .map(|(score_set, (min, max, vetoes))| ScoreSetEnvelope {
            score_set: score_set.to_string(),
            min,
            max,
            vetoes: vetoes.into_iter().collect(),
        })
        .collect()
}

/// Emitted facts that carry no upstream score for an app that produced them.
///
/// Coverage is uneven upstream — a format can be scored in one app and not the
/// other — so a fact only counts as unscored for the app whose custom format
/// omits every score set. A global check would fail against valid upstream data.
///
/// `guide-only` is simply another app here: its five formats all publish
/// `trash_scores`, so they answer the guard on their own without any
/// cross-app borrowing.
fn unscored_fact_apps(catalog: &DistilledCatalog) -> Vec<String> {
    let mut scored = BTreeMap::<(&str, &str), bool>::new();
    for (code, provenance) in emitted_fact_provenance(catalog) {
        if UNSCORED_UPSTREAM_FACT_CODES.contains(&code) {
            continue;
        }
        for record in provenance {
            let entry = scored.entry((record.app.as_str(), code)).or_default();
            *entry |= !record.scores.is_empty();
        }
    }

    scored
        .into_iter()
        .filter(|(_, scored)| !scored)
        .map(|((app, code), _)| format!("{app}/{code}"))
        .collect()
}

fn seed_legacy_group_rules(group_rules: &mut BTreeMap<GroupRuleKey, Vec<UpstreamRecord>>) {
    for seed in LEGACY_SEED_GROUPS {
        let facets: &[DistilledFacet] = match seed.context {
            DistilledContext::Anime => &[DistilledFacet::Anime],
            DistilledContext::Any => &[
                DistilledFacet::Movie,
                DistilledFacet::Series,
                DistilledFacet::Anime,
            ],
            _ => &[DistilledFacet::Movie, DistilledFacet::Series],
        };

        for facet in facets {
            let record = UpstreamRecord {
                app: "scryer".to_string(),
                stem: "legacy-release-groups".to_string(),
                source_path: "scryer://legacy-release-groups".to_string(),
                trash_id: String::new(),
                cf_name: "Legacy release-group carry-forward".to_string(),
                spec_name: seed.name.to_string(),
                implementation: "ScryerLegacySeed".to_string(),
                value: format!("{:?}:{:?}:{:?}", seed.tier, seed.context, facet),
                required_json: "null".to_string(),
                negate_json: "null".to_string(),
                scores: BTreeMap::new(),
            };
            let key = GroupRuleKey {
                matcher: seed.name.to_string(),
                match_kind: GroupMatchKindSpec::Exact,
                tier: seed.tier,
                facet: *facet,
                context: seed.context,
            };
            group_rules.entry(key).or_default().push(record);
        }
    }
}

fn collapse_group_rules(
    group_rules: BTreeMap<GroupRuleKey, Vec<UpstreamRecord>>,
    ignored_records: &mut BTreeSet<MetadataRuleRecord>,
) -> Vec<GroupRuleRecord> {
    let mut grouped =
        BTreeMap::<GroupRuleIdentity, BTreeMap<DistilledTier, Vec<UpstreamRecord>>>::new();

    for (key, provenance) in group_rules {
        let identity = GroupRuleIdentity {
            matcher: key.matcher,
            match_kind: key.match_kind,
            facet: key.facet,
            context: key.context,
        };
        grouped
            .entry(identity)
            .or_default()
            .entry(key.tier)
            .or_default()
            .extend(provenance);
    }

    grouped
        .into_iter()
        .map(|(identity, tier_records)| {
            let selected_tier = select_group_tier(tier_records.keys().copied());
            let mut provenance = tier_records
                .get(&selected_tier)
                .cloned()
                .unwrap_or_default();
            provenance.sort();
            provenance.dedup();

            for (tier, records) in tier_records {
                if tier == selected_tier {
                    continue;
                }
                for record in records {
                    ignored_records.insert(metadata_record(
                        &record,
                        "group_tier_conflict_lower_precedence",
                    ));
                }
            }

            GroupRuleRecord {
                key: GroupRuleKey {
                    matcher: identity.matcher,
                    match_kind: identity.match_kind,
                    tier: selected_tier,
                    facet: identity.facet,
                    context: identity.context,
                },
                provenance,
            }
        })
        .collect()
}

fn select_group_tier(tiers: impl IntoIterator<Item = DistilledTier>) -> DistilledTier {
    let mut saw_bronze = false;
    let mut saw_silver = false;
    let mut saw_gold = false;

    for tier in tiers {
        match tier {
            DistilledTier::Banned => return DistilledTier::Banned,
            DistilledTier::Gold => saw_gold = true,
            DistilledTier::Silver => saw_silver = true,
            DistilledTier::Bronze => saw_bronze = true,
        }
    }

    if saw_gold {
        DistilledTier::Gold
    } else if saw_silver {
        DistilledTier::Silver
    } else {
        let _ = saw_bronze;
        DistilledTier::Bronze
    }
}

fn metadata_record(record: &UpstreamRecord, reason: &str) -> MetadataRuleRecord {
    MetadataRuleRecord {
        app: record.app.clone(),
        facet: facet_for_record(record),
        stem: record.stem.clone(),
        trash_id: record.trash_id.clone(),
        cf_name: record.cf_name.clone(),
        spec_name: record.spec_name.clone(),
        implementation: record.implementation.clone(),
        value: record.value.clone(),
        reason: reason.to_string(),
        source_path: record.source_path.clone(),
    }
}

fn classify_active_group_stem(stem: &str) -> Option<(DistilledTier, DistilledContext)> {
    let tier = |value: &str| -> Option<DistilledTier> {
        let tier = value.parse::<u8>().ok()?;
        Some(match tier {
            1 => DistilledTier::Gold,
            2 => DistilledTier::Silver,
            _ => DistilledTier::Bronze,
        })
    };

    if let Some(value) = stem.strip_prefix("web-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::Web));
    }
    if let Some(value) = stem.strip_prefix("hd-bluray-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::BluRay));
    }
    if let Some(value) = stem.strip_prefix("uhd-bluray-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::UhdBluRay));
    }
    if let Some(value) = stem.strip_prefix("remux-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::Remux));
    }
    if let Some(value) = stem.strip_prefix("anime-bd-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::Anime));
    }
    if let Some(value) = stem.strip_prefix("anime-web-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::Anime));
    }

    match stem {
        "lq" | "bad-dual-groups" => Some((DistilledTier::Banned, DistilledContext::Any)),
        "anime-lq-groups" => Some((DistilledTier::Banned, DistilledContext::Anime)),
        _ => None,
    }
}

fn facet_for_record(record: &UpstreamRecord) -> DistilledFacet {
    if is_anime_stem(&record.stem) {
        return DistilledFacet::Anime;
    }

    match record.app.as_str() {
        "radarr" => DistilledFacet::Movie,
        "sonarr" => DistilledFacet::Series,
        _ => DistilledFacet::Series,
    }
}

fn is_anime_stem(stem: &str) -> bool {
    stem.starts_with("anime-") || matches!(stem, "fansub" | "fastsub" | "dubs-only")
}

fn is_localized_stem(stem: &str) -> bool {
    stem.starts_with("french-") || stem.starts_with("german-") || stem.starts_with("asian-")
}

fn locale_stem_has_managed_score(stem: &str) -> bool {
    stem.contains("tier-01")
        || stem.contains("tier-02")
        || stem.contains("tier-03")
        || stem.ends_with("-lq")
        || stem.ends_with("-scene")
        || matches!(
            stem,
            "french-vostfr"
                | "french-vff"
                | "french-vfi"
                | "french-vof"
                | "french-vfq"
                | "french-vq"
                | "french-voq"
                | "german-subbed"
        )
}

fn classify_stem(stem: &str) -> StemClassification {
    if classify_active_group_stem(stem).is_some() {
        return StemClassification {
            detection_owner: DetectionOwner::ExistingNative,
            effect_binding: EffectBinding::PersonaScore,
            reason: "reviewed_release_group_tier",
        };
    }
    if is_localized_stem(stem) {
        return StemClassification {
            detection_owner: DetectionOwner::NativeFact,
            effect_binding: if locale_stem_has_managed_score(stem) {
                EffectBinding::LocaleScore
            } else {
                EffectBinding::Informational
            },
            reason: "reviewed_locale_fact",
        };
    }
    // Language conditions need metadata the parser never sees — the
    // title's original language — so detection lives in the managed policy that
    // reads the rule input, not in a parser fact.
    if is_language_rule_stem_name(stem) {
        return match undistillable_language_stem_reason(stem) {
            Some(reason) => StemClassification {
                detection_owner: DetectionOwner::Unsupported,
                effect_binding: EffectBinding::None,
                reason,
            },
            None => StemClassification {
                detection_owner: DetectionOwner::ManagedRego,
                effect_binding: EffectBinding::LocaleScore,
                reason: "reviewed_language_rule",
            },
        };
    }
    if stem == "anime-dual-audio" {
        return StemClassification {
            detection_owner: DetectionOwner::ExistingNative,
            effect_binding: EffectBinding::PersonaScore,
            reason: "existing_parser_or_scoring_signal",
        };
    }
    if is_service_alias_stem(stem) {
        return StemClassification {
            detection_owner: DetectionOwner::ExistingNative,
            effect_binding: EffectBinding::Informational,
            reason: "reviewed_service_alias",
        };
    }
    if excluded_service_alias_reason(stem).is_some() {
        return StemClassification {
            detection_owner: DetectionOwner::Unsupported,
            effect_binding: EffectBinding::None,
            reason: "reviewed_service_alias_exclusion",
        };
    }
    if matches!(
        stem,
        "upscaled"
            | "repack-proper"
            | "repack2"
            | "repack3"
            | "dubs-only"
            | "anime-raws"
            | "lq-release-title"
            | "fansub"
            | "fastsub"
            | "scene"
            | "obfuscated"
            | "retags"
            | "no-rlsgroup"
    ) {
        return StemClassification {
            detection_owner: DetectionOwner::NativeFact,
            effect_binding: if matches!(
                stem,
                "anime-raws" | "lq-release-title" | "fansub" | "fastsub"
            ) {
                EffectBinding::HardBlock
            } else if stem == "no-rlsgroup" {
                EffectBinding::Informational
            } else {
                EffectBinding::PersonaScore
            },
            reason: "reviewed_native_fact",
        };
    }
    if matches!(
        stem,
        "hdr"
            | "hdr10plus-boost"
            | "dv-boost"
            | "dv-disk"
            | "dv-wo-hdr-fallback"
            | "hybrid"
            | "remaster"
            | "truehd-atmos"
            | "ddplus-atmos"
            | "10bit"
            | "10-mono"
            | "20-stereo"
            | "line-mic-dubbed"
    ) {
        return StemClassification {
            detection_owner: DetectionOwner::CustomRuleOnly,
            effect_binding: EffectBinding::None,
            reason: "existing_parser_or_scoring_signal",
        };
    }
    StemClassification {
        detection_owner: DetectionOwner::Unsupported,
        effect_binding: EffectBinding::None,
        reason: "unsupported_unreviewed_stem",
    }
}

fn is_service_alias_stem(stem: &str) -> bool {
    service_alias_spec(stem).is_some()
}

fn record_is_negated(record: &UpstreamRecord) -> bool {
    record.negate_json == "true"
}

fn record_is_required(record: &UpstreamRecord) -> bool {
    record.required_json == "true"
}

fn record_is_sufficient_fact(records: &[UpstreamRecord], record: &UpstreamRecord) -> bool {
    let required = records
        .iter()
        .filter(|candidate| candidate.app == record.app && candidate.trash_id == record.trash_id)
        .filter(|candidate| record_is_required(candidate))
        .collect::<Vec<_>>();

    required.is_empty()
        || (required.len() == 1
            && required[0].implementation == record.implementation
            && required[0].spec_name == record.spec_name
            && required[0].value == record.value)
}

fn native_fact_code(stem: &str) -> &'static str {
    match stem {
        "scene" => "trash.scene",
        "obfuscated" => "trash.obfuscated",
        "retags" => "trash.retagged",
        "no-rlsgroup" => "trash.no_release_group",
        _ => unreachable!("native fact code requested for unsupported stem {stem}"),
    }
}

fn fact_codes_for_signal(kind: ParserSignalKindSpec) -> &'static [&'static str] {
    match kind {
        ParserSignalKindSpec::AiEnhanced => &["trash.ai_enhanced"],
        ParserSignalKindSpec::Proper => &["trash.proper"],
        ParserSignalKindSpec::Repack => &["trash.proper", "trash.repack"],
        ParserSignalKindSpec::DubsOnly => &["trash.dubs_only"],
        ParserSignalKindSpec::HardcodedSubs => &["trash.hardcoded_subs"],
    }
}

fn locale_fact_code(stem: &str) -> String {
    let (locale, semantic) = stem
        .split_once('-')
        .expect("localized stems always include a locale separator");
    let semantic = if semantic.contains("tier-01") {
        "group.tier1".to_string()
    } else if semantic.contains("tier-02") {
        "group.tier2".to_string()
    } else if semantic.contains("tier-03") {
        "group.tier3".to_string()
    } else if semantic.contains("lq") {
        "lq".to_string()
    } else if semantic.contains("scene") {
        "scene".to_string()
    } else {
        format!("marker.{}", semantic.replace('-', "_"))
    };
    format!("trash.locale.{locale}.{semantic}")
}

// ---------------------------------------------------------------------------
// Language rules
// ---------------------------------------------------------------------------

/// One Sonarr/Radarr `Language` enum id.
///
/// Ids are identical in both apps (verified against both `Language.cs`), so a
/// single table serves the whole distillation. `code` is the canonical audio
/// language code Scryer publishes in `input.release.languages_audio` — the
/// output of `normalize_detected_audio_language_code`, which is ISO 639-2/T
/// lowercase — so a distilled condition compares against the rule input without
/// any further translation. `name` is upstream's own English name, used to
/// recognise the release-title specifications that restate a language condition.
struct LanguageId {
    id: i64,
    name: &'static str,
    code: &'static str,
}

/// Upstream's relative "Original" language: the title's own original language
/// rather than a fixed one.
const ORIGINAL_LANGUAGE_ID: i64 = -2;

const LANGUAGE_IDS: &[LanguageId] = &[
    LanguageId {
        id: 1,
        name: "english",
        code: "eng",
    },
    LanguageId {
        id: 2,
        name: "french",
        code: "fra",
    },
    LanguageId {
        id: 3,
        name: "spanish",
        code: "spa",
    },
    LanguageId {
        id: 4,
        name: "german",
        code: "deu",
    },
    LanguageId {
        id: 5,
        name: "italian",
        code: "ita",
    },
    LanguageId {
        id: 6,
        name: "danish",
        code: "dan",
    },
    LanguageId {
        id: 7,
        name: "dutch",
        code: "nld",
    },
    LanguageId {
        id: 8,
        name: "japanese",
        code: "jpn",
    },
    LanguageId {
        id: 9,
        name: "icelandic",
        code: "isl",
    },
    LanguageId {
        id: 10,
        name: "chinese",
        code: "zho",
    },
    LanguageId {
        id: 11,
        name: "russian",
        code: "rus",
    },
    LanguageId {
        id: 12,
        name: "polish",
        code: "pol",
    },
    LanguageId {
        id: 13,
        name: "vietnamese",
        code: "vie",
    },
    LanguageId {
        id: 14,
        name: "swedish",
        code: "swe",
    },
    LanguageId {
        id: 15,
        name: "norwegian",
        code: "nor",
    },
    LanguageId {
        id: 16,
        name: "finnish",
        code: "fin",
    },
    LanguageId {
        id: 17,
        name: "turkish",
        code: "tur",
    },
    LanguageId {
        id: 18,
        name: "portuguese",
        code: "por",
    },
    // Scryer's ISO table is `Dutch; Flemish` under the single canonical code
    // `nld`, so Flemish resolves to Dutch rather than to a code the rule input
    // would never carry. A format naming both ids therefore yields one
    // condition after deduplication, which is the same set of releases.
    LanguageId {
        id: 19,
        name: "flemish",
        code: "nld",
    },
    LanguageId {
        id: 20,
        name: "greek",
        code: "ell",
    },
    LanguageId {
        id: 21,
        name: "korean",
        code: "kor",
    },
    LanguageId {
        id: 22,
        name: "hungarian",
        code: "hun",
    },
    LanguageId {
        id: 23,
        name: "hebrew",
        code: "heb",
    },
    LanguageId {
        id: 24,
        name: "lithuanian",
        code: "lit",
    },
];

/// Language formats whose *required* specifications include a constraint no
/// language condition can express. Distilling them anyway would silently widen
/// what upstream matches, so they are captured as audit records instead.
const UNDISTILLABLE_LANGUAGE_STEMS: &[(&str, &str)] = &[(
    "language-original-plus-french",
    "language_rule_requires_unsupported_release_title_spec",
)];

fn language_id_code(id: i64) -> Option<&'static str> {
    LANGUAGE_IDS
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.code)
}

fn language_name_code(name: &str) -> Option<&'static str> {
    LANGUAGE_IDS
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.code)
}

/// Stems whose custom formats are language policies rather than detection.
///
/// Everything under `guide-only/` is one; in the per-app trees they are the
/// `language-*` family plus the `not-…-or-english` vetoes.
fn is_language_rule_stem(app: &str, stem: &str) -> bool {
    app == GUIDE_ONLY_APP || is_language_rule_stem_name(stem)
}

fn is_language_rule_stem_name(stem: &str) -> bool {
    stem.starts_with("language-") || (stem.starts_with("not-") && stem.ends_with("-or-english"))
}

fn undistillable_language_stem_reason(stem: &str) -> Option<&'static str> {
    UNDISTILLABLE_LANGUAGE_STEMS
        .iter()
        .find(|(candidate, _)| *candidate == stem)
        .map(|(_, reason)| *reason)
}

/// Stable rule code for a language format.
///
/// The `language-` prefix is redundant under `trash.lang.`, so it is dropped;
/// the `not-…-or-english` stems keep their whole name because it *is* the
/// policy.
fn language_rule_code(stem: &str) -> String {
    let base = stem.strip_prefix("language-").unwrap_or(stem);
    format!("trash.lang.{}", base.replace('-', "_"))
}

/// The single word a release-title specification asserts, when it is nothing
/// more than one word between word boundaries.
///
/// Anything richer — alternations, lookarounds, character classes — returns
/// `None`, because only the bare form can be checked for redundancy against a
/// language condition.
fn language_title_marker_word(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    let pattern = pattern.strip_prefix("(?i)").unwrap_or(pattern);
    let inner = pattern.strip_prefix(r"\b")?.strip_suffix(r"\b")?;
    if inner.is_empty() || !inner.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    Some(inner.to_ascii_lowercase())
}

/// Whether a required non-language specification is already implied by the
/// format's own language conditions.
///
/// The only sound direction is the negated one. Scryer derives
/// `release.languages_audio` from the same release title upstream is matching,
/// so "the audio languages do not include German" implies "the title does not
/// say GERMAN"; keeping the title check would narrow nothing. The positive
/// direction does not hold — an audio language can come from the indexer rather
/// than the title — so a positive title requirement is never dropped.
fn language_title_spec_is_subsumed(
    record: &UpstreamRecord,
    conditions: &BTreeSet<LanguageConditionEntry>,
) -> bool {
    if record.implementation != "ReleaseTitleSpecification" || !record_is_negated(record) {
        return false;
    }
    let Some(word) = language_title_marker_word(&record.value) else {
        return false;
    };
    let Some(code) = language_name_code(&word) else {
        return false;
    };
    conditions.contains(&LanguageConditionEntry {
        language: LanguageConditionSpec::Named(code.to_string()),
        negate: true,
        required: true,
    })
}

/// One distilled language format, or the reason it stayed out of the table.
#[derive(Debug)]
enum LanguageRuleDistillation {
    Rule(Vec<LanguageConditionEntry>),
    Inactive(&'static str),
}

/// Turn one custom format's specifications into the condition list the managed
/// packs evaluate.
///
/// Upstream decides a format with `SpecificationMatchesGroup.DidMatch`: no
/// required specification may fail, and at least one must match. Only the
/// language specifications survive here, so the non-language ones have to be
/// accounted for rather than dropped silently:
///
/// * an *optional* non-language specification only ever contributed to "at least
///   one matched", which a required condition already satisfies, so it is safe to
///   drop whenever the format has one — and lossy when it does not;
/// * a *required* non-language specification gates the format, so it is dropped
///   only when a language condition already implies it, and otherwise takes the
///   whole format out of the table.
fn distill_language_rule(records: &[UpstreamRecord]) -> Result<LanguageRuleDistillation> {
    let mut conditions = BTreeSet::<LanguageConditionEntry>::new();
    let mut other_specs = Vec::new();

    for record in records {
        if record.implementation != "LanguageSpecification" {
            other_specs.push(record);
            continue;
        }

        let fields = serde_json::from_str::<Value>(&record.value).with_context(|| {
            format!(
                "language specification fields in {} are not JSON",
                record.source_path
            )
        })?;
        // `exceptLanguage` inverts the specification into "carries some language
        // other than this one", which is a different question from the one the
        // condition list asks. Upstream only ships it as `false` today; a `true`
        // has to be designed for rather than silently misread.
        if fields
            .get("exceptLanguage")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(LanguageRuleDistillation::Inactive(
                "language_rule_uses_except_language",
            ));
        }
        let id = fields.get("value").and_then(Value::as_i64).ok_or_else(|| {
            anyhow!(
                "language specification in {} has no integer value",
                record.source_path
            )
        })?;
        let language = if id == ORIGINAL_LANGUAGE_ID {
            LanguageConditionSpec::Original
        } else {
            LanguageConditionSpec::Named(
                language_id_code(id)
                    .ok_or_else(|| {
                        anyhow!(
                            "unmapped TRaSH language id {id} in {}; add it to LANGUAGE_IDS",
                            record.source_path
                        )
                    })?
                    .to_string(),
            )
        };

        conditions.insert(LanguageConditionEntry {
            language,
            negate: record_is_negated(record),
            required: record_is_required(record),
        });
    }

    if conditions.is_empty() {
        return Ok(LanguageRuleDistillation::Inactive(
            "language_rule_has_no_language_specification",
        ));
    }

    let has_required_condition = conditions.iter().any(|condition| condition.required);
    for record in other_specs {
        if record_is_required(record) {
            if !language_title_spec_is_subsumed(record, &conditions) {
                return Ok(LanguageRuleDistillation::Inactive(
                    "language_rule_requires_unsupported_release_title_spec",
                ));
            }
        } else if !has_required_condition {
            // With no required condition the format matches on "at least one
            // specification matched", so discarding an optional one narrows it.
            return Ok(LanguageRuleDistillation::Inactive(
                "language_rule_drops_a_matching_optional_spec",
            ));
        }
    }

    Ok(LanguageRuleDistillation::Rule(
        conditions.into_iter().collect(),
    ))
}

fn locale_group_source_context(stem: &str) -> Option<DistilledContext> {
    if stem.contains("anime-") && stem.contains("tier-") {
        Some(DistilledContext::Anime)
    } else if stem.contains("remux-tier-") {
        Some(DistilledContext::Remux)
    } else if stem.contains("uhd-bluray-tier-") {
        Some(DistilledContext::UhdBluRay)
    } else if stem.contains("bluray-tier-") {
        Some(DistilledContext::BluRay)
    } else if stem.contains("web-tier-") {
        Some(DistilledContext::Web)
    } else if stem.starts_with("asian-tier-") || stem.ends_with("-lq") || stem.ends_with("-scene") {
        Some(DistilledContext::Any)
    } else {
        None
    }
}

fn distill_group_matchers(record: &UpstreamRecord) -> Result<Vec<(String, GroupMatchKindSpec)>> {
    let pattern = record.value.trim();
    if pattern.is_empty() {
        bail!("empty release-group pattern for {}", record.source_path);
    }

    if pattern == r"Pahe(\.(ph|in))?\b" {
        return Ok(vec![
            ("Pahe".to_string(), GroupMatchKindSpec::Exact),
            ("Pahe.ph".to_string(), GroupMatchKindSpec::Exact),
            ("Pahe.in".to_string(), GroupMatchKindSpec::Exact),
        ]);
    }

    let inner = strip_group_anchors(pattern);
    if inner.ends_with(".*") && !inner[..inner.len() - 2].contains('|') {
        return Ok(vec![(
            unescape_group_literal(inner.trim_end_matches(".*"))?,
            GroupMatchKindSpec::Prefix,
        )]);
    }

    let expanded = finite_group_literals(pattern)?;
    if !expanded.is_empty() {
        return Ok(expanded
            .into_iter()
            .map(|matcher| (matcher, GroupMatchKindSpec::Exact))
            .collect());
    }

    if inner.starts_with('(') && inner.ends_with(')') {
        let inner = &inner[1..inner.len() - 1];
        let alternatives = split_top_level_alternatives(inner);
        if alternatives.len() > 1 {
            return alternatives
                .into_iter()
                .map(|value| {
                    if value.ends_with(".*") {
                        Ok((
                            unescape_group_literal(value.trim_end_matches(".*"))?,
                            GroupMatchKindSpec::Prefix,
                        ))
                    } else {
                        Ok((unescape_group_literal(value)?, GroupMatchKindSpec::Exact))
                    }
                })
                .collect();
        }

        if let Some(value) = alternatives.first() {
            if value.ends_with(".*") {
                return Ok(vec![(
                    unescape_group_literal(value.trim_end_matches(".*"))?,
                    GroupMatchKindSpec::Prefix,
                )]);
            }

            return Ok(vec![(
                unescape_group_literal(value)?,
                GroupMatchKindSpec::Exact,
            )]);
        }
    }

    Ok(vec![(
        unescape_group_literal(inner)?,
        GroupMatchKindSpec::Exact,
    )])
}

const MAX_FINITE_REGEX_EXPANSIONS: usize = 4096;

fn finite_group_literals(pattern: &str) -> Result<BTreeSet<String>> {
    let hir = match RegexParser::new().parse(pattern) {
        Ok(hir) => hir,
        Err(_) => return Ok(BTreeSet::new()),
    };
    let mut values = expand_finite_hir(&hir)?;
    values.retain(|value| !value.is_empty());
    Ok(values)
}

fn distill_scene_group_matchers(record: &UpstreamRecord) -> Result<Vec<String>> {
    let Some(start) = record.value.find(r"|\b(-") else {
        return Ok(Vec::new());
    };
    Ok(finite_group_literals(&record.value[start + 1..])?
        .into_iter()
        .filter_map(|value| value.strip_prefix('-').map(str::to_string))
        .collect())
}

fn distill_terminal_group_matcher(record: &UpstreamRecord) -> Option<String> {
    let value = record.value.trim().strip_suffix(r"\b")?;
    let value = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('_'))?;
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric())
        .then(|| value.to_string())
}

fn expand_finite_hir(hir: &Hir) -> Result<BTreeSet<String>> {
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => Ok(BTreeSet::from([String::new()])),
        HirKind::Literal(literal) => Ok(BTreeSet::from([
            String::from_utf8(literal.0.to_vec()).context("regex literal was not UTF-8")?
        ])),
        HirKind::Class(Class::Unicode(class)) => {
            let mut values = BTreeSet::new();
            for range in class.iter() {
                for value in range.start()..=range.end() {
                    values.insert(value.to_string());
                    if values.len() > MAX_FINITE_REGEX_EXPANSIONS {
                        return Ok(BTreeSet::new());
                    }
                }
            }
            Ok(values)
        }
        HirKind::Class(Class::Bytes(class)) => {
            let mut values = BTreeSet::new();
            for range in class.iter() {
                for value in range.start()..=range.end() {
                    if value.is_ascii() {
                        values.insert(char::from(value).to_string());
                    }
                    if values.len() > MAX_FINITE_REGEX_EXPANSIONS {
                        return Ok(BTreeSet::new());
                    }
                }
            }
            Ok(values)
        }
        HirKind::Capture(capture) => expand_finite_hir(&capture.sub),
        HirKind::Concat(parts) => {
            let mut values = BTreeSet::from([String::new()]);
            for part in parts.iter() {
                values = concatenate_expansions(&values, &expand_finite_hir(part)?)?;
                if values.is_empty() {
                    break;
                }
            }
            Ok(values)
        }
        HirKind::Alternation(parts) => {
            let mut values = BTreeSet::new();
            for part in parts.iter() {
                values.extend(expand_finite_hir(part)?);
                if values.len() > MAX_FINITE_REGEX_EXPANSIONS {
                    return Ok(BTreeSet::new());
                }
            }
            Ok(values)
        }
        HirKind::Repetition(repetition) => {
            let Some(max) = repetition.max else {
                return Ok(BTreeSet::new());
            };
            if max > 8 {
                return Ok(BTreeSet::new());
            }
            let repeated = expand_finite_hir(&repetition.sub)?;
            if repeated.is_empty() {
                return Ok(BTreeSet::new());
            }
            let mut current = BTreeSet::from([String::new()]);
            let mut values = BTreeSet::new();
            for count in 0..=max {
                if count >= repetition.min {
                    values.extend(current.iter().cloned());
                }
                if count < max {
                    current = concatenate_expansions(&current, &repeated)?;
                    if current.is_empty() {
                        break;
                    }
                }
            }
            Ok(values)
        }
    }
}

fn concatenate_expansions(
    left: &BTreeSet<String>,
    right: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    if left.is_empty() || right.is_empty() {
        return Ok(BTreeSet::new());
    }
    if left.len().saturating_mul(right.len()) > MAX_FINITE_REGEX_EXPANSIONS {
        return Ok(BTreeSet::new());
    }

    Ok(left
        .iter()
        .flat_map(|left| right.iter().map(move |right| format!("{left}{right}")))
        .collect())
}

fn distill_title_spec_group_matcher(
    record: &UpstreamRecord,
) -> Option<(String, GroupMatchKindSpec)> {
    let expected = sanitize_token(&record.spec_name)?;

    for alternative in split_top_level_alternatives(&record.value) {
        let mut candidate = alternative.trim();
        candidate = candidate.strip_prefix(r"\b").unwrap_or(candidate);
        candidate = candidate.strip_suffix(r"\b").unwrap_or(candidate);
        candidate = candidate.strip_prefix('(').unwrap_or(candidate);
        candidate = candidate.strip_suffix(')').unwrap_or(candidate);
        candidate = candidate.strip_prefix(r"\[").unwrap_or(candidate);
        candidate = candidate.strip_suffix(r"\]").unwrap_or(candidate);
        candidate = candidate.strip_prefix('-').unwrap_or(candidate);
        candidate = candidate.trim();
        candidate = candidate
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();

        let normalized = candidate
            .replace("[ .-]?", "")
            .replace("[ ._]?", "")
            .replace("[.-]?", "")
            .replace("[ _-]?", "");

        if normalized.contains(['|', '(', ')', '[', ']', '?', '+', '*', '{', '}']) {
            continue;
        }

        if sanitize_token(&normalized).as_deref() == Some(expected.as_str()) {
            return Some((record.spec_name.clone(), GroupMatchKindSpec::Exact));
        }
    }

    None
}

fn strip_group_anchors(pattern: &str) -> &str {
    let stripped = pattern
        .strip_prefix('^')
        .unwrap_or(pattern)
        .strip_suffix('$')
        .unwrap_or(pattern);
    stripped
        .strip_prefix(r"\b")
        .unwrap_or(stripped)
        .strip_suffix(r"\b")
        .unwrap_or(stripped)
}

fn split_top_level_alternatives(input: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut depth = 0_i32;
    let mut escaped = false;
    let mut start = 0_usize;

    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '(' => depth += 1,
            ')' => depth -= 1,
            '|' if depth == 0 => {
                output.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }

    output.push(&input[start..]);
    output
}

fn unescape_group_literal(input: &str) -> Result<String> {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                let next = chars
                    .next()
                    .ok_or_else(|| anyhow!("unterminated escape in group pattern {input:?}"))?;
                match next {
                    '.' | '&' | '+' | '-' | '_' | '\'' | ' ' | '[' | ']' | '(' | ')' => {
                        output.push(next)
                    }
                    'b' => {}
                    other => output.push(other),
                }
            }
            '(' | ')' | '?' => {}
            _ => output.push(ch),
        }
    }
    let trimmed = output.trim();
    if trimmed.is_empty() {
        bail!("group literal reduced to empty string from {input:?}");
    }
    Ok(trimmed.to_string())
}

fn service_alias_spec(stem: &str) -> Option<&'static ServiceAliasSpec> {
    SERVICE_ALIAS_SPECS.iter().find(|spec| spec.stem == stem)
}

fn excluded_service_alias_reason(stem: &str) -> Option<&'static str> {
    EXCLUDED_SERVICE_ALIAS_STEMS
        .iter()
        .find(|(excluded, _)| *excluded == stem)
        .map(|(_, reason)| *reason)
}

/// Turns one upstream specification into alias rows under the §6a token policy.
///
/// `locale_owned` marks the records a second branch also classifies: for those
/// the locale branch records the audit entry, so failures here stay silent
/// rather than duplicating an inactive record under two reasons.
fn collect_service_alias_tokens(
    record: &UpstreamRecord,
    spec: &ServiceAliasSpec,
    locale_owned: bool,
    rules: &mut BTreeMap<ServiceAliasKey, Vec<UpstreamRecord>>,
    inactive_records: &mut BTreeSet<MetadataRuleRecord>,
) {
    if record.implementation != "ReleaseTitleSpecification" || record_is_negated(record) {
        if !locale_owned {
            inactive_records.insert(metadata_record(
                record,
                "service_alias_requires_positive_title_spec",
            ));
        }
        return;
    }
    let tokens = match distill_service_alias_tokens(record) {
        Ok(tokens) => tokens,
        Err(failure) => {
            inactive_records.insert(metadata_record(record, failure.reason()));
            return;
        }
    };
    for token in tokens {
        if spec
            .excluded_tokens
            .iter()
            .any(|(excluded, _)| *excluded == token)
        {
            continue;
        }
        let policy = spec
            .token_overrides
            .iter()
            .find(|(candidate, _)| *candidate == token)
            .map_or(spec.default_policy, |(_, policy)| *policy);
        rules
            .entry(ServiceAliasKey {
                token,
                service: spec.service.to_string(),
                requires_web_adjacency: policy.requires_web_adjacency(),
            })
            .or_default()
            .push(record.clone());
    }
}

/// Why one specification produced no usable alias tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AliasDistillationFailure {
    /// The pattern's safety mechanism is a negative assertion (§6a rule 3).
    NegativeLookaround,
    /// The expansion produced nothing a single title token could match.
    NotLossless,
}

impl AliasDistillationFailure {
    fn reason(self) -> &'static str {
        match self {
            Self::NegativeLookaround => "service_alias_negative_lookaround",
            Self::NotLossless => "service_alias_pattern_not_lossless",
        }
    }
}

/// Aliases are the losslessly expanded literals of a release-title regex.
///
/// Splitting the raw pattern on non-alphanumerics is not safe here: it turns
/// `\b(FUNi(mation)?)\b` into `FUNI` plus a bogus `MATION`. Expanding the parsed
/// regex instead yields exactly the strings upstream intended to match.
fn distill_service_alias_tokens(
    record: &UpstreamRecord,
) -> Result<Vec<String>, AliasDistillationFailure> {
    let stripped =
        strip_lookarounds(&record.value).ok_or(AliasDistillationFailure::NegativeLookaround)?;
    let tokens = finite_group_literals(&stripped)
        .map_err(|_| AliasDistillationFailure::NotLossless)?
        .iter()
        .filter_map(|literal| sanitize_alias_token(literal))
        .collect::<BTreeSet<_>>();

    if tokens.is_empty() {
        return Err(AliasDistillationFailure::NotLossless);
    }

    Ok(tokens.into_iter().collect())
}

/// An alias is matched against one normalized title token, so only the first
/// alphanumeric run of an expansion can ever match: `stan[ ._-]web` tags a
/// release whose first token is `STAN`, never a token spelled `STANWEB`. Where a
/// separator is optional (`hbo[ ._-]?max`) the joined expansion supplies
/// `HBOMAX` on its own.
///
/// The token must also be at least two characters and contain a letter: bare
/// digits are Sonarr source enums and single characters match nearly any title.
fn sanitize_alias_token(raw: &str) -> Option<String> {
    let first_run = raw
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .find(|run| !run.is_empty())?;
    let token = sanitize_token(first_run)?;
    if token.len() < 2 || !token.chars().any(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    Some(token)
}

/// Drops *positive* lookaround groups so a pattern that uses them can still be
/// parsed and expanded. A positive assertion only narrows the context a service
/// tag appears in, so the tag text survives stripping unchanged, and
/// `regex-syntax` rejects the syntax outright.
///
/// Returns `None` for a negative assertion. by design those are
/// not strippable: the assertion *is* the pattern's safety mechanism, so
/// discarding it would widen the match rather than narrow it —
/// `(?<!dts[ .-]?hd[ .-]?)\b(ma|ykw)\b` would start tagging DTS-HD MA audio as a
/// streaming service.
fn strip_lookarounds(pattern: &str) -> Option<String> {
    const POSITIVE_OPENERS: [&str; 2] = ["(?=", "(?<="];
    const NEGATIVE_OPENERS: [&str; 2] = ["(?!", "(?<!"];
    let mut output = String::new();
    let mut rest = pattern;

    'outer: while !rest.is_empty() {
        for (index, _) in rest.char_indices() {
            if is_escaped(rest, index) {
                continue;
            }
            let candidate = &rest[index..];
            if NEGATIVE_OPENERS
                .iter()
                .any(|opener| candidate.starts_with(opener))
            {
                return None;
            }
            if POSITIVE_OPENERS
                .iter()
                .any(|opener| candidate.starts_with(opener))
            {
                output.push_str(&rest[..index]);
                match lookaround_end(candidate) {
                    Some(end) => {
                        rest = &candidate[end..];
                        continue 'outer;
                    }
                    // Unbalanced group: keep what came before and stop.
                    None => return Some(output),
                }
            }
        }
        output.push_str(rest);
        break;
    }

    Some(output)
}

fn is_escaped(pattern: &str, index: usize) -> bool {
    pattern[..index]
        .chars()
        .rev()
        .take_while(|ch| *ch == '\\')
        .count()
        % 2
        == 1
}

/// Byte offset just past the `)` closing the group that starts at `pattern[0]`.
fn lookaround_end(pattern: &str) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, ch) in pattern.char_indices() {
        if is_escaped(pattern, index) {
            continue;
        }
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn distill_named_patterns(
    record: &UpstreamRecord,
    _kind: ParserSignalKindSpec,
) -> Result<Vec<TokenPatternSpec>> {
    let mut patterns = BTreeSet::new();
    for pattern in pattern_from_spec_value(&record.value) {
        patterns.insert(pattern);
    }

    match record.stem.as_str() {
        "upscaled" if record.spec_name.eq_ignore_ascii_case("AI Upscales") => {
            patterns.insert(TokenPatternSpec {
                kind: TokenPatternKindSpec::RequiredTokens,
                tokens: vec!["AI".to_string(), "ENHANCED".to_string()],
            });
        }
        "upscaled" if record.spec_name.eq_ignore_ascii_case("Upscaled") => {
            for token in ["UPSCALED", "UPREZ"] {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec![token.to_string()],
                });
            }
        }
        "repack-proper" if record.spec_name.eq_ignore_ascii_case("Repack/Proper/Rerip") => {
            for token in ["PROPER", "REPACK", "RERIP"] {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec![token.to_string()],
                });
            }
        }
        "repack-proper"
            if record
                .spec_name
                .eq_ignore_ascii_case("Not Higher Version Repack/Proper") =>
        {
            for token in ["REPACK2", "REPACK3"] {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec![token.to_string()],
                });
            }
            patterns.insert(TokenPatternSpec {
                kind: TokenPatternKindSpec::RequiredTokens,
                tokens: vec!["REAL".to_string(), "PROPER".to_string()],
            });
            patterns.insert(TokenPatternSpec {
                kind: TokenPatternKindSpec::RequiredTokens,
                tokens: vec!["REAL".to_string(), "REPACK".to_string()],
            });
        }
        "repack2" => {
            for token in ["PROPER2", "REPACK2"] {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec![token.to_string()],
                });
            }
            for token in ["PROPER", "REPACK"] {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::RequiredTokens,
                    tokens: vec!["REAL".to_string(), token.to_string()],
                });
            }
        }
        "repack3" => {
            for token in ["PROPER3", "REPACK3"] {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec![token.to_string()],
                });
            }
        }
        "french-vfq" => {
            patterns.insert(TokenPatternSpec {
                kind: TokenPatternKindSpec::Sequence,
                tokens: vec!["VFQ".to_string()],
            });
        }
        "german-subbed" => {
            for language in ["GER", "GERMAN"] {
                for subtitle in ["OMU", "SUB", "SUBBED", "SUBS"] {
                    patterns.insert(TokenPatternSpec {
                        kind: TokenPatternKindSpec::RequiredTokens,
                        tokens: vec![language.to_string(), subtitle.to_string()],
                    });
                }
            }
        }
        _ => {}
    }

    if patterns.is_empty() {
        bail!(
            "failed to distill parser token patterns for {} {}",
            record.source_path,
            record.spec_name
        );
    }

    Ok(patterns.into_iter().collect())
}

fn distill_repack_patterns(
    record: &UpstreamRecord,
) -> Result<Vec<(ParserSignalKindSpec, TokenPatternSpec)>> {
    let mut output = Vec::new();
    for pattern in distill_named_patterns(record, ParserSignalKindSpec::Proper)? {
        let kind = if pattern
            .tokens
            .iter()
            .any(|token| token == "REPACK" || token == "RERIP")
        {
            ParserSignalKindSpec::Repack
        } else {
            ParserSignalKindSpec::Proper
        };
        output.push((kind, pattern));
    }
    Ok(output)
}

fn distill_dubs_only_patterns(record: &UpstreamRecord) -> Result<Vec<TokenPatternSpec>> {
    let mut patterns = BTreeSet::new();
    if record.spec_name.eq_ignore_ascii_case("Dubbed") {
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::Sequence,
            tokens: vec!["DUB".to_string()],
        });
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::Sequence,
            tokens: vec!["DUBBED".to_string()],
        });
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::RequiredTokens,
            tokens: vec!["ENG".to_string(), "DUB".to_string()],
        });
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::RequiredTokens,
            tokens: vec!["FUNI".to_string(), "DUB".to_string()],
        });
    }

    Ok(patterns.into_iter().collect())
}

fn distill_blocked_title_patterns(
    record: &UpstreamRecord,
    _code: &str,
) -> Result<Vec<TokenPatternSpec>> {
    let mut patterns = BTreeSet::new();
    for pattern in pattern_from_spec_value(&record.value) {
        patterns.insert(pattern);
    }

    if record.spec_name.starts_with("BiTOR") {
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::RequiredTokens,
            tokens: vec!["2160P".to_string(), "BITOR".to_string()],
        });
    }

    if patterns.is_empty() {
        bail!(
            "failed to distill blocked-title patterns for {} {}",
            record.source_path,
            record.spec_name
        );
    }

    Ok(patterns.into_iter().collect())
}

fn pattern_from_spec_value(value: &str) -> Vec<TokenPatternSpec> {
    let mut patterns = BTreeSet::new();
    if let Some(boundary_patterns) = simple_boundary_patterns(value) {
        patterns.extend(boundary_patterns);
    }
    if let Ok(literals) = finite_group_literals(value) {
        for literal in literals {
            if literal.chars().all(|ch| ch.is_ascii_alphanumeric())
                && let Some(token) = sanitize_token(&literal)
            {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec![token],
                });
            }
        }
    }

    if value.contains("(?=.*") {
        let tokens = extract_boundary_tokens(value);
        if tokens.len() >= 2 {
            patterns.insert(TokenPatternSpec {
                kind: TokenPatternKindSpec::RequiredTokens,
                tokens,
            });
        }
    }

    if value.contains("[ ._-]?") || value.contains("[ ._-]") {
        let cleaned = value
            .replace(r"\b", " ")
            .replace(r"[ ._-]?", " ")
            .replace(r"[ ._-]", " ")
            .replace(['(', ')', '[', ']', '^', '$', '?', '*', '|'], " ");
        let tokens = cleaned
            .split_whitespace()
            .filter_map(sanitize_token)
            .collect::<Vec<_>>();
        if tokens.len() >= 2 {
            patterns.insert(TokenPatternSpec {
                kind: TokenPatternKindSpec::Sequence,
                tokens,
            });
        }
    }

    patterns.into_iter().collect()
}

fn simple_boundary_patterns(value: &str) -> Option<Vec<TokenPatternSpec>> {
    let mut inner = value.trim().strip_prefix(r"\b")?.strip_suffix(r"\b")?;
    inner = inner.strip_prefix('(').unwrap_or(inner);
    inner = inner.strip_suffix(')').unwrap_or(inner);
    inner = inner.strip_prefix("?:").unwrap_or(inner);

    let mut patterns = Vec::new();
    for alternative in inner.split('|') {
        let alternative = alternative.trim();
        if alternative.is_empty() || !alternative.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            return None;
        }
        patterns.push(TokenPatternSpec {
            kind: TokenPatternKindSpec::Sequence,
            tokens: vec![sanitize_token(alternative)?],
        });
    }
    (!patterns.is_empty()).then_some(patterns)
}

fn sanitize_token(raw: &str) -> Option<String> {
    let token = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_uppercase())
        .collect::<String>();
    if token.is_empty() { None } else { Some(token) }
}

fn extract_boundary_tokens(pattern: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut input = pattern;
    while let Some(start) = input.find(r"\b") {
        let after_start = &input[start + 2..];
        let Some(end) = after_start.find(r"\b") else {
            break;
        };
        if let Some(token) = sanitize_token(&after_start[..end])
            && !tokens.contains(&token)
        {
            tokens.push(token);
        }
        input = &after_start[end + 2..];
    }
    tokens
}

fn write_if_changed(path: &Path, next: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    if fs::read_to_string(path).ok().as_deref() == Some(next) {
        return Ok(());
    }

    fs::write(path, next).with_context(|| format!("failed to write {}", path.display()))
}

fn format_generated_rust(ctx: &TaskContext) -> Result<()> {
    let mut command = ctx.command_in("rustfmt", &ctx.repo_root);
    command.args(["--edition", "2024", QUALITY_OUTPUT, PARSER_OUTPUT]);
    run_checked(&mut command).context("failed to rustfmt generated TRaSH outputs")
}

fn render_quality_output(
    catalog: &DistilledCatalog,
    records: &[UpstreamRecord],
    source_revision: &str,
) -> Result<String> {
    let mut output = String::new();
    writeln!(
        output,
        "// Generated by `cargo xtask trash-guides sync`.\n// Do not edit by hand.\n"
    )?;
    writeln!(
        output,
        "#[allow(dead_code)]\npub const TRASH_GUIDES_SOURCE_REVISION: &str = {};",
        rust_str(source_revision)
    )?;
    writeln!(output)?;
    writeln!(
        output,
        "pub static KNOWN_RELEASE_GROUP_RULES: &[KnownReleaseGroupRule] = &["
    )?;
    for (matcher, match_kind) in catalog
        .group_rules
        .iter()
        .map(|rule| (rule.key.matcher.as_str(), rule.key.match_kind))
        .collect::<BTreeSet<_>>()
    {
        writeln!(
            output,
            "    KnownReleaseGroupRule {{ matcher: {}, match_kind: GroupMatchKind::{} }},",
            rust_str(matcher),
            match match_kind {
                GroupMatchKindSpec::Exact => "Exact",
                GroupMatchKindSpec::Prefix => "Prefix",
            },
        )?;
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "#[cfg(test)]\npub static GROUP_RULES: &[GroupRule] = &["
    )?;
    for rule in &catalog.group_rules {
        writeln!(
            output,
            "    GroupRule {{ matcher: {}, match_kind: GroupMatchKind::{}, entry: GroupEntry {{ name: {}, tier: GroupTier::{}, facet: RuleFacet::{}, source_context: SourceContext::{} }} }},",
            rust_str(&rule.key.matcher),
            match rule.key.match_kind {
                GroupMatchKindSpec::Exact => "Exact",
                GroupMatchKindSpec::Prefix => "Prefix",
            },
            rust_str(&rule.key.matcher),
            render_tier(rule.key.tier),
            render_facet(rule.key.facet),
            render_context(rule.key.context),
        )?;
    }
    writeln!(output, "];\n")?;

    Ok(output)
}

fn render_parser_output(catalog: &DistilledCatalog, source_revision: &str) -> Result<String> {
    let mut output = String::new();
    writeln!(
        output,
        "// Generated by `cargo xtask trash-guides sync`.\n// Do not edit by hand.\n"
    )?;
    writeln!(
        output,
        "#[allow(dead_code)]\npub const TRASH_GUIDES_SOURCE_REVISION: &str = {};",
        rust_str(source_revision)
    )?;
    writeln!(output)?;

    writeln!(
        output,
        "pub static SERVICE_ALIAS_RULES: &[ServiceAliasRule] = &["
    )?;
    for rule in &catalog.service_alias_rules {
        writeln!(
            output,
            "    ServiceAliasRule {{ token: {}, service: {}, requires_web_adjacency: {} }},",
            rust_str(&rule.key.token),
            rust_str(&rule.key.service),
            rule.key.requires_web_adjacency,
        )?;
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "#[cfg(test)]\npub static FACT_RULES: &[FactRule] = &["
    )?;
    for rule in &catalog.fact_rules {
        writeln!(
            output,
            "    FactRule {{ code: {}, facet: RuleFacet::{}, category: TitleCategoryScope::{}, pattern: {} }},",
            rust_str(&rule.key.code),
            render_facet(rule.key.facet),
            match rule.key.category {
                CategoryScopeSpec::Any => "Any",
                CategoryScopeSpec::Anime => "Anime",
            },
            render_token_pattern(&rule.key.pattern),
        )?;
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "#[cfg(test)]\npub static LOCALE_GROUP_FACT_RULES: &[LocaleGroupFactRule] = &["
    )?;
    for rule in &catalog.locale_group_fact_rules {
        writeln!(
            output,
            "    LocaleGroupFactRule {{ code: {}, matcher: {}, match_kind: LocaleGroupMatchKind::{}, facet: RuleFacet::{}, source_context: LocaleSourceContext::{} }},",
            rust_str(&rule.key.code),
            rust_str(&rule.key.matcher),
            match rule.key.match_kind {
                GroupMatchKindSpec::Exact => "Exact",
                GroupMatchKindSpec::Prefix => "Prefix",
            },
            render_facet(rule.key.facet),
            render_context(rule.key.source_context),
        )?;
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "#[cfg(test)]\npub static NO_RELEASE_GROUP_FACT_FACETS: &[RuleFacet] = &["
    )?;
    for facet in &catalog.no_release_group_facets {
        writeln!(output, "    RuleFacet::{},", render_facet(*facet))?;
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "pub static TOKEN_SIGNAL_RULES: &[TokenSignalRule] = &["
    )?;
    for rule in &catalog.signal_rules {
        for provenance in &rule.provenance {
            writeln!(
                output,
                "    TokenSignalRule {{ kind: ParserSignalKind::{}, pattern: {}, facet: RuleFacet::{}, app: {}, stem: {}, trash_id: {}, cf_name: {}, spec_name: {}, source_path: {} }},",
                render_signal_kind(rule.key.kind),
                render_token_pattern(&rule.key.pattern),
                render_facet(facet_for_record(provenance)),
                rust_str(&provenance.app),
                rust_str(&provenance.stem),
                rust_str(&provenance.trash_id),
                rust_str(&provenance.cf_name),
                rust_str(&provenance.spec_name),
                rust_str(&provenance.source_path),
            )?;
        }
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "#[cfg(test)]\npub static BLOCKED_TITLE_RULES: &[BlockedTitleRule] = &["
    )?;
    for rule in &catalog.blocked_title_rules {
        for provenance in &rule.provenance {
            writeln!(
                output,
                "    BlockedTitleRule {{ code: {}, facet: RuleFacet::{}, category: TitleCategoryScope::{}, pattern: {}, app: {}, stem: {}, trash_id: {}, cf_name: {}, spec_name: {}, source_path: {} }},",
                rust_str(&rule.key.code),
                render_facet(rule.key.facet),
                match rule.key.category {
                    CategoryScopeSpec::Any => "Any",
                    CategoryScopeSpec::Anime => "Anime",
                },
                render_token_pattern(&rule.key.pattern),
                rust_str(&provenance.app),
                rust_str(&provenance.stem),
                rust_str(&provenance.trash_id),
                rust_str(&provenance.cf_name),
                rust_str(&provenance.spec_name),
                rust_str(&provenance.source_path),
            )?;
        }
    }
    writeln!(output, "];\n")?;

    Ok(output)
}

fn render_summary(catalog: &DistilledCatalog, source_revision: &str) -> String {
    let mut output = String::new();
    let (movie_groups, series_groups, anime_groups) =
        count_group_rules_by_facet(&catalog.group_rules);
    let (movie_titles, series_titles, anime_titles) =
        count_blocked_title_rules_by_facet(&catalog.blocked_title_rules);
    let _ = writeln!(output, "TRaSH Guides sync summary");
    let _ = writeln!(output, "Source revision: {source_revision}");
    let _ = writeln!(output);
    let _ = writeln!(
        output,
        "Active release-group rules: {}",
        catalog.group_rules.len()
    );
    let _ = writeln!(
        output,
        "  by facet: movies={movie_groups}, series={series_groups}, anime={anime_groups}"
    );
    let _ = writeln!(
        output,
        "Active service aliases: {}",
        catalog.service_alias_rules.len()
    );
    let _ = writeln!(
        output,
        "Active parser token signals: {}",
        catalog.signal_rules.len()
    );
    let _ = writeln!(output, "Active guide facts: {}", catalog.fact_rules.len());
    let _ = writeln!(
        output,
        "Active language rules: {}",
        catalog.language_rules.len()
    );
    let _ = writeln!(
        output,
        "Active blocked title rules: {}",
        catalog.blocked_title_rules.len()
    );
    let _ = writeln!(
        output,
        "  by facet: movies={movie_titles}, series={series_titles}, anime={anime_titles}"
    );
    let _ = writeln!(
        output,
        "Preserved inactive records: {}",
        catalog.inactive_records.len()
    );
    let _ = writeln!(output, "Ignored records: {}", catalog.ignored_records.len());
    output
}

fn render_stem_manifest(
    records: &[UpstreamRecord],
    catalog: &DistilledCatalog,
    source_revision: &str,
) -> Result<String> {
    let manifest = build_stem_manifest(records, catalog, source_revision);
    serde_json::to_string_pretty(&manifest)
        .map(|mut rendered| {
            rendered.push('\n');
            rendered
        })
        .context("failed to serialize TRaSH stem classification manifest")
}

#[derive(Debug, Default)]
struct StemEmission {
    rule_count: usize,
    fact_codes: BTreeSet<String>,
    rule_signatures: BTreeSet<String>,
}

fn build_stem_manifest(
    records: &[UpstreamRecord],
    catalog: &DistilledCatalog,
    source_revision: &str,
) -> StemClassificationManifest {
    let mut emissions = BTreeMap::<(String, String), StemEmission>::new();
    for rule in &catalog.group_rules {
        add_stem_emission(
            &mut emissions,
            &rule.provenance,
            &[],
            &format!("group:{:?}", rule.key),
        );
    }
    for rule in &catalog.service_alias_rules {
        add_stem_emission(
            &mut emissions,
            &rule.provenance,
            &[],
            &format!("service:{:?}", rule.key),
        );
    }
    for rule in &catalog.signal_rules {
        add_stem_emission(
            &mut emissions,
            &rule.provenance,
            fact_codes_for_signal(rule.key.kind),
            &format!("signal:{:?}", rule.key),
        );
    }
    for rule in &catalog.blocked_title_rules {
        add_stem_emission(
            &mut emissions,
            &rule.provenance,
            &[],
            &format!("blocked:{:?}", rule.key),
        );
    }
    for rule in &catalog.fact_rules {
        add_stem_emission(
            &mut emissions,
            &rule.provenance,
            &[rule.key.code.as_str()],
            &format!("fact:{:?}", rule.key),
        );
    }
    for rule in &catalog.locale_group_fact_rules {
        add_stem_emission(
            &mut emissions,
            &rule.provenance,
            &[rule.key.code.as_str()],
            &format!("locale_group:{:?}", rule.key),
        );
    }
    for rule in &catalog.language_rules {
        add_stem_emission(
            &mut emissions,
            &rule.provenance,
            &[rule.key.code.as_str()],
            &format!("language:{:?}", rule.key),
        );
    }

    let mut stems = BTreeMap::<(&str, &str), StemClassification>::new();
    for record in records {
        stems
            .entry((&record.app, &record.stem))
            .or_insert_with(|| classify_stem(&record.stem));
    }

    StemClassificationManifest {
        source_revision: source_revision.to_string(),
        score_envelope: score_set_envelopes(records),
        stems: stems
            .into_iter()
            .map(
                |((app, stem), classification)| StemClassificationManifestRecord {
                    app: app.to_string(),
                    stem: stem.to_string(),
                    detection_owner: classification.detection_owner,
                    effect_binding: classification.effect_binding,
                    reason: classification.reason.to_string(),
                    emitted_rule_count: emissions
                        .get(&(app.to_string(), stem.to_string()))
                        .map_or(0, |emission| emission.rule_count),
                    emitted_fact_codes: emissions
                        .get(&(app.to_string(), stem.to_string()))
                        .map(|emission| emission.fact_codes.iter().cloned().collect())
                        .unwrap_or_default(),
                    emitted_rule_digest: emissions
                        .get(&(app.to_string(), stem.to_string()))
                        .map(stable_emission_digest)
                        .unwrap_or_default(),
                },
            )
            .collect(),
        inactive_records: catalog
            .inactive_records
            .iter()
            .map(AuditRecord::from)
            .collect(),
        ignored_records: catalog
            .ignored_records
            .iter()
            .map(AuditRecord::from)
            .collect(),
    }
}

fn add_stem_emission(
    emissions: &mut BTreeMap<(String, String), StemEmission>,
    provenance: &[UpstreamRecord],
    fact_codes: &[&str],
    signature: &str,
) {
    let stems = provenance
        .iter()
        .map(|record| (record.app.clone(), record.stem.clone()))
        .collect::<BTreeSet<_>>();
    for stem in stems {
        let emission = emissions.entry(stem).or_default();
        emission.rule_count += 1;
        emission
            .fact_codes
            .extend(fact_codes.iter().map(|code| (*code).to_string()));
        emission.rule_signatures.insert(signature.to_string());
    }
}

fn stable_emission_digest(emission: &StemEmission) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for signature in &emission.rule_signatures {
        for byte in signature.bytes().chain(std::iter::once(0)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("fnv1a64:{hash:016x}")
}

fn enforce_stem_coverage(
    manifest_path: &Path,
    records: &[UpstreamRecord],
    catalog: &DistilledCatalog,
) -> Result<()> {
    let accepts_new_stems = accepts_new_stems();
    let known = match fs::read_to_string(manifest_path) {
        Ok(content) => {
            serde_json::from_str::<StemClassificationManifest>(&content).with_context(|| {
                format!(
                    "failed to parse reviewed TRaSH stem manifest {}",
                    manifest_path.display()
                )
            })?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && accepts_new_stems => {
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "reviewed TRaSH stem manifest {} is missing; set {ACCEPT_STEMS_ENV}=1 only to bootstrap or explicitly accept the fetched inventory",
                manifest_path.display()
            )
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", manifest_path.display()));
        }
    };

    let current = build_stem_manifest(records, catalog, "");
    let drift = score_envelope_drift(&known.score_envelope, &current.score_envelope);
    if !drift.is_empty() && !accepts_new_stems {
        bail!(
            "TRaSH score envelope moved outside the reviewed range: {}; confirm the mapping still treats vetoes as vetoes, then set {ACCEPT_STEMS_ENV}=1 to record the new envelope in {}",
            drift.join(", "),
            manifest_path.display()
        );
    }

    let known = known
        .stems
        .into_iter()
        .map(|record| ((record.app.clone(), record.stem.clone()), record))
        .collect::<BTreeMap<_, _>>();
    let current = current
        .stems
        .into_iter()
        .map(|record| ((record.app.clone(), record.stem.clone()), record))
        .collect::<BTreeMap<_, _>>();
    let keys = known
        .keys()
        .chain(current.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let changes = keys
        .into_iter()
        .filter(|key| match (known.get(key), current.get(key)) {
            (Some(known), Some(current)) => {
                known.detection_owner != current.detection_owner
                    || known.effect_binding != current.effect_binding
                    || known.reason != current.reason
                    || known.emitted_rule_count != current.emitted_rule_count
                    || known.emitted_fact_codes != current.emitted_fact_codes
                    || known.emitted_rule_digest != current.emitted_rule_digest
            }
            _ => true,
        })
        .map(|(app, stem)| format!("{app}/{stem}"))
        .collect::<Vec<_>>();
    if !changes.is_empty() && !accepts_new_stems {
        bail!(
            "unreviewed TRaSH stem behavior detected for: {}; inspect the generated effects in {} or set {ACCEPT_STEMS_ENV}=1 to intentionally accept them",
            changes.join(", "),
            manifest_path.display()
        );
    }
    Ok(())
}

/// Differences between the reviewed and observed score envelopes.
///
/// Any difference counts, not just a widening: the failure this guards against
/// is upstream *shrinking* its scale, where scores that used to veto quietly
/// stop doing so.
fn score_envelope_drift(known: &[ScoreSetEnvelope], current: &[ScoreSetEnvelope]) -> Vec<String> {
    let known = known
        .iter()
        .map(|envelope| (envelope.score_set.as_str(), envelope))
        .collect::<BTreeMap<_, _>>();
    let current = current
        .iter()
        .map(|envelope| (envelope.score_set.as_str(), envelope))
        .collect::<BTreeMap<_, _>>();

    known
        .keys()
        .chain(current.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(
            |score_set| match (known.get(score_set), current.get(score_set)) {
                (Some(known), Some(current)) if known == current => None,
                (Some(known), Some(current)) => Some(format!(
                    "{score_set} {}..{} vetoes {:?} -> {}..{} vetoes {:?}",
                    known.min, known.max, known.vetoes, current.min, current.max, current.vetoes
                )),
                (Some(_), None) => Some(format!("{score_set} disappeared")),
                (None, _) => Some(format!("{score_set} is new")),
            },
        )
        .collect()
}

#[cfg(test)]
fn unexpected_stems(
    known_stems: &BTreeSet<(String, String)>,
    records: &[UpstreamRecord],
) -> BTreeSet<(String, String)> {
    records
        .iter()
        .map(|record| (record.app.clone(), record.stem.clone()))
        .filter(|stem| !known_stems.contains(stem))
        .collect()
}

fn accepts_new_stems() -> bool {
    matches!(
        std::env::var(ACCEPT_STEMS_ENV).as_deref(),
        Ok("1" | "true" | "TRUE")
    )
}

fn count_group_rules_by_facet(rules: &[GroupRuleRecord]) -> (usize, usize, usize) {
    let mut movie = 0;
    let mut series = 0;
    let mut anime = 0;

    for rule in rules {
        match rule.key.facet {
            DistilledFacet::Movie => movie += 1,
            DistilledFacet::Series => series += 1,
            DistilledFacet::Anime => anime += 1,
        }
    }

    (movie, series, anime)
}

fn count_blocked_title_rules_by_facet(rules: &[BlockedTitleRecord]) -> (usize, usize, usize) {
    let mut movie = 0;
    let mut series = 0;
    let mut anime = 0;

    for rule in rules {
        match rule.key.facet {
            DistilledFacet::Movie => movie += 1,
            DistilledFacet::Series => series += 1,
            DistilledFacet::Anime => anime += 1,
        }
    }

    (movie, series, anime)
}

fn render_tier(tier: DistilledTier) -> &'static str {
    match tier {
        DistilledTier::Gold => "Gold",
        DistilledTier::Silver => "Silver",
        DistilledTier::Bronze => "Bronze",
        DistilledTier::Banned => "Banned",
    }
}

fn render_context(context: DistilledContext) -> &'static str {
    match context {
        DistilledContext::Web => "Web",
        DistilledContext::BluRay => "BluRay",
        DistilledContext::UhdBluRay => "UhdBluRay",
        DistilledContext::Remux => "Remux",
        DistilledContext::Anime => "Anime",
        DistilledContext::Any => "Any",
    }
}

fn render_facet(facet: DistilledFacet) -> &'static str {
    match facet {
        DistilledFacet::Movie => "Movie",
        DistilledFacet::Series => "Series",
        DistilledFacet::Anime => "Anime",
    }
}

fn render_signal_kind(kind: ParserSignalKindSpec) -> &'static str {
    match kind {
        ParserSignalKindSpec::AiEnhanced => "AiEnhanced",
        ParserSignalKindSpec::Proper => "Proper",
        ParserSignalKindSpec::Repack => "Repack",
        ParserSignalKindSpec::DubsOnly => "DubsOnly",
        ParserSignalKindSpec::HardcodedSubs => "HardcodedSubs",
    }
}

fn render_token_pattern(pattern: &TokenPatternSpec) -> String {
    let tokens = pattern
        .tokens
        .iter()
        .map(|token| rust_str(token))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "TokenPattern {{ kind: TokenPatternKind::{}, tokens: &[{}] }}",
        match pattern.kind {
            TokenPatternKindSpec::Sequence => "Sequence",
            TokenPatternKindSpec::RequiredTokens => "RequiredTokens",
        },
        tokens
    )
}

fn rust_str(value: &str) -> String {
    format!("{value:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_maps_to_release_group_contexts() {
        assert_eq!(
            classify_active_group_stem("web-tier-01"),
            Some((DistilledTier::Gold, DistilledContext::Web))
        );
        assert_eq!(
            classify_active_group_stem("uhd-bluray-tier-03"),
            Some((DistilledTier::Bronze, DistilledContext::UhdBluRay))
        );
        assert_eq!(
            classify_active_group_stem("anime-lq-groups"),
            Some((DistilledTier::Banned, DistilledContext::Anime))
        );
    }

    #[test]
    fn record_facets_follow_app_and_anime_rule_boundaries() {
        let movie = UpstreamRecord {
            app: "radarr".to_string(),
            stem: "web-tier-01".to_string(),
            source_path: String::new(),
            trash_id: String::new(),
            cf_name: String::new(),
            spec_name: String::new(),
            implementation: String::new(),
            value: String::new(),
            ..Default::default()
        };
        assert_eq!(facet_for_record(&movie), DistilledFacet::Movie);

        let series = UpstreamRecord {
            app: "sonarr".to_string(),
            ..movie.clone()
        };
        assert_eq!(facet_for_record(&series), DistilledFacet::Series);

        let anime = UpstreamRecord {
            stem: "anime-web-tier-01".to_string(),
            ..series
        };
        assert_eq!(facet_for_record(&anime), DistilledFacet::Anime);
    }

    #[test]
    fn release_group_regex_expansion_handles_alias_and_prefix_cases() {
        let record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "web-tier-01".to_string(),
            source_path: "docs/json/sonarr/cf/web-tier-01.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "WEB Tier 01".to_string(),
            spec_name: "APEX".to_string(),
            implementation: "ReleaseGroupSpecification".to_string(),
            value: "^(APEX|PAXA|PEXA|XEPA)$".to_string(),
            ..Default::default()
        };
        let expanded = distill_group_matchers(&record).expect("expand aliases");
        assert_eq!(
            expanded,
            vec![
                ("APEX".to_string(), GroupMatchKindSpec::Exact),
                ("PAXA".to_string(), GroupMatchKindSpec::Exact),
                ("PEXA".to_string(), GroupMatchKindSpec::Exact),
                ("XEPA".to_string(), GroupMatchKindSpec::Exact),
            ]
        );

        let prefix_record = UpstreamRecord {
            value: "^(alfaHD.*)$".to_string(),
            ..record
        };
        let expanded_prefix = distill_group_matchers(&prefix_record).expect("expand prefix");
        assert_eq!(
            expanded_prefix,
            vec![("alfaHD".to_string(), GroupMatchKindSpec::Prefix)]
        );
    }

    #[test]
    fn release_group_regex_expansion_handles_pahe_suffixes() {
        let record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "lq".to_string(),
            source_path: "docs/json/sonarr/cf/lq.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "LQ".to_string(),
            spec_name: "Pahe".to_string(),
            implementation: "ReleaseGroupSpecification".to_string(),
            value: r"Pahe(\.(ph|in))?\b".to_string(),
            ..Default::default()
        };
        let expanded = distill_group_matchers(&record).expect("expand pahe");
        assert_eq!(
            expanded,
            vec![
                ("Pahe".to_string(), GroupMatchKindSpec::Exact),
                ("Pahe.ph".to_string(), GroupMatchKindSpec::Exact),
                ("Pahe.in".to_string(), GroupMatchKindSpec::Exact),
            ]
        );
        let description_only = UpstreamRecord {
            spec_name: "Friendly Display Name".to_string(),
            value: r"^(?!.*\bEXCLUDED\b).*\bTOKEN\b".to_string(),
            source_path: "fixture.json".to_string(),
            ..Default::default()
        };
        assert!(distill_named_patterns(&description_only, ParserSignalKindSpec::Proper).is_err());
    }

    #[test]
    fn title_spec_group_distillation_activates_lossless_anime_groups() {
        let exact_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "anime-lq-groups".to_string(),
            source_path: "docs/json/sonarr/cf/anime-lq-groups.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "Anime LQ Groups".to_string(),
            spec_name: "AnimeRG".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: r"\b(AnimeRG)\b".to_string(),
            ..Default::default()
        };
        assert_eq!(
            distill_title_spec_group_matcher(&exact_record),
            Some(("AnimeRG".to_string(), GroupMatchKindSpec::Exact))
        );

        let bracket_record = UpstreamRecord {
            spec_name: "Cleo".to_string(),
            value: r"\[Cleo\]|-Cleo".to_string(),
            ..exact_record
        };
        assert_eq!(
            distill_title_spec_group_matcher(&bracket_record),
            Some(("Cleo".to_string(), GroupMatchKindSpec::Exact))
        );

        let complex_record = UpstreamRecord {
            spec_name: "Fish".to_string(),
            value: r"\b((Baked|Dead|Space)Fish)\b".to_string(),
            ..bracket_record
        };
        assert_eq!(distill_title_spec_group_matcher(&complex_record), None);
    }

    #[test]
    fn group_tier_conflicts_resolve_deterministically() {
        assert_eq!(
            select_group_tier([DistilledTier::Bronze, DistilledTier::Silver]),
            DistilledTier::Silver
        );
        assert_eq!(
            select_group_tier([DistilledTier::Silver, DistilledTier::Gold]),
            DistilledTier::Gold
        );
        assert_eq!(
            select_group_tier([DistilledTier::Gold, DistilledTier::Banned]),
            DistilledTier::Banned
        );
    }

    #[test]
    fn parser_rule_distillation_covers_service_and_upscaled_patterns() {
        let service_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "max".to_string(),
            source_path: "docs/json/sonarr/cf/max.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "MAX".to_string(),
            spec_name: "MAX Rename".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: r"\[(MAX)\b|\b(MAX)\]".to_string(),
            ..Default::default()
        };
        let aliases = distill_service_alias_tokens(&service_record).expect("service aliases");
        assert!(aliases.contains(&"MAX".to_string()));

        let upscaled_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "upscaled".to_string(),
            source_path: "docs/json/sonarr/cf/upscaled.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "Upscaled".to_string(),
            spec_name: "TheUpscaler".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: r"\b(The[ ._-]?Upscaler)\b".to_string(),
            ..Default::default()
        };
        let patterns = distill_named_patterns(&upscaled_record, ParserSignalKindSpec::AiEnhanced)
            .expect("upscaled patterns");
        assert!(
            patterns
                .iter()
                .any(|pattern| pattern.tokens.as_slice() == ["THEUPSCALER"])
        );
    }

    #[test]
    fn blocked_title_patterns_handle_sequence_and_required_tokens() {
        let anime_raw_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "anime-raws".to_string(),
            source_path: "docs/json/sonarr/cf/anime-raws.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "Anime Raws".to_string(),
            spec_name: "AsukaRaws".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: "Asuka[ ._-]?(Raws)".to_string(),
            ..Default::default()
        };
        let patterns = distill_blocked_title_patterns(&anime_raw_record, "trash_guides_anime_raws")
            .expect("anime raw patterns");
        assert!(patterns.iter().any(|pattern| {
            pattern.tokens.as_slice() == ["ASUKARAWS"]
                || pattern.tokens.as_slice() == ["ASUKA", "RAWS"]
        }));

        let lq_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "lq-release-title".to_string(),
            source_path: "docs/json/sonarr/cf/lq-release-title.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "LQ".to_string(),
            spec_name: "BiTOR (2160p)".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: "(?=.*?(\\b2160p\\b))(?=.*?(\\bBiTOR\\b))".to_string(),
            ..Default::default()
        };
        let lq_patterns =
            distill_blocked_title_patterns(&lq_record, "trash_guides_lq_release_title")
                .expect("lq title patterns");
        assert!(lq_patterns.iter().any(|pattern| {
            pattern.kind == TokenPatternKindSpec::RequiredTokens
                && pattern.tokens.as_slice() == ["2160P", "BITOR"]
        }));
    }

    #[test]
    fn top_level_specification_polarity_and_fields_json_are_preserved() {
        let upstream = serde_json::from_str::<UpstreamCf>(
            r#"{
                "name": "Scene",
                "trash_id": "scene-id",
                "trash_scores": {"default": -100},
                "specifications": [{
                    "name": "Not GERMAN",
                    "implementation": "ReleaseTitleSpecification",
                    "required": true,
                    "negate": true,
                    "fields": {"value": "\\bGERMAN\\b", "extra": ["kept"]}
                }]
            }"#,
        )
        .expect("fixture should deserialize");
        let spec = upstream
            .specifications
            .first()
            .expect("fixture specification");
        assert_eq!(json_string(Some(&spec.required)), "true");
        assert_eq!(json_string(Some(&spec.negate)), "true");
        let complete_fields = serde_json::from_str::<Value>(&json_string(Some(&spec.fields)))
            .expect("complete fields JSON");
        assert_eq!(complete_fields, spec.fields);

        let record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "scene".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: r"\bGERMAN\b".to_string(),
            negate_json: json_string(Some(&spec.negate)),
            ..Default::default()
        };
        let catalog = distill_records(&[record]).expect("distill negated scene");
        assert!(catalog.fact_rules.is_empty());
        assert!(
            catalog
                .inactive_records
                .iter()
                .any(|record| record.reason == "native_fact_requires_positive_title_spec")
        );
    }

    #[test]
    fn coverage_check_rejects_new_stems() {
        let records = vec![UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "new-upstream-stem".to_string(),
            ..Default::default()
        }];
        assert_eq!(
            unexpected_stems(&BTreeSet::new(), &records),
            BTreeSet::from([("sonarr".to_string(), "new-upstream-stem".to_string())])
        );
    }

    #[test]
    fn marker_patterns_use_regex_literals_not_display_names() {
        assert_eq!(
            pattern_from_spec_value(r"\b(VQ|VFQ)\b"),
            vec![
                TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec!["VFQ".to_string()],
                },
                TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec!["VQ".to_string()],
                },
            ]
        );
    }

    #[test]
    fn finite_group_expansion_keeps_exact_branches_and_skips_unbounded_ones() {
        assert_eq!(
            finite_group_literals(r"\b(ACOOL|DDLFRENCH(ORG)?|CZ\d+)\b").unwrap(),
            BTreeSet::from([
                "ACOOL".to_string(),
                "DDLFRENCH".to_string(),
                "DDLFRENCHORG".to_string(),
            ])
        );
    }

    #[test]
    fn scene_distillation_uses_release_group_suffixes() {
        let record = UpstreamRecord {
            value: r"^(?=.*(\b\d{3,4}p\b).*([_. ]WEB[_. ])(?!DL)\b)|\b(-CAKES|-GGEZ)".to_string(),
            ..Default::default()
        };
        assert_eq!(
            distill_scene_group_matchers(&record).unwrap(),
            vec!["CAKES".to_string(), "GGEZ".to_string()]
        );
    }

    fn alias_tokens(value: &str) -> Vec<String> {
        distill_service_alias_tokens(&UpstreamRecord {
            implementation: "ReleaseTitleSpecification".to_string(),
            value: value.to_string(),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn service_aliases_expand_optional_groups_losslessly() {
        // Naive splitting turned these into `FUNI` + `MATION` and `C`/`RUNCHY`/`OLL`.
        assert_eq!(alias_tokens(r"\b(FUNi(mation)?)\b"), ["FUNI", "FUNIMATION"]);
        assert_eq!(
            alias_tokens(r"\b(amzn|amazon(hd)?)\b"),
            ["AMAZON", "AMAZONHD", "AMZN"]
        );
    }

    #[test]
    fn service_aliases_survive_lookarounds_and_stay_single_tokens() {
        // Lookaround-bearing patterns must still yield HBO Max's real aliases.
        assert_eq!(
            alias_tokens(r"\b(hmax|hbom|hbo[ ._-]?max)\b(?=[ ._-]web[ ._-]?(dl|rip)\b)"),
            ["HBO", "HBOM", "HBOMAX", "HMAX"]
        );
        // A mandatory separator means the alias is only the first token: a
        // release is tagged by a `STAN` token, never one spelled `STANWEB`.
        assert_eq!(
            alias_tokens(r"\b(stan)\b[ ._-]web[ ._-]?(dl|rip)?\b"),
            ["STAN"]
        );
    }

    fn service_record(stem: &str, spec_name: &str, value: &str) -> UpstreamRecord {
        UpstreamRecord {
            app: "sonarr".to_string(),
            stem: stem.to_string(),
            source_path: format!("docs/json/sonarr/cf/{stem}.json"),
            trash_id: format!("{stem}-id"),
            cf_name: stem.to_uppercase(),
            spec_name: spec_name.to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: value.to_string(),
            required_json: "true".to_string(),
            ..Default::default()
        }
    }

    fn alias_row(catalog: &DistilledCatalog, token: &str) -> Option<(String, bool)> {
        catalog
            .service_alias_rules
            .iter()
            .find(|rule| rule.key.token == token)
            .map(|rule| (rule.key.service.clone(), rule.key.requires_web_adjacency))
    }

    #[test]
    fn negative_lookarounds_are_not_strippable_and_land_in_the_audit() {
        // Positive assertions only narrow context, so the tag text survives.
        assert_eq!(
            strip_lookarounds(r"\b(CNLP)\b(?=[ ._-]web[ ._-]?(dl|rip)\b)").as_deref(),
            Some(r"\b(CNLP)\b")
        );
        // A negative assertion is the pattern's safety mechanism: stripping it
        // would widen the match, so the whole specification is refused.
        assert_eq!(
            strip_lookarounds(r"(?<!dts[ .-]?hd[ .-]?)\b(ma|ykw)\b"),
            None
        );
        assert_eq!(strip_lookarounds(r"\b(hbo)(?![ ._-]max)\b"), None);

        let record = service_record("it", "iT Rename", r"\[(iT)(?![+])\b|\b(?<![+])(iT)\]");
        assert_eq!(
            distill_service_alias_tokens(&record),
            Err(AliasDistillationFailure::NegativeLookaround)
        );
        let catalog = distill_records(&[record]).expect("distill negative lookaround");
        assert!(catalog.service_alias_rules.is_empty());
        assert!(
            catalog
                .inactive_records
                .iter()
                .any(|record| record.reason == "service_alias_negative_lookaround")
        );
    }

    #[test]
    fn service_alias_policies_follow_the_reviewed_inclusion_table() {
        let catalog = distill_records(&[
            // Required title spec: upstream deems the bare token sufficient.
            service_record("abema", "ABEMA", r"\b(ABEMA[ ._-]?(TV)?)\b"),
            // No required spec: the token only counts beside a WEB marker.
            UpstreamRecord {
                required_json: "false".to_string(),
                ..service_record("now", "NOW", r"\b(now)\b[ ._-]web[ ._-]?(dl|rip)?\b")
            },
            // Per-token override: SHOWTIME is unambiguous, bare SHO is not.
            UpstreamRecord {
                required_json: "false".to_string(),
                ..service_record(
                    "sho",
                    "SHOWTIME",
                    r"\b(sho|showtime)\b[ ._-]web[ ._-]?(dl|rip)?\b",
                )
            },
            // Per-token exclusions: brand prefix and curated-supplement handoff.
            service_record("dcu", "DC Universe", r"\b(dcu|DC Universe)\b"),
            UpstreamRecord {
                required_json: "false".to_string(),
                ..service_record(
                    "red",
                    "YouTube Red",
                    r"\b(red|youtube red)\b[ ._-]web[ ._-]?(dl|rip)?\b",
                )
            },
        ])
        .expect("distill policy table");

        assert_eq!(
            alias_row(&catalog, "ABEMA"),
            Some(("ABEMA".to_string(), false))
        );
        assert_eq!(
            alias_row(&catalog, "ABEMATV"),
            Some(("ABEMA".to_string(), false))
        );
        assert_eq!(alias_row(&catalog, "NOW"), Some(("NOW".to_string(), true)));
        assert_eq!(
            alias_row(&catalog, "SHO"),
            Some(("Showtime".to_string(), true))
        );
        assert_eq!(
            alias_row(&catalog, "SHOWTIME"),
            Some(("Showtime".to_string(), false))
        );
        assert_eq!(
            alias_row(&catalog, "DCU"),
            Some(("DC Universe".to_string(), false))
        );
        assert_eq!(alias_row(&catalog, "DC"), None);
        assert_eq!(
            alias_row(&catalog, "RED"),
            Some(("YouTube Premium".to_string(), true))
        );
        assert_eq!(alias_row(&catalog, "YOUTUBE"), None);
        validate_service_alias_rules(&catalog.service_alias_rules)
            .expect("guards accept the table");
    }

    #[test]
    fn reviewed_service_exclusions_never_reach_the_alias_table() {
        for (stem, reason) in EXCLUDED_SERVICE_ALIAS_STEMS {
            let record = service_record(stem, "Service", r"\b(EXAMPLE)\b");
            let catalog = distill_records(&[record]).expect("distill excluded service stem");
            assert!(
                catalog.service_alias_rules.is_empty(),
                "{stem} produced alias rows"
            );
            assert!(
                catalog
                    .ignored_records
                    .iter()
                    .any(|record| record.reason == *reason),
                "{stem} lost its exclusion reason"
            );
            assert_eq!(
                classify_stem(stem).reason,
                "reviewed_service_alias_exclusion"
            );
        }
        // `bcore` is admitted, but only for its own token.
        let catalog = distill_records(&[service_record(
            "bcore",
            "Bravia Core",
            r"\b(BCORE)\b|\b(CORE)\b",
        )])
        .expect("distill bcore");
        assert_eq!(
            alias_row(&catalog, "BCORE"),
            Some(("BCORE".to_string(), false))
        );
        assert_eq!(alias_row(&catalog, "CORE"), None);
    }

    #[test]
    fn alias_guards_reject_ambiguous_tokens_and_split_policies() {
        let rule = |token: &str, service: &str, web: bool| ServiceAliasRecord {
            key: ServiceAliasKey {
                token: token.to_string(),
                service: service.to_string(),
                requires_web_adjacency: web,
            },
            provenance: Vec::new(),
        };

        validate_service_alias_rules(&[
            rule("SHO", "Showtime", true),
            rule("SHOWTIME", "Showtime", false),
        ])
        .expect("one token, one service, one policy");

        let ambiguous =
            validate_service_alias_rules(&[rule("PLAY", "PLAY", true), rule("PLAY", "Stan", true)])
                .expect_err("one token must not name two services");
        assert!(ambiguous.to_string().contains("more than one service"));

        let split =
            validate_service_alias_rules(&[rule("NOW", "NOW", true), rule("NOW", "NOW", false)])
                .expect_err("one token must not carry two policies");
        assert!(split.to_string().contains("standalone and WEB-adjacent"));
    }

    #[test]
    fn service_aliases_reject_source_enums_and_stray_fragments() {
        // Sonarr `SourceSpecification` values are numeric enums, never tokens.
        assert!(
            distill_service_alias_tokens(&UpstreamRecord {
                implementation: "SourceSpecification".to_string(),
                value: "3".to_string(),
                ..Default::default()
            })
            .is_err()
        );
        assert!(!alias_tokens(r"\b(C(runchy)?[ .-]?R(oll)?)\b").contains(&"C".to_string()));
    }

    #[test]
    fn composite_marker_component_is_not_treated_as_a_complete_fact() {
        let marker = UpstreamRecord {
            app: "sonarr".to_string(),
            trash_id: "composite".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            spec_name: "DL".to_string(),
            value: r"\bDL\b".to_string(),
            required_json: "true".to_string(),
            ..Default::default()
        };
        let required_language = UpstreamRecord {
            implementation: "LanguageSpecification".to_string(),
            spec_name: "German".to_string(),
            value: "german".to_string(),
            ..marker.clone()
        };
        assert!(!record_is_sufficient_fact(
            &[marker.clone(), required_language],
            &marker
        ));
        assert!(record_is_sufficient_fact(
            std::slice::from_ref(&marker),
            &marker
        ));
    }

    #[test]
    fn localized_lq_release_groups_emit_managed_facts() {
        let catalog = distill_records(&[UpstreamRecord {
            app: "radarr".to_string(),
            stem: "asian-lq".to_string(),
            source_path: "docs/json/radarr/cf/asian-lq.json".to_string(),
            trash_id: "asian-lq".to_string(),
            implementation: "ReleaseGroupSpecification".to_string(),
            spec_name: "groups".to_string(),
            value: "^(AppleTor|NEXT)$".to_string(),
            required_json: "false".to_string(),
            negate_json: "false".to_string(),
            ..Default::default()
        }])
        .unwrap();
        let matchers = catalog
            .locale_group_fact_rules
            .iter()
            .filter(|rule| rule.key.code == "trash.locale.asian.lq")
            .map(|rule| rule.key.matcher.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(matchers, BTreeSet::from(["AppleTor", "NEXT"]));
    }

    fn scored_record(app: &str, stem: &str, scores: &[(&str, i64)]) -> UpstreamRecord {
        UpstreamRecord {
            app: app.to_string(),
            stem: stem.to_string(),
            source_path: format!("docs/json/{app}/cf/{stem}.json"),
            trash_id: stem.to_string(),
            implementation: "ReleaseGroupSpecification".to_string(),
            spec_name: "groups".to_string(),
            value: "^(AppleTor|NEXT)$".to_string(),
            required_json: "false".to_string(),
            negate_json: "false".to_string(),
            scores: scores
                .iter()
                .map(|(set, score)| ((*set).to_string(), *score))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn trash_scores_must_be_integers() {
        let task = FetchTask {
            app: "radarr".to_string(),
            dir: "radarr/cf".to_string(),
            filename: "scene.json".to_string(),
        };
        let parsed = serde_json::from_str::<UpstreamCf>(
            r#"{"name": "Scene", "trash_scores": {"default": 1.5}, "specifications": []}"#,
        )
        .expect("fixture should deserialize");
        let error =
            records_from_custom_format(&task, parsed, "docs/json/radarr/cf/scene.json".to_string())
                .expect_err("fractional scores are not scores");
        assert!(error.to_string().contains("docs/json/radarr/cf/scene.json"));
    }

    #[test]
    fn veto_magnitude_is_the_smallest_veto_a_set_uses() {
        let records = vec![
            // `default` carries vetoes harder than its own cutoff, so the
            // largest would misread the set as a 35000 one.
            scored_record("radarr", "french-lq", &[("default", -10_000)]),
            scored_record("radarr", "german-lq", &[("default", -35_000)]),
            scored_record("radarr", "german-dl", &[("german", -35_000)]),
            scored_record("radarr", "asian-tier-01", &[("anime-radarr", 1650)]),
        ];
        assert_eq!(
            score_set_veto_magnitudes(&records),
            BTreeMap::from([
                ("anime-radarr".to_string(), DEFAULT_VETO_MAGNITUDE),
                ("default".to_string(), 10_000),
                ("german".to_string(), 35_000),
            ])
        );
    }

    #[test]
    fn score_envelope_records_the_observed_range_and_sentinels() {
        let records = vec![
            scored_record("radarr", "french-lq", &[("default", -10_000)]),
            scored_record("radarr", "german-lq", &[("default", -35_000)]),
            scored_record("radarr", "asian-tier-01", &[("default", 1650)]),
        ];
        assert_eq!(
            score_set_envelopes(&records),
            vec![ScoreSetEnvelope {
                score_set: "default".to_string(),
                min: -35_000,
                max: 1650,
                vetoes: vec![-35_000, -10_000],
            }]
        );
    }

    #[test]
    fn envelope_gate_reports_new_shrunk_and_missing_sets() {
        let known = vec![ScoreSetEnvelope {
            score_set: "default".to_string(),
            min: -35_000,
            max: 11_000,
            vetoes: vec![-35_000, -10_000],
        }];
        assert!(score_envelope_drift(&known, &known).is_empty());

        // A rescaled upstream keeps every value inside the old bounds while
        // silently retiring the veto sentinels, which is the case this exists
        // for.
        let rescaled = vec![ScoreSetEnvelope {
            score_set: "default".to_string(),
            min: 0,
            max: 100,
            vetoes: Vec::new(),
        }];
        assert_eq!(score_envelope_drift(&known, &rescaled).len(), 1);
        assert_eq!(
            score_envelope_drift(&known, &[]),
            vec!["default disappeared".to_string()]
        );
        assert_eq!(
            score_envelope_drift(&[], &known),
            vec!["default is new".to_string()]
        );
    }

    #[test]
    fn score_guard_is_scoped_per_app_and_skips_reviewed_native_facts() {
        let scored = distill_records(&[
            scored_record("radarr", "asian-lq", &[("default", -10_000)]),
            scored_record("sonarr", "asian-lq", &[]),
        ])
        .unwrap();
        assert_eq!(
            unscored_fact_apps(&scored),
            vec!["sonarr/trash.locale.asian.lq".to_string()]
        );

        // Radarr-only provenance never reports the app that publishes nothing.
        let radarr_only =
            distill_records(&[scored_record("radarr", "asian-lq", &[("default", -10_000)])])
                .unwrap();
        assert!(unscored_fact_apps(&radarr_only).is_empty());

        // The regional markers ship unscored upstream and are reviewed as such.
        let native = distill_records(&[UpstreamRecord {
            implementation: "ReleaseTitleSpecification".to_string(),
            spec_name: "VFF".to_string(),
            value: r"\b(VFF)\b".to_string(),
            ..scored_record("radarr", "french-vff", &[])
        }])
        .unwrap();
        assert!(
            native
                .fact_rules
                .iter()
                .any(|rule| rule.key.code == "trash.locale.french.marker.vff")
        );
        assert!(unscored_fact_apps(&native).is_empty());
    }

    #[test]
    fn fact_scores_join_every_score_set_the_producing_format_publishes() {
        let catalog = distill_records(&[scored_record(
            "radarr",
            "asian-lq",
            &[("default", -10_000), ("german", -35_000)],
        )])
        .unwrap();
        assert_eq!(
            collect_fact_scores(&catalog),
            vec![
                FactScoreRow {
                    code: "trash.locale.asian.lq".to_string(),
                    app: "radarr".to_string(),
                    score_set: "default".to_string(),
                    score: -10_000,
                },
                FactScoreRow {
                    code: "trash.locale.asian.lq".to_string(),
                    app: "radarr".to_string(),
                    score_set: "german".to_string(),
                    score: -35_000,
                },
            ]
        );
    }

    #[test]
    fn emission_digest_changes_when_matchers_change_at_constant_count() {
        let first = StemEmission {
            rule_count: 1,
            fact_codes: BTreeSet::from(["trash.test".to_string()]),
            rule_signatures: BTreeSet::from(["matcher:one".to_string()]),
        };
        let second = StemEmission {
            rule_count: 1,
            fact_codes: BTreeSet::from(["trash.test".to_string()]),
            rule_signatures: BTreeSet::from(["matcher:two".to_string()]),
        };

        assert_ne!(
            stable_emission_digest(&first),
            stable_emission_digest(&second)
        );
    }

    // -----------------------------------------------------------------------
    // Language rules
    // -----------------------------------------------------------------------

    fn language_record(
        app: &str,
        stem: &str,
        spec_name: &str,
        fields: &str,
        negate: bool,
        required: bool,
    ) -> UpstreamRecord {
        let dir = if app == GUIDE_ONLY_APP {
            GUIDE_ONLY_APP.to_string()
        } else {
            format!("{app}/cf")
        };
        UpstreamRecord {
            app: app.to_string(),
            stem: stem.to_string(),
            source_path: format!("docs/json/{dir}/{stem}.json"),
            trash_id: stem.to_string(),
            cf_name: stem.to_string(),
            spec_name: spec_name.to_string(),
            implementation: "LanguageSpecification".to_string(),
            value: fields.to_string(),
            required_json: required.to_string(),
            negate_json: negate.to_string(),
            scores: BTreeMap::from([("default".to_string(), -10_000)]),
        }
    }

    fn title_record(
        app: &str,
        stem: &str,
        spec_name: &str,
        pattern: &str,
        negate: bool,
        required: bool,
    ) -> UpstreamRecord {
        UpstreamRecord {
            implementation: "ReleaseTitleSpecification".to_string(),
            ..language_record(app, stem, spec_name, pattern, negate, required)
        }
    }

    fn distilled_conditions(records: &[UpstreamRecord]) -> Vec<LanguageConditionEntry> {
        match distill_language_rule(records).expect("fixture should distill") {
            LanguageRuleDistillation::Rule(conditions) => conditions,
            LanguageRuleDistillation::Inactive(reason) => {
                panic!("expected a distilled rule, got {reason}")
            }
        }
    }

    fn inactive_reason(records: &[UpstreamRecord]) -> &'static str {
        match distill_language_rule(records).expect("fixture should distill") {
            LanguageRuleDistillation::Rule(conditions) => {
                panic!("expected an inactive record, got {conditions:?}")
            }
            LanguageRuleDistillation::Inactive(reason) => reason,
        }
    }

    /// The ids are the contract with the rule input: every one has to name a
    /// code `normalize_detected_audio_language_code` can produce, or the
    /// condition would compare against a value `languages_audio` never carries.
    #[test]
    fn language_ids_map_to_the_codes_the_rule_input_publishes() {
        assert_eq!(language_id_code(1), Some("eng"));
        assert_eq!(language_id_code(2), Some("fra"));
        assert_eq!(language_id_code(4), Some("deu"));
        assert_eq!(language_id_code(8), Some("jpn"));
        assert_eq!(language_id_code(10), Some("zho"));
        assert_eq!(language_id_code(18), Some("por"));
        // Scryer's ISO table is `Dutch; Flemish` under one canonical code, so
        // Flemish and Dutch answer the same question.
        assert_eq!(language_id_code(7), language_id_code(19));
        assert_eq!(language_id_code(19), Some("nld"));
        assert_eq!(language_id_code(20), Some("ell"));
        assert_eq!(language_id_code(21), Some("kor"));
        assert_eq!(language_id_code(24), Some("lit"));

        // `Original` is relative, not a language, so it must not sit in the
        // id table at all.
        assert_eq!(language_id_code(ORIGINAL_LANGUAGE_ID), None);

        // Codes are lowercase ISO 639-2/T and names are lowercase English, both
        // matched exactly rather than case-folded at the call sites.
        for entry in LANGUAGE_IDS {
            assert_eq!(entry.code, entry.code.to_ascii_lowercase());
            assert_eq!(entry.name, entry.name.to_ascii_lowercase());
            assert_eq!(entry.code.len(), 3, "{}", entry.name);
            assert_eq!(language_name_code(entry.name), Some(entry.code));
        }
    }

    #[test]
    fn an_unmapped_language_id_fails_the_sync_naming_the_file() {
        let records = vec![language_record(
            "sonarr",
            "language-not-klingon",
            "Not Klingon",
            r#"{"value":99}"#,
            true,
            false,
        )];
        let error = distill_language_rule(&records).expect_err("unmapped ids are not distillable");
        let message = error.to_string();
        assert!(message.contains("99"), "{message}");
        assert!(
            message.contains("docs/json/sonarr/cf/language-not-klingon.json"),
            "{message}"
        );
    }

    #[test]
    fn the_relative_original_id_distills_to_the_original_condition() {
        let records = vec![language_record(
            "radarr",
            "language-not-original",
            "Not Original Language",
            r#"{"value":-2}"#,
            true,
            false,
        )];
        assert_eq!(
            distilled_conditions(&records),
            vec![LanguageConditionEntry {
                language: LanguageConditionSpec::Original,
                negate: true,
                required: false,
            }]
        );
    }

    /// `SpecificationMatchesGroup.DidMatch` is "no required specification may
    /// fail, and at least one must match". An optional non-language
    /// specification only ever answered the second clause, which a required
    /// condition already answers, so it drops out — this is `language-german-and-original`.
    #[test]
    fn optional_non_language_specs_drop_when_a_required_condition_gates_the_format() {
        let records = vec![
            language_record(
                GUIDE_ONLY_APP,
                "language-german-and-original",
                "German",
                r#"{"value":4}"#,
                false,
                true,
            ),
            title_record(
                GUIDE_ONLY_APP,
                "language-german-and-original",
                "DL",
                r"(?<!WEB[-_. ]?)\b(DL)\b",
                false,
                false,
            ),
            title_record(
                GUIDE_ONLY_APP,
                "language-german-and-original",
                "ML",
                r"\b(ML)\b",
                false,
                false,
            ),
        ];
        assert_eq!(
            distilled_conditions(&records),
            vec![LanguageConditionEntry {
                language: LanguageConditionSpec::Named("deu".to_string()),
                negate: false,
                required: true,
            }]
        );
    }

    /// With no required condition the format matches on "at least one
    /// specification matched", so an optional specification Scryer cannot
    /// express would narrow it — the whole format stays out.
    #[test]
    fn an_optional_non_language_spec_is_lossy_when_nothing_is_required() {
        let records = vec![
            language_record(
                GUIDE_ONLY_APP,
                "language-prefer-german",
                "German Language",
                r#"{"value":4}"#,
                false,
                false,
            ),
            title_record(
                GUIDE_ONLY_APP,
                "language-prefer-german",
                "DL",
                r"\b(DL)\b",
                false,
                false,
            ),
        ];
        assert_eq!(
            inactive_reason(&records),
            "language_rule_drops_a_matching_optional_spec"
        );
    }

    /// A required title specification is dropped only in the direction that is
    /// sound: a *negated* language check implies the negated title check,
    /// because Scryer reads `languages_audio` off the same title. This is the
    /// `not-…-or-english` family.
    #[test]
    fn a_required_title_spec_drops_only_when_a_language_condition_implies_it() {
        let subsumed = vec![
            language_record(
                "sonarr",
                "not-german-or-english",
                "Not English Language",
                r#"{"value":1}"#,
                true,
                true,
            ),
            language_record(
                "sonarr",
                "not-german-or-english",
                "Not German Language",
                r#"{"value":4}"#,
                true,
                true,
            ),
            title_record(
                "sonarr",
                "not-german-or-english",
                "Not German in Title",
                r"(?i)\bgerman\b",
                true,
                true,
            ),
        ];
        assert_eq!(
            distilled_conditions(&subsumed),
            vec![
                LanguageConditionEntry {
                    language: LanguageConditionSpec::Named("deu".to_string()),
                    negate: true,
                    required: true,
                },
                LanguageConditionEntry {
                    language: LanguageConditionSpec::Named("eng".to_string()),
                    negate: true,
                    required: true,
                },
            ]
        );

        // A required title marker that names no language gates the format on
        // something no condition can express, so it is recorded instead of
        // approximated. This is `language-original-plus-french`.
        let unsupported = vec![
            language_record(
                "sonarr",
                "language-original-plus-french",
                "Original Language",
                r#"{"value":-2}"#,
                false,
                true,
            ),
            language_record(
                "sonarr",
                "language-original-plus-french",
                "French Language",
                r#"{"value":2}"#,
                false,
                true,
            ),
            title_record(
                "sonarr",
                "language-original-plus-french",
                "MULTi",
                r"\b(MULTi)(\b|\d)",
                false,
                true,
            ),
        ];
        assert_eq!(
            inactive_reason(&unsupported),
            "language_rule_requires_unsupported_release_title_spec"
        );
        // The reviewed classification and the distiller must agree about why.
        assert_eq!(
            undistillable_language_stem_reason("language-original-plus-french"),
            Some("language_rule_requires_unsupported_release_title_spec")
        );

        // The positive direction never holds: an audio language can arrive from
        // the indexer rather than the title, so the title check still narrows.
        let positive = vec![
            language_record(
                "sonarr",
                "language-german-only",
                "German Language",
                r#"{"value":4}"#,
                false,
                true,
            ),
            title_record(
                "sonarr",
                "language-german-only",
                "German in Title",
                r"(?i)\bgerman\b",
                false,
                true,
            ),
        ];
        assert_eq!(
            inactive_reason(&positive),
            "language_rule_requires_unsupported_release_title_spec"
        );
    }

    #[test]
    fn except_language_is_a_different_question_and_stays_out() {
        let records = vec![language_record(
            "radarr",
            "language-not-japanese",
            "Not Japanese Language",
            r#"{"exceptLanguage":true,"value":8}"#,
            true,
            true,
        )];
        assert_eq!(
            inactive_reason(&records),
            "language_rule_uses_except_language"
        );
    }

    #[test]
    fn language_rule_codes_are_stable_and_drop_the_redundant_prefix() {
        assert_eq!(
            language_rule_code("language-not-english"),
            "trash.lang.not_english"
        );
        assert_eq!(
            language_rule_code("language-not-original-or-german"),
            "trash.lang.not_original_or_german"
        );
        assert_eq!(
            language_rule_code("not-german-or-english"),
            "trash.lang.not_german_or_english"
        );
    }

    #[test]
    fn language_stems_are_recognised_by_name_and_by_the_guide_only_tree() {
        assert!(is_language_rule_stem("sonarr", "language-not-english"));
        assert!(is_language_rule_stem("radarr", "not-german-or-english"));
        assert!(is_language_rule_stem(
            "sonarr",
            "not-german-japanese-korean-chinese-or-english"
        ));
        // Everything under guide-only is a language policy, named or not.
        assert!(is_language_rule_stem(
            GUIDE_ONLY_APP,
            "language-prefer-dutch"
        ));
        // Locale stems keep their own branch.
        assert!(!is_language_rule_stem("sonarr", "french-vostfr"));
        assert!(!is_language_rule_stem("radarr", "german-lq"));
        assert!(!is_language_rule_stem("sonarr", "not-german-or-french"));
    }

    /// Apps that publish the same code have to agree about what it means,
    /// because the managed packs resolve a code through whichever app answers
    /// first.
    #[test]
    fn language_rules_that_disagree_across_apps_fail_the_sync() {
        let rule = |app: &str, code: &str| LanguageRuleRecord {
            key: LanguageRuleKey {
                code: code.to_string(),
                app: app.to_string(),
                stem: "language-not-english".to_string(),
                conditions: vec![LanguageConditionEntry {
                    language: LanguageConditionSpec::Named(
                        if app == "sonarr" { "eng" } else { "deu" }.to_string(),
                    ),
                    negate: true,
                    required: false,
                }],
            },
            provenance: Vec::new(),
        };

        let error = validate_language_rules(&[
            rule("sonarr", "trash.lang.not_english"),
            rule("radarr", "trash.lang.not_english"),
        ])
        .expect_err("apps must agree");
        assert!(
            error.to_string().contains("trash.lang.not_english"),
            "{error}"
        );
        assert!(validate_language_rules(&[rule("sonarr", "trash.lang.not_english")]).is_ok());
    }

    /// The local-source path has to walk `docs/json/guide-only/` alongside the
    /// two app trees, or the offline sync would silently produce a different
    /// catalog than the network one.
    #[test]
    fn the_local_source_tree_carries_guide_only_records() {
        let root = std::env::temp_dir().join(format!(
            "scryer-trash-guide-only-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        for source in GUIDE_SOURCES {
            fs::create_dir_all(root.join("docs/json").join(source.dir)).unwrap();
        }
        fs::write(
            root.join("docs/json/sonarr/cf/language-not-english.json"),
            r#"{"trash_id":"a","trash_scores":{"default":-10000},"name":"Language: Not English","specifications":[{"name":"Not English Language","implementation":"LanguageSpecification","negate":true,"required":false,"fields":{"value":1}}]}"#,
        )
        .unwrap();
        fs::write(
            root.join("docs/json/radarr/cf/language-not-english.json"),
            r#"{"trash_id":"b","trash_scores":{"default":-10000},"name":"Language: Not English","specifications":[{"name":"Not English Language","implementation":"LanguageSpecification","negate":true,"required":false,"fields":{"value":1}}]}"#,
        )
        .unwrap();
        fs::write(
            root.join("docs/json/guide-only/language-prefer-dutch.json"),
            r#"{"trash_id":"guide-only","trash_scores":{"default":10},"name":"Language: Prefer Dutch","specifications":[{"name":"Dutch Language","implementation":"LanguageSpecification","negate":false,"required":false,"fields":{"value":7}},{"name":"Flemish Language","implementation":"LanguageSpecification","negate":false,"required":false,"fields":{"value":19}}]}"#,
        )
        .unwrap();

        // SAFETY: single-threaded within this test, and the value is restored
        // before returning so no other test observes it.
        let previous = std::env::var(SOURCE_REVISION_ENV).ok();
        unsafe { std::env::set_var(SOURCE_REVISION_ENV, "test-revision") };
        let fetched = read_local_records(&root);
        match previous {
            Some(value) => unsafe { std::env::set_var(SOURCE_REVISION_ENV, value) },
            None => unsafe { std::env::remove_var(SOURCE_REVISION_ENV) },
        }
        let fetched = fetched.expect("local source should read");
        let _ = fs::remove_dir_all(&root);

        assert_eq!(fetched.source_revision, "test-revision");
        let guide_only = fetched
            .records
            .iter()
            .filter(|record| record.app == GUIDE_ONLY_APP)
            .collect::<Vec<_>>();
        assert_eq!(guide_only.len(), 2);
        assert!(
            guide_only
                .iter()
                .all(|record| record.source_path
                    == "docs/json/guide-only/language-prefer-dutch.json"),
            "{guide_only:?}"
        );

        let catalog = distill_records(&fetched.records).expect("fixture should distill");
        let codes = catalog
            .language_rules
            .iter()
            .map(|rule| (rule.key.code.as_str(), rule.key.app.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            codes,
            vec![
                ("trash.lang.not_english", "radarr"),
                ("trash.lang.not_english", "sonarr"),
                ("trash.lang.prefer_dutch", GUIDE_ONLY_APP),
            ]
        );
        // Dutch and Flemish collapse onto one condition, so the format asks a
        // single question.
        let prefer_dutch = catalog
            .language_rules
            .iter()
            .find(|rule| rule.key.code == "trash.lang.prefer_dutch")
            .unwrap();
        assert_eq!(
            prefer_dutch.key.conditions,
            vec![LanguageConditionEntry {
                language: LanguageConditionSpec::Named("nld".to_string()),
                negate: false,
                required: false,
            }]
        );

        // guide-only is simply another app to the score join, so its scores
        // land in the table under their own app.
        assert!(collect_fact_scores(&catalog).contains(&FactScoreRow {
            code: "trash.lang.prefer_dutch".to_string(),
            app: GUIDE_ONLY_APP.to_string(),
            score_set: "default".to_string(),
            score: 10,
        }));
        assert!(unscored_fact_apps(&catalog).is_empty());
    }

    /// Distillation must not depend on the order records arrive in, because the
    /// network path fetches concurrently.
    #[test]
    fn language_rule_distillation_is_order_independent() {
        let mut records = vec![
            language_record(
                "sonarr",
                "not-german-or-english",
                "Not English Language",
                r#"{"value":1}"#,
                true,
                true,
            ),
            language_record(
                "sonarr",
                "not-german-or-english",
                "Not German Language",
                r#"{"value":4}"#,
                true,
                true,
            ),
            title_record(
                "sonarr",
                "not-german-or-english",
                "Not German in Title",
                r"(?i)\bgerman\b",
                true,
                true,
            ),
            language_record(
                GUIDE_ONLY_APP,
                "language-not-dutch",
                "Not Dutch Language",
                r#"{"value":7}"#,
                true,
                true,
            ),
            language_record(
                GUIDE_ONLY_APP,
                "language-not-dutch",
                "Not Flemish Language",
                r#"{"value":19}"#,
                true,
                true,
            ),
        ];

        let forward = distill_records(&records).unwrap();
        records.reverse();
        let reversed = distill_records(&records).unwrap();

        let keys = |catalog: &DistilledCatalog| {
            catalog
                .language_rules
                .iter()
                .map(|rule| rule.key.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&forward), keys(&reversed));
        assert_eq!(
            render_quality_output(&forward, &records, "rev").unwrap(),
            render_quality_output(&reversed, &records, "rev").unwrap()
        );
    }
}
