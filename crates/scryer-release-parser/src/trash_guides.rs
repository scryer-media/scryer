use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[cfg(test)]
use crate::model::{GuideFact, ParsedReleaseMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenPatternKind {
    Sequence,
    RequiredTokens,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenPattern {
    pub kind: TokenPatternKind,
    pub tokens: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ParserSignalKind {
    AiEnhanced,
    Proper,
    Repack,
    DubsOnly,
    HardcodedSubs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleFacet {
    Movie,
    Series,
    Anime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ServiceAliasRule {
    pub token: &'static str,
    pub service: &'static str,
    /// The token only names a service when a WEB marker follows it.
    ///
    /// Upstream applies these formats to every WEB release, because none of
    /// their specifications is required. Their tokens are
    /// common words — `NOW`, `RED`, `PLAY`, `FRIDAY`, `IT` — so without the
    /// adjacency they would name a service in ordinary titles.
    pub requires_web_adjacency: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenSignalRule {
    pub kind: ParserSignalKind,
    pub pattern: TokenPattern,
    pub facet: RuleFacet,
    pub app: &'static str,
    pub stem: &'static str,
    pub trash_id: &'static str,
    pub cf_name: &'static str,
    pub spec_name: &'static str,
    pub source_path: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub enum TitleCategoryScope {
    Any,
    Anime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub struct BlockedTitleRule {
    pub code: &'static str,
    pub facet: RuleFacet,
    pub category: TitleCategoryScope,
    pub pattern: TokenPattern,
    pub app: &'static str,
    pub stem: &'static str,
    pub trash_id: &'static str,
    pub cf_name: &'static str,
    pub spec_name: &'static str,
    pub source_path: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct FactRule {
    pub code: &'static str,
    pub facet: RuleFacet,
    pub category: TitleCategoryScope,
    pub pattern: TokenPattern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub(crate) enum LocaleGroupMatchKind {
    Exact,
    #[allow(dead_code)]
    Prefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub(crate) enum LocaleSourceContext {
    Web,
    BluRay,
    UhdBluRay,
    Remux,
    Anime,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct LocaleGroupFactRule {
    pub code: &'static str,
    pub matcher: &'static str,
    pub match_kind: LocaleGroupMatchKind,
    pub facet: RuleFacet,
    pub source_context: LocaleSourceContext,
}

include!("trash_guides_parser_knowledge.generated.rs");

#[derive(Debug, Default)]
struct TokenAnchorIndex {
    rules_by_anchor: BTreeMap<&'static str, Vec<usize>>,
}

impl TokenAnchorIndex {
    fn from_patterns(patterns: impl Iterator<Item = (usize, &'static TokenPattern)>) -> Self {
        let mut rules_by_anchor = BTreeMap::<&'static str, Vec<usize>>::new();
        for (index, pattern) in patterns {
            let Some(anchor) = pattern_anchor(pattern) else {
                continue;
            };
            rules_by_anchor.entry(anchor).or_default().push(index);
        }
        Self { rules_by_anchor }
    }

    fn candidate_indices(&self, normalized_tokens: &[String]) -> Vec<usize> {
        let mut indices = BTreeSet::new();
        for token in normalized_tokens {
            if let Some(matches) = self.rules_by_anchor.get(token.as_str()) {
                indices.extend(matches.iter().copied());
            }
        }
        indices.into_iter().collect()
    }
}

fn pattern_anchor(pattern: &TokenPattern) -> Option<&'static str> {
    match pattern.kind {
        TokenPatternKind::Sequence => pattern.tokens.first().copied(),
        TokenPatternKind::RequiredTokens => pattern.tokens.iter().copied().min(),
    }
}

fn token_signal_index() -> &'static TokenAnchorIndex {
    static INDEX: OnceLock<TokenAnchorIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        TokenAnchorIndex::from_patterns(
            TOKEN_SIGNAL_RULES
                .iter()
                .enumerate()
                .map(|(index, rule)| (index, &rule.pattern)),
        )
    })
}

#[cfg(test)]
fn blocked_title_index() -> &'static TokenAnchorIndex {
    static INDEX: OnceLock<TokenAnchorIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        TokenAnchorIndex::from_patterns(
            BLOCKED_TITLE_RULES
                .iter()
                .enumerate()
                .map(|(index, rule)| (index, &rule.pattern)),
        )
    })
}

#[cfg(test)]
fn fact_index() -> &'static TokenAnchorIndex {
    static INDEX: OnceLock<TokenAnchorIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        TokenAnchorIndex::from_patterns(
            FACT_RULES
                .iter()
                .enumerate()
                .map(|(index, rule)| (index, &rule.pattern)),
        )
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenSignalMatch {
    pub ai_enhanced: bool,
    pub proper: bool,
    pub repack: bool,
    pub dubs_only: bool,
    pub hardcoded_subs: bool,
}

/// Services TRaSH does not publish, or spellings its patterns do not carry.
///
/// The generated table is the source of truth for service detection; this is the
/// remainder the parser detected before unification and still must. A generated
/// entry supersedes a curated one for the same token, and
/// `curated_supplement_adds_no_generated_token` asserts that no such collision
/// exists today.
pub(crate) static CURATED_SERVICE_ALIASES: &[(&str, &str)] = &[
    ("BBC", "BBC iPlayer"),
    ("BBCI", "BBC iPlayer"),
    ("DNSP", "Disney+"),
    ("HOTSTAR", "Hotstar"),
    ("ITUNES", "iTunes"),
    ("YOUTUBE", "YouTube"),
];

fn generated_alias(token: &str) -> Option<&'static ServiceAliasRule> {
    SERVICE_ALIAS_RULES
        .iter()
        .find(|rule| rule.token.eq_ignore_ascii_case(token))
}

fn curated_alias(token: &str) -> Option<&'static str> {
    CURATED_SERVICE_ALIASES
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(token))
        .map(|(_, service)| *service)
}

/// The display name for a token already established to name a service.
///
/// Policy-agnostic on purpose: adjacency decides whether the token gets the
/// streaming-service role at all, and by the time a role-assigned token needs a
/// name that question is settled.
pub(crate) fn normalize_streaming_service_alias(token: &str) -> Option<&'static str> {
    generated_alias(token)
        .map(|rule| rule.service)
        .or_else(|| curated_alias(token))
}

/// Aliases that name a service on their own, with no neighboring WEB marker.
///
/// This is the lookup for context-free callers, which see one token and no
/// index. A WEB-adjacent alias must never answer here: `NOW`, `RED`, and `IT`
/// are ordinary title words until upstream's adjacency holds.
pub(crate) fn normalize_streaming_service_alias_standalone(token: &str) -> Option<&'static str> {
    match generated_alias(token) {
        Some(rule) if rule.requires_web_adjacency => None,
        Some(rule) => Some(rule.service),
        None => curated_alias(token),
    }
}

/// Aliases that name a service given what follows them.
///
/// `web_adjacent` is true when the next normalized token is a WEB marker, which
/// is the adjacency upstream's own `token[ ._-]web[ ._-]?(dl|rip)?` patterns
/// encode. Bare `web` satisfies them, so the `DL` is not required.
pub(crate) fn normalize_streaming_service_alias_in_context(
    token: &str,
    web_adjacent: bool,
) -> Option<&'static str> {
    match generated_alias(token) {
        Some(rule) if rule.requires_web_adjacency && !web_adjacent => None,
        Some(rule) => Some(rule.service),
        None => curated_alias(token),
    }
}

pub(crate) fn detect_token_signals(normalized_tokens: &[String]) -> TokenSignalMatch {
    let mut matched = TokenSignalMatch::default();
    for index in token_signal_index().candidate_indices(normalized_tokens) {
        let rule = &TOKEN_SIGNAL_RULES[index];
        if !pattern_matches(&rule.pattern, normalized_tokens) {
            continue;
        }
        match rule.kind {
            ParserSignalKind::AiEnhanced => matched.ai_enhanced = true,
            ParserSignalKind::Proper => matched.proper = true,
            ParserSignalKind::Repack => {
                matched.proper = true;
                matched.repack = true;
            }
            ParserSignalKind::DubsOnly => matched.dubs_only = true,
            ParserSignalKind::HardcodedSubs => matched.hardcoded_subs = true,
        }
    }
    matched
}

#[cfg(test)]
pub(crate) fn derive_facts(raw_title: &str, category_hint: Option<&str>) -> Vec<GuideFact> {
    let tokens = normalize_raw_title_tokens(raw_title);
    derive_facts_from_tokens(raw_title, &tokens, category_hint)
}

#[cfg(test)]
pub(crate) fn derive_locale_group_facts(
    projected: &ParsedReleaseMetadata,
    category_hint: Option<&str>,
) -> Vec<GuideFact> {
    let Some(release_group) = projected.release_group.as_deref() else {
        return Vec::new();
    };
    let facet = normalize_facet(category_hint);
    let mut codes = BTreeSet::new();
    for rule in LOCALE_GROUP_FACT_RULES {
        if rule.facet == facet
            && locale_group_matches(rule, release_group)
            && locale_source_context_matches(rule.source_context, projected)
        {
            codes.insert(rule.code);
        }
    }
    codes
        .into_iter()
        .map(|code| GuideFact {
            code: code.to_string(),
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn derive_structural_facts(
    projected: &ParsedReleaseMetadata,
    category_hint: Option<&str>,
) -> Vec<GuideFact> {
    if projected.release_group.is_some() {
        return Vec::new();
    }
    let facet = normalize_facet(category_hint);
    NO_RELEASE_GROUP_FACT_FACETS
        .contains(&facet)
        .then(|| GuideFact {
            code: "trash.no_release_group".to_string(),
        })
        .into_iter()
        .collect()
}

#[cfg(test)]
fn locale_group_matches(rule: &LocaleGroupFactRule, release_group: &str) -> bool {
    match rule.match_kind {
        LocaleGroupMatchKind::Exact => rule.matcher.eq_ignore_ascii_case(release_group),
        LocaleGroupMatchKind::Prefix => release_group
            .get(..rule.matcher.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(rule.matcher)),
    }
}

#[cfg(test)]
fn locale_source_context_matches(
    source_context: LocaleSourceContext,
    projected: &ParsedReleaseMetadata,
) -> bool {
    match source_context {
        LocaleSourceContext::Any | LocaleSourceContext::Anime => true,
        LocaleSourceContext::Web => matches!(
            projected.source,
            Some(crate::model::ReleaseSource::WebDl | crate::model::ReleaseSource::WebRip)
        ),
        LocaleSourceContext::Remux => projected.is_remux,
        LocaleSourceContext::BluRay => {
            matches!(projected.source, Some(crate::model::ReleaseSource::BluRay))
                && !projected.is_remux
                && !is_uhd_quality(projected.quality.as_deref())
        }
        LocaleSourceContext::UhdBluRay => {
            matches!(projected.source, Some(crate::model::ReleaseSource::BluRay))
                && is_uhd_quality(projected.quality.as_deref())
        }
    }
}

#[cfg(test)]
fn is_uhd_quality(quality: Option<&str>) -> bool {
    quality.is_some_and(|quality| quality.contains("2160"))
}

#[cfg(test)]
fn derive_facts_from_tokens(
    raw_title: &str,
    normalized_tokens: &[String],
    category_hint: Option<&str>,
) -> Vec<GuideFact> {
    let scope = normalize_scope(category_hint);
    let facet = normalize_facet(category_hint);
    let mut codes = BTreeSet::new();

    for index in token_signal_index().candidate_indices(normalized_tokens) {
        let rule = &TOKEN_SIGNAL_RULES[index];
        if !pattern_matches(&rule.pattern, normalized_tokens) {
            continue;
        }
        match rule.kind {
            ParserSignalKind::AiEnhanced => {
                codes.insert("trash.ai_enhanced");
            }
            ParserSignalKind::Proper => {
                codes.insert("trash.proper");
            }
            ParserSignalKind::Repack => {
                codes.insert("trash.proper");
                codes.insert("trash.repack");
            }
            ParserSignalKind::DubsOnly => {
                codes.insert("trash.dubs_only");
            }
            ParserSignalKind::HardcodedSubs => {
                codes.insert("trash.hardcoded_subs");
            }
        }
    }

    for index in blocked_title_index().candidate_indices(normalized_tokens) {
        let rule = &BLOCKED_TITLE_RULES[index];
        if rule_applies(rule.facet, rule.category, facet, scope)
            && pattern_matches(&rule.pattern, normalized_tokens)
        {
            codes.insert(blocked_fact_code(rule.code));
        }
    }

    for index in fact_index().candidate_indices(normalized_tokens) {
        let rule = &FACT_RULES[index];
        if rule_applies(rule.facet, rule.category, facet, scope)
            && pattern_matches(&rule.pattern, normalized_tokens)
        {
            codes.insert(rule.code);
        }
    }

    if has_french_vf2_exclusion(raw_title) {
        codes.remove("trash.locale.french.marker.vff");
        codes.remove("trash.locale.french.marker.vfq");
    }
    if codes.contains("trash.locale.german.marker.subbed")
        && !matches_german_subbed(normalized_tokens)
    {
        codes.remove("trash.locale.german.marker.subbed");
    }

    codes
        .into_iter()
        .map(|code| GuideFact {
            code: code.to_string(),
        })
        .collect()
}

#[cfg(test)]
fn has_french_vf2_exclusion(raw_title: &str) -> bool {
    let upper = raw_title.to_ascii_uppercase();
    [
        "VF2", "VFF.VFF", "VFF.VFQ", "VFQ.VFF", "VFQ.VFQ", "VFF VFF", "VFF VFQ", "VFQ VFF",
        "VFQ VFQ",
    ]
    .iter()
    .any(|pattern| contains_ascii_bounded(&upper, pattern))
}

#[cfg(test)]
fn matches_german_subbed(normalized_tokens: &[String]) -> bool {
    for (language_index, language) in normalized_tokens.iter().enumerate() {
        if !matches!(language.as_str(), "GER" | "GERMAN") {
            continue;
        }
        for (subtitle_offset, subtitle) in
            normalized_tokens[language_index + 1..].iter().enumerate()
        {
            if !matches!(subtitle.as_str(), "OMU" | "SUB" | "SUBBED" | "SUBS") {
                continue;
            }
            let subtitle_index = language_index + 1 + subtitle_offset;
            let gap = &normalized_tokens[language_index + 1..subtitle_index];
            if gap.iter().all(|token| {
                token.chars().all(|ch| ch.is_ascii_alphabetic())
                    && !token.contains("DUB")
                    && !matches!(token.as_str(), "DL" | "ML")
            }) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
fn contains_ascii_bounded(value: &str, pattern: &str) -> bool {
    value.match_indices(pattern).any(|(start, _)| {
        let end = start + pattern.len();
        let before_is_word = start > 0 && value.as_bytes()[start - 1].is_ascii_alphanumeric();
        let after_is_word = end < value.len() && value.as_bytes()[end].is_ascii_alphanumeric();
        !before_is_word && !after_is_word
    })
}

#[cfg(test)]
pub(crate) fn project_safe_facts(projected: &mut ParsedReleaseMetadata) {
    for fact in &projected.guide_facts {
        match fact.code.as_str() {
            "trash.ai_enhanced" => projected.is_ai_enhanced = true,
            "trash.proper" => projected.is_proper_upload = true,
            "trash.repack" => {
                projected.is_proper_upload = true;
                projected.is_repack = true;
            }
            "trash.hardcoded_subs" => projected.is_hardcoded_subs = true,
            _ => {}
        }
    }
}

#[cfg(test)]
pub fn detect_blocked_title(raw_title: &str, category_hint: Option<&str>) -> Option<&'static str> {
    let tokens = normalize_raw_title_tokens(raw_title);
    detect_blocked_title_tokens(&tokens, category_hint)
}

#[cfg(test)]
pub(crate) fn detect_blocked_title_tokens(
    normalized_tokens: &[String],
    category_hint: Option<&str>,
) -> Option<&'static str> {
    let scope = normalize_scope(category_hint);
    let facet = normalize_facet(category_hint);
    blocked_title_index()
        .candidate_indices(normalized_tokens)
        .into_iter()
        .map(|index| &BLOCKED_TITLE_RULES[index])
        .find(|rule| {
            rule_applies(rule.facet, rule.category, facet, scope)
                && pattern_matches(&rule.pattern, normalized_tokens)
        })
        .map(|rule| rule.code)
}

#[cfg(test)]
fn rule_applies(
    rule_facet: RuleFacet,
    rule_category: TitleCategoryScope,
    facet: RuleFacet,
    scope: TitleCategoryScope,
) -> bool {
    rule_facet == facet
        && (matches!(rule_category, TitleCategoryScope::Any)
            || matches!(scope, TitleCategoryScope::Anime)
                && matches!(rule_category, TitleCategoryScope::Anime))
}

#[cfg(test)]
fn blocked_fact_code(code: &str) -> &'static str {
    match code {
        "trash_guides_anime_raws" => "trash.blocked.anime_raws",
        "trash_guides_lq_release_title" => "trash.blocked.lq_release_title",
        "trash_guides_fansub" => "trash.blocked.fansub",
        "trash_guides_fastsub" => "trash.blocked.fastsub",
        _ => "trash.blocked.legacy",
    }
}

#[cfg(test)]
fn normalize_facet(category_hint: Option<&str>) -> RuleFacet {
    match category_hint
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("anime") => RuleFacet::Anime,
        Some("series") => RuleFacet::Series,
        _ => RuleFacet::Movie,
    }
}

#[cfg(test)]
fn normalize_scope(category_hint: Option<&str>) -> TitleCategoryScope {
    match category_hint
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("anime") => TitleCategoryScope::Anime,
        _ => TitleCategoryScope::Any,
    }
}

#[cfg(test)]
fn normalize_raw_title_tokens(raw_title: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in raw_title.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_uppercase());
        } else if !current.is_empty() {
            tokens.push(current.clone());
            current.clear();
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn pattern_matches(pattern: &TokenPattern, normalized_tokens: &[String]) -> bool {
    match pattern.kind {
        TokenPatternKind::Sequence => {
            normalized_tokens
                .windows(pattern.tokens.len())
                .any(|window| {
                    window
                        .iter()
                        .map(String::as_str)
                        .eq(pattern.tokens.iter().copied())
                })
        }
        TokenPatternKind::RequiredTokens => pattern
            .tokens
            .iter()
            .all(|token| normalized_tokens.iter().any(|candidate| candidate == token)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_supplement_adds_no_generated_token() {
        // Generated entries supersede curated ones, so a curated token the
        // generated table already carries would be silently dead. Keeping the
        // sets disjoint is what makes the supplement readable as "the rest".
        let overlapping = CURATED_SERVICE_ALIASES
            .iter()
            .filter(|(token, _)| generated_alias(token).is_some())
            .map(|(token, _)| *token)
            .collect::<Vec<_>>();
        assert_eq!(overlapping, Vec::<&str>::new());
    }

    #[test]
    fn unification_keeps_every_service_the_hardcoded_list_detected() {
        // The list `classify_token` carried before the generated table became
        // authoritative. Each of these was context-free, so each
        // must still resolve standalone.
        const LEGACY_TOKENS: &[&str] = &[
            "NF",
            "NETFLIX",
            "AMZN",
            "AMAZON",
            "CR",
            "CRUNCHYROLL",
            "HULU",
            "DSNP",
            "DNSP",
            "MAX",
            "HMAX",
            "HBO",
            "ATVP",
            "APTV",
            "PMTP",
            "PARAMOUNT",
            "PCOK",
            "PEACOCK",
            "FUNI",
            "FUNIMATION",
            "HIDIVE",
            "STAN",
            "ITUNES",
            "BILI",
            "HOTSTAR",
            "BBC",
            "BBCI",
            "IPLAYER",
            "YOUTUBE",
        ];
        let lost = LEGACY_TOKENS
            .iter()
            .copied()
            .filter(|token| normalize_streaming_service_alias_standalone(token).is_none())
            .collect::<Vec<_>>();
        assert_eq!(lost, Vec::<&str>::new());
    }

    #[test]
    fn every_alias_service_projects_to_a_streaming_service() {
        // Detection is worthless if projection cannot name what it found: the
        // `StreamingService` enum has to keep up with the distilled table.
        let unprojectable = SERVICE_ALIAS_RULES
            .iter()
            .map(|rule| rule.service)
            .chain(CURATED_SERVICE_ALIASES.iter().map(|(_, service)| *service))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|service| {
                crate::model::StreamingService::parse(service)
                    .is_none_or(|parsed| parsed.as_str() != *service)
            })
            .collect::<Vec<_>>();
        assert_eq!(unprojectable, Vec::<&str>::new());
    }

    #[test]
    fn standalone_lookup_refuses_web_adjacent_aliases() {
        // Context-free callers must not resolve a token whose policy needs a
        // neighbor they cannot see.
        assert_eq!(normalize_streaming_service_alias("NOW"), Some("NOW"));
        assert_eq!(normalize_streaming_service_alias_standalone("NOW"), None);
        assert_eq!(
            normalize_streaming_service_alias_in_context("NOW", false),
            None
        );
        assert_eq!(
            normalize_streaming_service_alias_in_context("NOW", true),
            Some("NOW")
        );

        // Standalone aliases answer regardless of what follows them.
        assert_eq!(
            normalize_streaming_service_alias_standalone("AMZN"),
            Some("Amazon")
        );
        assert_eq!(
            normalize_streaming_service_alias_in_context("AMZN", false),
            Some("Amazon")
        );

        // The curated supplement is standalone in both lookups.
        assert_eq!(
            normalize_streaming_service_alias_standalone("BBCI"),
            Some("BBC iPlayer")
        );
        assert_eq!(
            normalize_streaming_service_alias_in_context("hotstar", false),
            Some("Hotstar")
        );
    }

    #[test]
    fn detects_generated_streaming_alias_and_token_signals() {
        assert_eq!(normalize_streaming_service_alias("MAX"), Some("HBO Max"));

        let tokens = vec![
            "THE".to_string(),
            "UPSCALER".to_string(),
            "REPACK".to_string(),
        ];
        let matched = detect_token_signals(&tokens);
        assert!(matched.ai_enhanced);
        assert!(matched.proper);
        assert!(matched.repack);
    }

    #[test]
    #[allow(deprecated)]
    fn parser_keeps_ordinary_signals_and_streaming_service_without_guide_facts() {
        let context = crate::ReleaseParseContext {
            facet_hint: crate::ContextFacetHint::Movie,
            title: crate::ContextTitle {
                name: "Movie Title".to_string(),
            },
            aliases: vec![],
            known_years: vec![2024],
            imdb_ids: vec![],
            episodes: vec![],
        };
        let analysis = crate::analyze_release_for_target(
            "Movie.Title.2024.MAX.WEB-DL.REPACK.H.264-GRP",
            &context,
        );
        let projected = analysis
            .best_candidate()
            .expect("ordinary release parses")
            .projected
            .clone();

        assert_eq!(
            projected
                .streaming_service
                .as_ref()
                .map(crate::model::StreamingService::as_str),
            Some("HBO Max")
        );
        assert!(projected.is_proper_upload);
        assert!(projected.is_repack);
        assert!(
            projected
                .normalized_tokens
                .iter()
                .any(|token| token == "MAX")
        );
        assert!(analysis.guide_facts.is_empty());
        assert!(projected.guide_facts.is_empty());
    }

    #[test]
    fn token_signal_anchor_index_matches_reference_rule_scans() {
        for tokens in [
            vec!["THE", "UPSCALER", "REPACK"],
            vec!["PROPER", "HARDCODED", "DUBSONLY"],
            vec!["UNRELATED", "TITLE"],
        ] {
            let tokens = tokens.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(
                detect_token_signals(&tokens),
                detect_token_signals_reference(&tokens)
            );
        }
    }

    fn detect_token_signals_reference(normalized_tokens: &[String]) -> TokenSignalMatch {
        let mut matched = TokenSignalMatch::default();
        for rule in TOKEN_SIGNAL_RULES {
            if !pattern_matches(&rule.pattern, normalized_tokens) {
                continue;
            }
            match rule.kind {
                ParserSignalKind::AiEnhanced => matched.ai_enhanced = true,
                ParserSignalKind::Proper => matched.proper = true,
                ParserSignalKind::Repack => {
                    matched.proper = true;
                    matched.repack = true;
                }
                ParserSignalKind::DubsOnly => matched.dubs_only = true,
                ParserSignalKind::HardcodedSubs => matched.hardcoded_subs = true,
            }
        }
        matched
    }
}
