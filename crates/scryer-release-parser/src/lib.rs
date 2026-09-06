//! Deterministic multi-hypothesis release parser with required target context.
//!
//! The crate exposes a lossless tokenization layer, a lightweight CST, token
//! annotations with capped structural ambiguity, and a beam-scored parser that
//! requires target context from the caller.

mod context;
mod enrichment;
mod lex;
mod model;
mod parse;
mod sanitize;
mod trash_guides;

pub use context::{
    ContextAlias, ContextEpisode, ContextFacetHint, ContextTitle, ReleaseParseContext,
};
pub use lex::{BracketKind, CstNode, ReleaseCst, SeparatorKind, TextSpan, Token};
pub use model::{
    AudioCodec, CandidateZones, ContextTitleMatch, ContextTitleMatchKind, ExternalIdSource,
    GuideFact, MetadataAst, MetadataEnrichment, ParseDisposition, ParseFamily, ParseReason,
    ParsedEpisodeMetadata, ParsedEpisodeReleaseType, ParsedExternalId, ParsedReleaseMetadata,
    ParsedSpecialKind, ReleaseIdentity, ReleaseParseAnalysis, ReleaseParseCandidate, ReleaseSource,
    StreamingService, TargetScoredAnalysis, TargetedReleaseParseAnalysis, TitleSegment,
    TitleSegmentKind, TokenAnnotations, TokenRange, TokenRole, VideoCodec,
};
pub use parse::SCORING_MODEL_VERSION;
pub use trash_guides::TRASH_GUIDES_SOURCE_REVISION;
pub use trash_guides::detect_blocked_title as detect_trash_guides_blocked_title;

use parse::{AnalysisInputs, analyze_inputs};
use sanitize::sanitize_input;

const PARSER_VERSION: &str = "2026.09.06-fused-episode-label";

/// Analyze a raw release name against one required target context.
#[must_use]
pub fn analyze_release_for_target(raw: &str, target: &ReleaseParseContext) -> ReleaseParseAnalysis {
    analyze_release_internal(raw, target)
}

/// Analyze a raw release name against multiple candidate targets.
#[must_use]
pub fn analyze_release_against_targets(
    raw: &str,
    targets: &[ReleaseParseContext],
) -> TargetedReleaseParseAnalysis {
    let targets = targets
        .iter()
        .enumerate()
        .map(|(target_index, target)| {
            let analysis = analyze_release_for_target(raw, target);
            let best_score = analysis
                .best_candidate()
                .map(|candidate| candidate.raw_score)
                .unwrap_or(i32::MIN / 2);
            TargetScoredAnalysis {
                target_index,
                analysis,
                best_score,
            }
        })
        .collect::<Vec<_>>();

    let best_target_index = targets
        .iter()
        .filter(|item| !item.analysis.is_unparseable())
        .max_by_key(|item| item.best_score)
        .map(|item| item.target_index);

    TargetedReleaseParseAnalysis {
        targets,
        best_target_index,
    }
}

/// Return the highest-scoring parsed metadata projection for one required target.
#[must_use]
pub fn best_parse_for_target(raw: &str, target: &ReleaseParseContext) -> ParsedReleaseMetadata {
    analyze_release_for_target(raw, target)
        .best_candidate()
        .map(|candidate| candidate.projected.clone())
        .unwrap_or_else(|| ParsedReleaseMetadata::empty(raw, PARSER_VERSION))
}

fn analyze_release_internal(raw: &str, target: &ReleaseParseContext) -> ReleaseParseAnalysis {
    let sanitized = sanitize_input(raw);
    let inputs = AnalysisInputs {
        raw_input: raw,
        sanitized_input: &sanitized.value,
        sanitize_hints: &sanitized.hints,
        parser_version: PARSER_VERSION,
        target,
    };
    analyze_inputs(inputs)
}

#[cfg(test)]
mod tests;
