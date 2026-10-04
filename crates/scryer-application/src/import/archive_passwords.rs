//! Password candidates for import-time archive extraction.
//!
//! Extraction is always attempted without a password first. Only when the
//! archive plugin reports that the archive needs one are these candidates
//! tried, in order. The values are secrets: they never reach logs, errors,
//! import results or events, and their `Debug` output is redacted.

/// Where an archive password candidate came from. Safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchivePasswordSource {
    /// Typed by the operator when retrying the import.
    Operator,
    /// Announced by the indexer at grab time and stored encrypted.
    Indexer,
    /// Embedded in the client-reported release name as `Name{{password}}`.
    ReleaseName,
    /// Supplied by the installed archive plugin's secret settings.
    PluginSettings,
    /// Preserved from the NZB download response.
    ResponseHeader,
}

#[cfg_attr(not(feature = "runtime-archives"), allow(dead_code))]
impl ArchivePasswordSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Indexer => "indexer",
            Self::ReleaseName => "release name",
            Self::PluginSettings => "plugin settings",
            Self::ResponseHeader => "response header",
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
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            self.push(ArchivePasswordSource::Operator, value.to_string());
        }
    }

    /// A literal password stored for the release. Marker fields are classified
    /// by the indexer adapter before reaching this boundary.
    pub fn push_indexer(&mut self, value: Option<&str>) {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            self.push(ArchivePasswordSource::Indexer, value.to_string());
        }
    }

    /// The `{{password}}` suffix of a client-reported release or job name.
    pub fn push_release_name(&mut self, name: Option<&str>) {
        if let Some(value) = name.and_then(release_name_password) {
            self.push(ArchivePasswordSource::ReleaseName, value);
        }
    }

    /// Snapshot operator settings without a candidate-count cap. Whitespace
    /// inside a nonblank line is part of the password.
    pub fn extend_settings(&mut self, content: &str) {
        let mut seen: std::collections::HashSet<String> = self
            .candidates
            .iter()
            .map(|candidate| candidate.value.clone())
            .collect();
        for line in content.lines().filter(|line| !line.trim().is_empty()) {
            if seen.insert(line.to_string()) {
                self.candidates.push(ArchivePasswordCandidate {
                    source: ArchivePasswordSource::PluginSettings,
                    value: line.to_string(),
                });
            }
        }
    }

    pub fn push_response_header(&mut self, value: &str) {
        if !value.is_empty() {
            self.push(ArchivePasswordSource::ResponseHeader, value.to_string());
        }
    }

    pub fn extend(&mut self, other: Self) {
        for candidate in other.candidates {
            self.push(candidate.source, candidate.value);
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

/// Mask password conventions in diagnostics, including names inside paths or
/// decoder errors. This does not discover candidates or alter filesystem paths.
pub fn redact_archive_diagnostic(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(open) = remaining.find("{{") {
        output.push_str(&remaining[..open]);
        output.push_str("[redacted]");
        let secret = &remaining[open + 2..];
        remaining = match secret.find("}}") {
            Some(close) => &secret[close + 2..],
            None => "",
        };
    }
    output.push_str(remaining);
    if let Some(start) = output.to_ascii_lowercase().find(" password=") {
        output.truncate(start);
        output.push_str(" [redacted]");
    }
    if let Some(start) = output.find(" / ") {
        output.truncate(start);
        output.push_str(" [redacted]");
    }
    output
}

/// Read a password from a release/job label, never a URL or filesystem path.
/// Password contents are literal, including flag-like values and whitespace.
pub fn release_name_password(name: &str) -> Option<String> {
    release_password_parts(name).map(|(_, password)| password.to_string())
}

pub fn release_name_without_password(name: &str) -> &str {
    release_password_parts(name)
        .map(|(name, _)| name.trim_end())
        .unwrap_or(name)
}

fn release_password_parts(name: &str) -> Option<(&str, &str)> {
    let name = name.trim_start();
    let name = if name.trim_end().ends_with("}}") && name.contains("{{") {
        name.trim_end()
    } else {
        name
    };
    let name = match name.len().checked_sub(4) {
        Some(split)
            if name.is_char_boundary(split) && name[split..].eq_ignore_ascii_case(".nzb") =>
        {
            &name[..split]
        }
        _ => name,
    };
    let (prefix, inner) = if let Some(without_close) = name.strip_suffix("}}")
        && let Some(open) = without_close.rfind("{{")
    {
        let inner = &without_close[open + 2..];
        if inner.contains(['{', '}']) {
            return None;
        }
        (&without_close[..open], inner)
    } else if let Some((prefix, inner)) = name.rsplit_once(" password=") {
        (prefix, inner)
    } else {
        name.split_once(" / ")?
    };
    let drive_prefix = prefix.as_bytes().get(1) == Some(&b':')
        && prefix
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
    if prefix.contains(['/', '\\']) || drive_prefix || inner.trim().is_empty() {
        return None;
    }
    Some((prefix, inner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_diagnostics_redact_filename_passwords_inside_paths_and_errors() {
        for value in [
            "/downloads/Release{{synthetic-secret}}/member{{synthetic-secret}}.mkv",
            "unsafe member: Release password=synthetic-secret.mkv",
            "archive Release / synthetic-secret could not be opened",
            "Release{{synthetic-secret",
        ] {
            let safe = redact_archive_diagnostic(value);
            assert!(!safe.contains("synthetic-secret"));
            assert!(safe.contains("[redacted]"));
        }
        assert_eq!(
            redact_archive_diagnostic("Release{{secret}}.7z"),
            "Release[redacted].7z"
        );
        assert_eq!(
            redact_archive_diagnostic("ordinary failure"),
            "ordinary failure"
        );
    }

    fn values(candidates: &ArchivePasswordCandidates) -> Vec<(ArchivePasswordSource, &str)> {
        candidates
            .iter()
            .map(|candidate| (candidate.source(), candidate.value()))
            .collect()
    }

    #[test]
    fn settings_passwords_are_deduplicated_and_redacted() {
        let mut candidates = ArchivePasswordCandidates::default();
        candidates.push_operator(Some("synthetic-one"));
        candidates.extend_settings("synthetic-one\r\nsynthetic-two\nsynthetic-one\n");
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates.iter().last().unwrap().source(),
            ArchivePasswordSource::PluginSettings
        );
        assert!(!format!("{candidates:?}").contains("synthetic"));
        candidates.extend_settings(
            &(0..18)
                .map(|i| format!("synthetic-{i}\n"))
                .collect::<String>(),
        );
        assert_eq!(candidates.len(), 20);
    }

    #[test]
    fn release_name_password_reads_the_braced_suffix() {
        assert_eq!(
            release_name_password("Quiet.Harbor.S01E01.1080p-NOGRP{{hunter-42}}").as_deref(),
            Some("hunter-42")
        );
        assert_eq!(
            release_name_password("  Quiet.Harbor.S01E01{{ spaced out }}  ").as_deref(),
            Some(" spaced out ")
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
    fn release_name_password_preserves_literal_values_in_all_conventions() {
        for flag in ["1", "true", "YES", "passworded", "protected", "0", "no"] {
            assert_eq!(
                release_name_password(&format!("Quiet.Harbor{{{{{flag}}}}}")),
                Some(flag.to_string()),
                "{flag:?}"
            );
            assert_eq!(
                release_name_password(&format!("Quiet.Harbor password={flag}")),
                Some(flag.to_string())
            );
            assert_eq!(
                release_name_password(&format!("Quiet.Harbor / {flag}")),
                Some(flag.to_string())
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
    fn candidates_ignore_missing_values_and_preserve_explicit_whitespace() {
        let mut candidates = ArchivePasswordCandidates::default();
        candidates.push_operator(Some(""));
        candidates.push_operator(None);
        candidates.push_indexer(None);
        candidates.push_release_name(None);

        assert!(candidates.is_empty());
        candidates.push_operator(Some("   "));
        assert_eq!(
            values(&candidates),
            [(ArchivePasswordSource::Operator, "   ")]
        );
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
