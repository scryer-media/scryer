use std::collections::BTreeMap;

use async_trait::async_trait;
use scryer_domain::MediaFacet;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenameWriteAction {
    Noop,
    Move,
    Replace,
    Skip,
    Error,
}

impl RenameWriteAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Move => "move",
            Self::Replace => "replace",
            Self::Skip => "skip",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenameApplyStatus {
    Applied,
    Skipped,
    Failed,
}

impl RenameApplyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RenameCollisionPolicy {
    #[default]
    Skip,
    Error,
    ReplaceIfBetter,
}

impl RenameCollisionPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Error => "error",
            Self::ReplaceIfBetter => "replace_if_better",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RenameMissingMetadataPolicy {
    Skip,
    #[default]
    FallbackTitle,
}

impl RenameMissingMetadataPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::FallbackTitle => "fallback_title",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePlanItem {
    pub collection_id: Option<String>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub normalized_filename: Option<String>,
    pub collision: bool,
    pub reason_code: String,
    pub write_action: RenameWriteAction,
    pub source_size_bytes: Option<u64>,
    pub source_mtime_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePlan {
    pub facet: MediaFacet,
    pub title_id: Option<String>,
    pub template: String,
    pub collision_policy: RenameCollisionPolicy,
    pub missing_metadata_policy: RenameMissingMetadataPolicy,
    pub fingerprint: String,
    pub total: usize,
    pub renamable: usize,
    pub noop: usize,
    pub conflicts: usize,
    pub errors: usize,
    pub items: Vec<RenamePlanItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameApplyItemResult {
    pub collection_id: Option<String>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub final_path: Option<String>,
    pub write_action: RenameWriteAction,
    pub status: RenameApplyStatus,
    pub reason_code: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameApplyResult {
    pub plan_fingerprint: String,
    pub total: usize,
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub items: Vec<RenameApplyItemResult>,
}

#[async_trait]
pub trait LibraryRenamer: Send + Sync {
    async fn validate_targets(&self, plan: &RenamePlan) -> AppResult<()>;
    async fn apply_plan(&self, plan: &RenamePlan) -> AppResult<Vec<RenameApplyItemResult>>;
    async fn rollback(
        &self,
        applied_items: &[RenameApplyItemResult],
    ) -> AppResult<Vec<RenameApplyItemResult>>;
}

#[derive(Default)]
pub struct NullLibraryRenamer;

#[async_trait]
impl LibraryRenamer for NullLibraryRenamer {
    async fn validate_targets(&self, _plan: &RenamePlan) -> AppResult<()> {
        Err(AppError::Repository(
            "library renamer is not configured".into(),
        ))
    }

    async fn apply_plan(&self, _plan: &RenamePlan) -> AppResult<Vec<RenameApplyItemResult>> {
        Err(AppError::Repository(
            "library renamer is not configured".into(),
        ))
    }

    async fn rollback(
        &self,
        _applied_items: &[RenameApplyItemResult],
    ) -> AppResult<Vec<RenameApplyItemResult>> {
        Ok(Vec::new())
    }
}

pub fn render_rename_template(template: &str, tokens: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut cursor = 0usize;

    while cursor < chars.len() {
        let ch = chars[cursor];
        if ch != '{' {
            out.push(ch);
            cursor += 1;
            continue;
        }

        if let Some(end) = chars[cursor + 1..].iter().position(|c| *c == '}') {
            let end_index = cursor + 1 + end;
            let token_spec: String = chars[cursor + 1..end_index].iter().collect();
            out.push_str(&resolve_template_token(tokens, token_spec.trim()));
            cursor = end_index + 1;
            continue;
        }

        out.push(ch);
        cursor += 1;
    }

    sanitize_filesystem_component(&out)
}

pub fn build_rename_plan_fingerprint(
    items: &[RenamePlanItem],
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
) -> String {
    let bytes = serde_json::to_vec(&(
        template,
        collision_policy.as_str(),
        missing_metadata_policy.as_str(),
        items,
    ))
    .unwrap_or_default();
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

fn resolve_template_token(tokens: &BTreeMap<String, String>, token_spec: &str) -> String {
    let (name, pad_width) = match token_spec.split_once(':') {
        Some((n, fmt)) => (n.trim().to_lowercase(), fmt.trim().parse::<usize>().ok()),
        None => (token_spec.trim().to_lowercase(), None),
    };
    let raw = tokens.get(&name).cloned().unwrap_or_default();
    match pad_width {
        Some(width) if width > 0 => {
            if raw.chars().all(|c| c.is_ascii_digit()) && !raw.is_empty() {
                format!("{:0>width$}", raw, width = width)
            } else {
                raw
            }
        }
        _ => raw,
    }
}

fn sanitize_filesystem_component(raw: &str) -> String {
    let mut sanitized = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            sanitized.push(' ');
        } else {
            sanitized.push(ch);
        }
    }

    collapse_separators(&sanitized)
}

fn collapse_separators(raw: &str) -> String {
    let mut collapsed = String::with_capacity(raw.len());
    let mut previous: Option<char> = None;

    for ch in raw.chars() {
        let normalized = if ch.is_whitespace() { ' ' } else { ch };
        let is_separator = matches!(normalized, ' ' | '.' | '-' | '_');
        if is_separator && previous.is_some_and(|prev| prev == normalized) {
            continue;
        }
        collapsed.push(normalized);
        previous = Some(normalized);
    }

    collapsed
        .trim_matches(|value: char| value.is_whitespace() || matches!(value, '.' | '-' | '_'))
        .to_string()
}

#[cfg(test)]
#[path = "library_rename_tests.rs"]
mod library_rename_tests;
