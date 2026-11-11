//! Translates pasted Sonarr and Radarr custom-format JSON into a Rego rule.
//!
//! The translation is a pure function of the pasted text, its source
//! application, and the operator's per-format scores. It stores nothing; the
//! operator reviews the generated source in the rule editor before saving it.

mod inspection;
mod js;
mod onig;
mod rego;
mod specifications;
mod translate;

#[cfg(test)]
mod tests;

use crate::error_reasons::arr_custom_formats as reasons;
use crate::{AppError, RefusalFact};
use std::collections::HashMap;

/// The largest paste accepted, in UTF-8 bytes.
pub const MAX_PASTED_JSON_BYTES: usize = 16 * 1024 * 1024;
/// The most custom formats one paste may hold.
pub const MAX_FORMATS: usize = inspection::MAX_FORMATS;

/// Stable diagnostic codes. Clients branch on these, never on the message.
pub mod codes {
    /// A pasted custom format is not a JSON object.
    pub const FORMAT_INVALID: &str = "arr.format.invalid";
    /// A pasted custom format has no name.
    pub const FORMAT_NAME_MISSING: &str = "arr.format.name_missing";
    /// A pasted custom format has no specifications.
    pub const FORMAT_SPECIFICATIONS_MISSING: &str = "arr.format.specifications_missing";
    /// A format lists more specifications than the import reads.
    pub const SPECIFICATIONS_TOO_MANY: &str = "arr.specifications.too_many";
    /// A specification is malformed.
    pub const SPECIFICATION_INVALID: &str = "arr.specification.invalid";
    /// The exported score and `trash_scores.default` disagree.
    pub const SCORE_CONFLICT: &str = "arr.score.conflict";
    /// The format exceeds the specification limit.
    pub const FORMAT_TOO_LARGE: &str = "arr.format.too_large";
    /// No specification of the format survived inspection.
    pub const SPECIFICATIONS_EMPTY: &str = "arr.specifications.empty";
    /// No usable score was supplied for the format.
    pub const SCORE_INVALID: &str = "arr.score.invalid";
    /// A regex specification has no regex.
    pub const REGEX_VALUE_INVALID: &str = "regex.value_invalid";
    /// A regex holds a control character.
    pub const REGEX_CONTROL_CHARACTER: &str = "regex.control_character";
    /// A regex exceeds the length limit.
    pub const REGEX_TOO_LARGE: &str = "regex.too_large";
    /// A regex uses a backreference.
    pub const REGEX_BACKREFERENCE_UNSUPPORTED: &str = "regex.backreference_unsupported";
    /// A regex uses a .NET construct with no Rego equivalent.
    pub const REGEX_RE2_INCOMPATIBLE: &str = "regex.re2_incompatible";
    /// A regex could not be parsed or uses an unsupported construct.
    pub const REGEX_PARSE_FAILED: &str = "regex.parse_failed";
    /// A regex quantifies a positional assertion.
    pub const REGEX_QUANTIFIED_LOOKAROUND_UNSUPPORTED: &str =
        "regex.quantified_lookaround_unsupported";
    /// A specification carries fields the translator does not read.
    pub const FIELDS_UNSUPPORTED: &str = "arr.fields.unsupported";
    /// An edition specification outside Radarr.
    pub const EDITION_UNMAPPED: &str = "arr.edition.unmapped";
    /// The specification implementation is unknown.
    pub const IMPLEMENTATION_UNSUPPORTED: &str = "arr.implementation.unsupported";
    /// A specification value is not an integer.
    pub const VALUE_INVALID: &str = "arr.value.invalid";
    /// A resolution has no Scryer equivalent.
    pub const RESOLUTION_UNMAPPED: &str = "arr.resolution.unmapped";
    /// A source has no Scryer equivalent.
    pub const SOURCE_UNMAPPED: &str = "arr.source.unmapped";
    /// A quality modifier has no Scryer equivalent.
    pub const QUALITY_MODIFIER_UNMAPPED: &str = "arr.quality_modifier.unmapped";
    /// A release type has no Scryer equivalent.
    pub const RELEASE_TYPE_UNMAPPED: &str = "arr.release_type.unmapped";
    /// An indexer flag has no Scryer equivalent.
    pub const INDEXER_FLAG_UNMAPPED: &str = "arr.indexer_flag.unmapped";
    /// `exceptLanguage` is not a boolean.
    pub const LANGUAGE_INVALID: &str = "arr.language.invalid";
    /// A language has no Scryer equivalent.
    pub const LANGUAGE_UNMAPPED: &str = "arr.language.unmapped";
    /// Year conditions need metadata the rule input lacks.
    pub const YEAR_UNMAPPED: &str = "arr.year.unmapped";
    /// A size range is malformed.
    pub const RANGE_INVALID: &str = "arr.range.invalid";
    /// A size range exceeds exact integer precision.
    pub const RANGE_UNSAFE: &str = "arr.range.unsafe";

    /// Every diagnostic code the translator emits.
    pub const ALL: &[&str] = &[
        FORMAT_INVALID,
        FORMAT_NAME_MISSING,
        FORMAT_SPECIFICATIONS_MISSING,
        SPECIFICATIONS_TOO_MANY,
        SPECIFICATION_INVALID,
        SCORE_CONFLICT,
        FORMAT_TOO_LARGE,
        SPECIFICATIONS_EMPTY,
        SCORE_INVALID,
        REGEX_VALUE_INVALID,
        REGEX_CONTROL_CHARACTER,
        REGEX_TOO_LARGE,
        REGEX_BACKREFERENCE_UNSUPPORTED,
        REGEX_RE2_INCOMPATIBLE,
        REGEX_PARSE_FAILED,
        REGEX_QUANTIFIED_LOOKAROUND_UNSUPPORTED,
        FIELDS_UNSUPPORTED,
        EDITION_UNMAPPED,
        IMPLEMENTATION_UNSUPPORTED,
        VALUE_INVALID,
        RESOLUTION_UNMAPPED,
        SOURCE_UNMAPPED,
        QUALITY_MODIFIER_UNMAPPED,
        RELEASE_TYPE_UNMAPPED,
        INDEXER_FLAG_UNMAPPED,
        LANGUAGE_INVALID,
        LANGUAGE_UNMAPPED,
        YEAR_UNMAPPED,
        RANGE_INVALID,
        RANGE_UNSAFE,
    ];
}

/// The application a custom-format export came from. Sonarr and Radarr number
/// sources and indexer flags differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrSourceApplication {
    Sonarr,
    Radarr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrDiagnosticSeverity {
    Error,
    Warning,
    Info,
}

/// One finding about the paste. `code` is one of [`codes::ALL`].
#[derive(Debug, Clone, PartialEq)]
pub struct ArrDiagnostic {
    pub code: &'static str,
    pub severity: ArrDiagnosticSeverity,
    pub message: String,
    /// JSON-path-like location in the pasted document, when known.
    pub location: Option<String>,
    pub format_id: Option<String>,
    pub specification_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrSpecificationSummary {
    pub index: usize,
    pub implementation: String,
    pub name: String,
    pub negate: bool,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrFormatStatus {
    Translated,
    Disabled,
}

/// One pasted format: what inspection read, and how translation went.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrTranslatedFormat {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub status: ArrFormatStatus,
    pub diagnostics: Vec<ArrDiagnostic>,
    /// Scores the export suggests, labelled as exported, in export order.
    pub suggested_scores: Vec<(String, f64)>,
    pub specifications: Vec<ArrSpecificationSummary>,
    pub inspection_diagnostics: Vec<ArrDiagnostic>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrCustomFormatTranslation {
    pub rego_source: String,
    pub name: String,
    pub description: String,
    pub applied_facets: Vec<String>,
    pub formats: Vec<ArrTranslatedFormat>,
    /// Diagnostics of the disabled formats.
    pub diagnostics: Vec<ArrDiagnostic>,
    /// Everything inspection found, including formats it could not read.
    pub inspection_diagnostics: Vec<ArrDiagnostic>,
}

/// A paste the translator cannot read at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrImportRefusal {
    JsonTooLarge,
    JsonInvalid,
    FormatsEmpty,
    FormatsTooMany,
}

impl ArrImportRefusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::JsonTooLarge => reasons::JSON_TOO_LARGE,
            Self::JsonInvalid => reasons::JSON_INVALID,
            Self::FormatsEmpty => reasons::FORMATS_EMPTY,
            Self::FormatsTooMany => reasons::FORMATS_TOO_MANY,
        }
    }

    pub fn message(self) -> String {
        match self {
            Self::JsonTooLarge => {
                format!("Paste no more than {MAX_PASTED_JSON_BYTES} bytes of JSON.")
            }
            Self::JsonInvalid => "Paste valid JSON exported by Sonarr or Radarr.".to_string(),
            Self::FormatsEmpty => "Paste at least one custom format.".to_string(),
            Self::FormatsTooMany => {
                format!("Paste no more than {MAX_FORMATS} custom formats at once.")
            }
        }
    }

    fn facts(self) -> Vec<RefusalFact> {
        match self {
            Self::JsonTooLarge => vec![RefusalFact::new(
                reasons::MAX_BYTES_FACT,
                MAX_PASTED_JSON_BYTES.to_string(),
            )],
            Self::FormatsTooMany => vec![RefusalFact::new(
                reasons::MAX_FORMATS_FACT,
                MAX_FORMATS.to_string(),
            )],
            Self::JsonInvalid | Self::FormatsEmpty => Vec::new(),
        }
    }
}

impl ArrImportRefusal {
    pub fn into_app_error(self) -> AppError {
        AppError::validation_refused_with_facts(self.reason(), self.message(), self.facts())
    }
}

/// Inspects and translates pasted custom-format JSON. Formats without a score
/// in `scores` translate as disabled, so an empty map reviews the paste.
pub fn translate_arr_custom_formats(
    source: ArrSourceApplication,
    json: &str,
    scores: &HashMap<String, i32>,
) -> Result<ArrCustomFormatTranslation, ArrImportRefusal> {
    let inspected = inspection::inspect(json, source)?;
    let translated = translate::translate_formats(&inspected.formats, scores);
    let formats = inspected
        .formats
        .into_iter()
        .zip(translated.formats)
        .map(|(format, outcome)| ArrTranslatedFormat {
            specifications: format
                .specifications
                .iter()
                .map(|spec| ArrSpecificationSummary {
                    index: spec.index,
                    implementation: spec.implementation.clone(),
                    name: spec.name.clone(),
                    negate: spec.negate,
                    required: spec.required,
                })
                .collect(),
            id: format.id,
            name: format.name,
            description: format.description,
            status: outcome.status,
            diagnostics: outcome.diagnostics,
            suggested_scores: format.suggested_scores,
            inspection_diagnostics: format.inspection_diagnostics,
        })
        .collect();
    Ok(ArrCustomFormatTranslation {
        rego_source: translated.rego_source,
        name: translated.name,
        description: translated.description,
        applied_facets: translated.applied_facets,
        formats,
        diagnostics: translated.diagnostics,
        inspection_diagnostics: inspected.diagnostics,
    })
}
