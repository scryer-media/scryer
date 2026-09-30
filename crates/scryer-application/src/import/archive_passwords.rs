//! Password candidates for import-time archive extraction.
//!
//! Extraction is always attempted without a password first. Only when the
//! archive plugin reports that the archive needs one are these candidates
//! tried, in order. The values are secrets: they never reach logs, errors,
//! import results or events, and their `Debug` output is redacted.

use crate::normalize_release_password;

/// Where an archive password candidate came from. Safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchivePasswordSource {
    /// Typed by the operator when retrying the import.
    Operator,
    /// Announced by the indexer at grab time and stored encrypted.
    Indexer,
    /// Embedded in the client-reported release name as `Name{{password}}`.
    ReleaseName,
}

#[cfg_attr(not(feature = "runtime-archives"), allow(dead_code))]
impl ArchivePasswordSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Indexer => "indexer",
            Self::ReleaseName => "release name",
        }
    }
}

#[derive(Clone)]
pub struct ArchivePasswordCandidate {
    source: ArchivePasswordSource,
    value: String,
}

// Read only by the archive extractor, which the build without archive support
// replaces with a stub.
#[cfg_attr(not(feature = "runtime-archives"), allow(dead_code))]
impl ArchivePasswordCandidate {
    pub fn source(&self) -> ArchivePasswordSource {
        self.source
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Debug for ArchivePasswordCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchivePasswordCandidate")
            .field("source", &self.source)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// An ordered, de-duplicated list of archive password candidates. The first
/// source to offer a value keeps its place; later duplicates are dropped.
#[derive(Debug, Clone, Default)]
pub struct ArchivePasswordCandidates {
    candidates: Vec<ArchivePasswordCandidate>,
}

impl ArchivePasswordCandidates {
    /// The operator's retry password, kept exactly as typed: it is an explicit
    /// answer to a password prompt, never an indexer flag. Blank input is
    /// ignored.
    pub fn push_operator(&mut self, value: Option<&str>) {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            self.push(ArchivePasswordSource::Operator, value.to_string());
        }
    }

    /// A password stored for the release, normalized like every indexer
    /// password so flag values ("1", "yes", "passworded", …) are ignored.
    pub fn push_indexer(&mut self, value: Option<&str>) {
        if let Some(value) = normalize_release_password(value) {
            self.push(ArchivePasswordSource::Indexer, value);
        }
    }

    /// The `{{password}}` suffix of a client-reported release or job name.
    pub fn push_release_name(&mut self, name: Option<&str>) {
        if let Some(value) = name.and_then(release_name_password) {
            self.push(ArchivePasswordSource::ReleaseName, value);
        }
    }

    fn push(&mut self, source: ArchivePasswordSource, value: String) {
        if self
            .candidates
            .iter()
            .any(|candidate| candidate.value == value)
        {
            return;
        }
        self.candidates
            .push(ArchivePasswordCandidate { source, value });
    }

    #[cfg_attr(not(feature = "runtime-archives"), allow(dead_code))]
    pub fn iter(&self) -> impl Iterator<Item = &ArchivePasswordCandidate> {
        self.candidates.iter()
    }

    #[cfg_attr(not(feature = "runtime-archives"), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    #[cfg_attr(not(feature = "runtime-archives"), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

/// The password a release name carries in the SABnzbd/NZBGet convention
/// `Release.Name{{password}}` (an upload's `.nzb` extension tolerated).
///
/// Only a name, never a path: a value with a path separator before the braces,
/// or with anything after them, is a filesystem path whose directory happened
/// to carry braces and yields nothing. Flag-like values are ignored.
pub fn release_name_password(name: &str) -> Option<String> {
    let name = name.trim();
    let name = match name.len().checked_sub(4) {
        Some(split)
            if name.is_char_boundary(split) && name[split..].eq_ignore_ascii_case(".nzb") =>
        {
            &name[..split]
        }
        _ => name,
    };
    let without_close = name.strip_suffix("}}")?;
    let open = without_close.rfind("{{")?;
    let prefix = &without_close[..open];
    let inner = &without_close[open + 2..];
    if prefix.contains(['/', '\\']) || inner.contains(['{', '}']) {
        return None;
    }
    normalize_release_password(Some(inner))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(candidates: &ArchivePasswordCandidates) -> Vec<(ArchivePasswordSource, &str)> {
        candidates
            .iter()
            .map(|candidate| (candidate.source(), candidate.value()))
            .collect()
    }

    #[test]
    fn release_name_password_reads_the_braced_suffix() {
        assert_eq!(
            release_name_password("Quiet.Harbor.S01E01.1080p-NOGRP{{hunter-42}}").as_deref(),
            Some("hunter-42")
        );
        assert_eq!(
            release_name_password("  Quiet.Harbor.S01E01{{ spaced out }}  ").as_deref(),
            Some("spaced out")
        );
        assert_eq!(
            release_name_password("Quiet.Harbor.S01E01{{p4ss}}.NZB").as_deref(),
            Some("p4ss")
        );
        assert_eq!(
            release_name_password("Quiet.Harbor{{old}}.S01E01{{new}}").as_deref(),
            Some("new")
        );
        assert_eq!(
            release_name_password("{{only-a-password}}").as_deref(),
            Some("only-a-password")
        );
        assert_eq!(
            release_name_password("Quiet.Harbor{{pa/ss\\word}}").as_deref(),
            Some("pa/ss\\word")
        );
    }

    #[test]
    fn release_name_password_rejects_malformed_and_path_values() {
        for name in [
            "Quiet.Harbor.S01E01.1080p-NOGRP",
            "Quiet.Harbor{{}}",
            "Quiet.Harbor{{   }}",
            "Quiet.Harbor{{open",
            "Quiet.Harbor}}",
            "Quiet.Harbor{{secret}}.mkv",
            "Quiet.Harbor{{sec{ret}}",
            "Quiet.Harbor{{sec}}ret}}",
            "/downloads/Quiet.Harbor{{secret}}",
            "C:\\downloads\\Quiet.Harbor{{secret}}",
            "/downloads/Quiet.Harbor{{secret}}/quiet.harbor.rar",
            "",
        ] {
            assert_eq!(release_name_password(name), None, "{name:?}");
        }
    }

    #[test]
    fn release_name_password_ignores_flag_values() {
        for flag in ["1", "true", "YES", "passworded", "protected", "0", "no"] {
            assert_eq!(
                release_name_password(&format!("Quiet.Harbor{{{{{flag}}}}}")),
                None,
                "{flag:?}"
            );
        }
    }

    #[test]
    fn candidates_keep_source_order_and_drop_duplicates() {
        let mut candidates = ArchivePasswordCandidates::default();
        candidates.push_operator(Some(" typed "));
        candidates.push_indexer(Some("from-indexer"));
        candidates.push_release_name(Some("Quiet.Harbor{{from-indexer}}"));
        candidates.push_release_name(Some("Quiet.Harbor{{from-name}}"));
        candidates.push_release_name(Some("Quiet.Harbor{{from-name}}"));

        assert_eq!(
            values(&candidates),
            [
                (ArchivePasswordSource::Operator, " typed "),
                (ArchivePasswordSource::Indexer, "from-indexer"),
                (ArchivePasswordSource::ReleaseName, "from-name"),
            ]
        );
    }

    #[test]
    fn candidates_ignore_blank_and_flag_values() {
        let mut candidates = ArchivePasswordCandidates::default();
        candidates.push_operator(Some("   "));
        candidates.push_operator(None);
        candidates.push_indexer(Some("passworded"));
        candidates.push_indexer(Some("1"));
        candidates.push_indexer(None);
        candidates.push_release_name(Some("Quiet.Harbor{{yes}}"));
        candidates.push_release_name(None);

        assert!(candidates.is_empty());
    }

    #[test]
    fn candidate_debug_output_never_contains_the_value() {
        let mut candidates = ArchivePasswordCandidates::default();
        candidates.push_indexer(Some("do-not-print-me"));

        let rendered = format!("{candidates:?}");
        assert!(!rendered.contains("do-not-print-me"), "{rendered}");
        assert!(rendered.contains("Indexer"), "{rendered}");
    }
}
