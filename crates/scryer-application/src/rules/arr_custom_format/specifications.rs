//! Non-regex Arr specifications mapped onto Scryer rule inputs.
//!
//! Sonarr and Radarr number their sources and indexer flags differently, so
//! each application keeps its own map.

use super::codes;
use super::inspection::{ImportedFormat, Specification};
use super::js::{is_safe_integer, js_number_to_string};
use super::rego::{
    Expr, Term, call, call_term, compare, member, number, ref_terms, reference, text, var,
};
use super::{ArrDiagnostic, ArrDiagnosticSeverity, ArrSourceApplication};
use serde_json::Value;

pub(super) struct Compiled {
    pub(super) body: Vec<Expr>,
    pub(super) alternatives: Vec<Vec<Expr>>,
    pub(super) rules: Vec<super::rego::Rule>,
    pub(super) diagnostics: Vec<ArrDiagnostic>,
    pub(super) handles_negation: bool,
}

impl Compiled {
    pub(super) fn ok(body: Vec<Expr>) -> Self {
        Self {
            body,
            alternatives: Vec::new(),
            rules: Vec::new(),
            diagnostics: Vec::new(),
            handles_negation: false,
        }
    }

    pub(super) fn failed(diagnostic: ArrDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            ..Self::ok(Vec::new())
        }
    }
}

// Arr's language ids are distinct from ISO codes. Regional variants without a
// distinct Scryer representation are deliberately absent (e.g. 30 and 37).
const LANGUAGES: &[(i64, &str)] = &[
    (0, "und"),
    (1, "eng"),
    (2, "fra"),
    (3, "spa"),
    (4, "deu"),
    (5, "ita"),
    (6, "dan"),
    (7, "nld"),
    (8, "jpn"),
    (9, "isl"),
    (10, "zho"),
    (11, "rus"),
    (12, "pol"),
    (13, "vie"),
    (14, "swe"),
    (15, "nor"),
    (16, "fin"),
    (17, "tur"),
    (18, "por"),
    (20, "ell"),
    (21, "kor"),
    (22, "hun"),
    (23, "heb"),
    (24, "lit"),
    (25, "ces"),
    (26, "hin"),
    (27, "ron"),
    (28, "tha"),
    (29, "bul"),
    (31, "ara"),
    (32, "ukr"),
    (33, "fas"),
    (34, "ben"),
    (35, "slk"),
    (36, "lav"),
    (38, "cat"),
    (39, "hrv"),
    (40, "srp"),
    (41, "bos"),
    (42, "est"),
    (43, "tam"),
    (44, "ind"),
    (45, "tel"),
    (46, "mkd"),
    (47, "slv"),
    (48, "mal"),
    (49, "kan"),
    (50, "sqi"),
    (51, "afr"),
    (52, "mar"),
    (53, "tgl"),
    (54, "urd"),
    (55, "roh"),
    (56, "mon"),
    (57, "kat"),
];

const FIELDS: &[(&str, &[&str])] = &[
    ("ResolutionSpecification", &["value"]),
    ("SourceSpecification", &["value"]),
    ("QualityModifierSpecification", &["value"]),
    ("ReleaseTypeSpecification", &["value"]),
    ("IndexerFlagSpecification", &["value"]),
    ("LanguageSpecification", &["value", "exceptLanguage"]),
    ("SizeSpecification", &["min", "max"]),
    ("YearSpecification", &["min", "max"]),
];

pub(super) const REGEX_IMPLEMENTATIONS: &[&str] = &[
    "ReleaseTitleSpecification",
    "ReleaseGroupSpecification",
    "EditionSpecification",
];

fn release(field: &str) -> Term {
    reference(&["input", "release", field])
}

fn numeric(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn integer(value: Option<&Value>) -> Option<f64> {
    numeric(value).filter(|value| is_safe_integer(*value))
}

/// Arr's `Convert.ToInt64(double)` rounds half to even after converting GiB to bytes.
fn gib_bytes(value: f64) -> f64 {
    let bytes = value * 1024f64.powi(3);
    let floor = bytes.floor();
    if bytes - floor == 0.5 {
        floor + floor % 2.0
    } else {
        bytes.round()
    }
}

fn strings(values: &[&str]) -> Vec<Term> {
    values.iter().map(|value| text(*value)).collect()
}

/// Compiles a non-regex specification. Returns `None` for regex kinds.
pub(super) fn compile_leaf(spec: &Specification, format: &ImportedFormat) -> Option<Compiled> {
    if REGEX_IMPLEMENTATIONS.contains(&spec.implementation.as_str()) {
        return None;
    }
    let fail = |code: &'static str, message: String| {
        Compiled::failed(ArrDiagnostic {
            code,
            severity: ArrDiagnosticSeverity::Error,
            message,
            location: Some(format!("specifications[{}].fields", spec.index)),
            format_id: Some(format.id.clone()),
            specification_index: Some(spec.index),
        })
    };
    let implementation = spec.implementation.as_str();
    let fields = FIELDS
        .iter()
        .find(|(name, _)| *name == implementation)
        .map(|(_, fields)| *fields);
    let Some(fields) = fields else {
        return Some(fail(
            codes::IMPLEMENTATION_UNSUPPORTED,
            format!("Unsupported Arr implementation {implementation}."),
        ));
    };
    if spec
        .fields
        .iter()
        .any(|(key, _)| !fields.contains(&key.as_str()))
    {
        return Some(fail(
            codes::FIELDS_UNSUPPORTED,
            format!(
                "Unrecognized field in {implementation}; the format is disabled to preserve its conditions."
            ),
        ));
    }
    let raw_value = spec.field("value");
    let value = integer(raw_value);
    if fields.contains(&"value") && value.is_none() {
        return Some(fail(
            codes::VALUE_INVALID,
            format!("{implementation} requires an integer value."),
        ));
    }
    let value = value.unwrap_or(0.0);
    let shown = js_number_to_string(value);
    let radarr = format.source == ArrSourceApplication::Radarr;
    let sonarr = format.source == ArrSourceApplication::Sonarr;
    let compiled = match implementation {
        "ResolutionSpecification" => {
            if ![0.0, 480.0, 576.0, 720.0, 1080.0, 2160.0].contains(&value) {
                return Some(fail(
                    codes::RESOLUTION_UNMAPPED,
                    format!("Unsupported resolution {shown}."),
                ));
            }
            if value == 0.0 {
                Compiled::ok(vec![compare("equal", release("quality"), Term::Null)])
            } else {
                Compiled::ok(vec![member(
                    call_term("lower", vec![release("quality")]),
                    vec![text(format!("{shown}p")), text(format!("{shown}i"))],
                )])
            }
        }
        "SourceSpecification" => {
            let normalized = call_term("scryer.normalize_source", vec![release("source")]);
            if value == 0.0 {
                Compiled::ok(vec![compare("equal", release("source"), Term::Null)])
            } else if radarr {
                let mapped: Option<&[&str]> = match value as i64 {
                    1 => Some(&["CAM"]),
                    2 => Some(&["TELESYNC"]),
                    3 => Some(&["TELECINE"]),
                    4 => Some(&["WORKPRINT"]),
                    5 => Some(&["DVD", "DVDSCR"]),
                    6 => Some(&["HDTV"]),
                    7 => Some(&["WEB-DL"]),
                    8 => Some(&["WEBRIP"]),
                    9 => Some(&["BLURAY", "BRDISK"]),
                    _ => None,
                };
                match mapped {
                    Some(mapped) => Compiled::ok(vec![member(normalized, strings(mapped))]),
                    None => {
                        return Some(fail(
                            codes::SOURCE_UNMAPPED,
                            format!("Unsupported Radarr source {shown}."),
                        ));
                    }
                }
            } else {
                let mapped: Option<&[&str]> = match value as i64 {
                    1 => Some(&["HDTV"]),
                    3 => Some(&["WEB-DL"]),
                    4 => Some(&["WEBRIP"]),
                    5 => Some(&["DVD"]),
                    6 | 7 => Some(&["BLURAY"]),
                    _ => None,
                };
                let Some(mapped) = mapped else {
                    return Some(fail(
                        codes::SOURCE_UNMAPPED,
                        format!("Sonarr source {shown} has no distinct Scryer input."),
                    ));
                };
                let mut body = vec![member(normalized, strings(mapped))];
                if value == 6.0 || value == 7.0 {
                    body.push(compare(
                        "equal",
                        release("is_remux"),
                        Term::Bool(value == 7.0),
                    ));
                }
                // Scryer folds RAWHD into HDTV; the original token distinguishes the
                // explicit raw-TV form when testing ordinary Television source.
                if value == 1.0 {
                    body.push(
                        call(
                            "regex.match",
                            vec![text("(?i)\\bRAW[ ._-]?HD\\b"), release("raw_title")],
                        )
                        .negated(true),
                    );
                }
                Compiled::ok(body)
            }
        }
        "QualityModifierSpecification" => {
            if !radarr {
                return Some(fail(
                    codes::QUALITY_MODIFIER_UNMAPPED,
                    "Quality modifiers are Radarr specifications.".to_string(),
                ));
            }
            match value as i64 {
                5 => Compiled::ok(vec![compare(
                    "equal",
                    release("is_remux"),
                    Term::Bool(true),
                )]),
                4 => Compiled::ok(vec![compare(
                    "equal",
                    release("is_bd_disk"),
                    Term::Bool(true),
                )]),
                2 => Compiled::ok(vec![compare(
                    "equal",
                    call_term("scryer.normalize_source", vec![release("source")]),
                    text("DVDSCR"),
                )]),
                _ => {
                    return Some(fail(
                        codes::QUALITY_MODIFIER_UNMAPPED,
                        format!(
                            "Quality modifier {shown} cannot be distinguished reliably from existing inputs."
                        ),
                    ));
                }
            }
        }
        "ReleaseTypeSpecification" => {
            if !sonarr {
                return Some(fail(
                    codes::RELEASE_TYPE_UNMAPPED,
                    "Release types are Sonarr specifications.".to_string(),
                ));
            }
            let mapped = match value as i64 {
                0 => "unknown",
                1 => "single_episode",
                2 => "multi_episode",
                3 => "season_pack",
                _ => {
                    return Some(fail(
                        codes::RELEASE_TYPE_UNMAPPED,
                        format!("Unsupported release type {shown}."),
                    ));
                }
            };
            Compiled::ok(vec![compare(
                "equal",
                release("episode_release_type"),
                text(mapped),
            )])
        }
        "IndexerFlagSpecification" => {
            let extra = || reference(&["input", "release", "extra"]);
            let factor =
                |key: &str| call_term("object.get", vec![extra(), text(key), number(-1.0)]);
            let download = || factor("downloadvolumefactor");
            let upload = factor("uploadvolumefactor");
            let (internal, scene, factor75, factor25) = if sonarr {
                (8.0, 16.0, 32.0, 64.0)
            } else {
                (32.0, 128.0, 256.0, 512.0)
            };
            if value == 1.0 {
                let free = call_term(
                    "object.get",
                    vec![extra(), text("freeleech"), Term::Bool(false)],
                );
                // Either Torznab's explicit flag or its normalized volume factor.
                Compiled {
                    alternatives: vec![vec![compare("equal", download(), number(0.0))]],
                    ..Compiled::ok(vec![compare("equal", free, Term::Bool(true))])
                }
            } else if value == 2.0 {
                Compiled::ok(vec![compare("equal", download(), number(0.5))])
            } else if value == 4.0 {
                Compiled::ok(vec![compare("equal", upload, number(2.0))])
            } else if value == factor75 {
                Compiled::ok(vec![compare("equal", download(), number(0.75))])
            } else if value == factor25 {
                Compiled::ok(vec![compare("equal", download(), number(0.25))])
            } else if value == internal || value == scene {
                let flag = ref_terms(vec![
                    var("input"),
                    text("release"),
                    text("extra"),
                    text("indexer_flags"),
                    var("_"),
                ]);
                let name = if value == internal {
                    "internal"
                } else {
                    "scene"
                };
                Compiled::ok(vec![compare("equal", flag, text(name))])
            } else {
                return Some(fail(
                    codes::INDEXER_FLAG_UNMAPPED,
                    format!("Indexer flag {shown} has no documented Scryer mapping."),
                ));
            }
        }
        "LanguageSpecification" => {
            let except = spec.field("exceptLanguage");
            if except.is_some_and(|except| !except.is_boolean()) {
                return Some(fail(
                    codes::LANGUAGE_INVALID,
                    "exceptLanguage must be a boolean.".to_string(),
                ));
            }
            let target = if value == -2.0 {
                reference(&["input", "context", "original_language"])
            } else {
                match LANGUAGES.iter().find(|(id, _)| *id as f64 == value) {
                    Some((_, code)) => text(*code),
                    None => {
                        return Some(fail(
                            codes::LANGUAGE_UNMAPPED,
                            format!("Language {shown} has no distinct Scryer language mapping."),
                        ));
                    }
                }
            };
            let language = || var("_arr_language");
            let matches = Term::ArrayComprehension(
                Box::new(language()),
                vec![
                    compare(
                        "assign",
                        language(),
                        ref_terms(vec![
                            var("input"),
                            text("release"),
                            text("languages_audio"),
                            var("_"),
                        ]),
                    ),
                    call("scryer.lang_matches", vec![language(), target])
                        .negated(except == Some(&Value::Bool(true))),
                ],
            );
            Compiled {
                handles_negation: true,
                ..Compiled::ok(vec![
                    call("is_array", vec![release("languages_audio")]),
                    compare(
                        if spec.negate { "equal" } else { "gt" },
                        call_term("count", vec![matches]),
                        number(0.0),
                    ),
                ])
            }
        }
        "YearSpecification" => {
            return Some(fail(
                codes::YEAR_UNMAPPED,
                "Radarr year conditions require a movie-metadata fallback that is unavailable in the rule input."
                    .to_string(),
            ));
        }
        "SizeSpecification" => {
            let (Some(min), Some(max)) = (numeric(spec.field("min")), numeric(spec.field("max")))
            else {
                return Some(fail(
                    codes::RANGE_INVALID,
                    "The condition needs a valid minimum and maximum range.".to_string(),
                ));
            };
            if min < 0.0 || max <= min {
                return Some(fail(
                    codes::RANGE_INVALID,
                    "The condition needs a valid minimum and maximum range.".to_string(),
                ));
            }
            let (lower, upper) = (gib_bytes(min), gib_bytes(max));
            if !is_safe_integer(lower) || !is_safe_integer(upper) {
                return Some(fail(
                    codes::RANGE_UNSAFE,
                    "Range exceeds the exact numeric precision supported by this translator."
                        .to_string(),
                ));
            }
            let input = || release("size_bytes");
            Compiled::ok(vec![
                call("is_number", vec![input()]),
                compare("gt", input(), number(lower)),
                compare("lte", input(), number(upper)),
            ])
        }
        _ => {
            return Some(fail(
                codes::IMPLEMENTATION_UNSUPPORTED,
                format!("Unsupported Arr implementation {implementation}."),
            ));
        }
    };
    Some(compiled)
}
