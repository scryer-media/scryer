use scryer_application::arr_custom_format::{
    ArrDiagnosticSeverity, ArrFormatStatus, ArrSourceApplication, translate_arr_custom_formats,
};
use scryer_rules::{
    rewrite_package_declaration, runtime::RuntimeLimits, validation::validate_user_rule_with_limits,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize)]
struct Corpus {
    trash_revision: String,
    formats: Vec<TranslatedFormat>,
}

#[derive(Deserialize)]
struct TranslatedFormat {
    source: String,
    path: String,
    status: String,
    rego_source: String,
    #[serde(default)]
    diagnostics: Vec<FixtureDiagnostic>,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct FixtureDiagnostic {
    code: String,
    level: String,
    message: String,
    location: Option<String>,
    format_id: Option<String>,
    specification_index: Option<usize>,
}

#[derive(Deserialize)]
struct SourceCorpus {
    revision: String,
    formats: Vec<SourceFormat>,
}

#[derive(Deserialize)]
struct SourceFormat {
    source: String,
    path: String,
    format: serde_json::Value,
}

fn corpus_limits() -> RuntimeLimits {
    RuntimeLimits {
        max_execution_time: std::time::Duration::from_secs(60),
        ..RuntimeLimits::release_defaults()
    }
}

fn severity_name(severity: ArrDiagnosticSeverity) -> &'static str {
    match severity {
        ArrDiagnosticSeverity::Error => "error",
        ArrDiagnosticSeverity::Warning => "warning",
        ArrDiagnosticSeverity::Info => "info",
    }
}

/// The server translator reproduces the checked-in output of the web
/// translator it replaced: every status and diagnostic, and the Rego source
/// byte for byte. Any source that differs must still pass Regorus.
#[test]
fn server_translator_reproduces_the_checked_in_corpus() {
    let source: SourceCorpus = serde_json::from_str(include_str!(
        "../../../apps/scryer-web/lib/arr-custom-format/__fixtures__/trash-corpus.json"
    ))
    .expect("pinned corpus must be valid JSON");
    let expected: Corpus =
        serde_json::from_str(include_str!("fixtures/arr_custom_formats_rego.json"))
            .expect("checked-in translator output must be valid JSON");
    assert_eq!(source.revision, expected.trash_revision);
    assert_eq!(source.formats.len(), expected.formats.len());
    let limits = corpus_limits();
    let mut status_mismatches = Vec::new();
    let mut diagnostic_mismatches = Vec::new();
    let mut source_differences = Vec::new();
    let mut identical = 0;
    for (entry, expected) in source.formats.iter().zip(&expected.formats) {
        assert_eq!(entry.path, expected.path);
        assert_eq!(entry.source, expected.source);
        let application = match entry.source.as_str() {
            "sonarr" => ArrSourceApplication::Sonarr,
            _ => ArrSourceApplication::Radarr,
        };
        let json = serde_json::to_string(&entry.format).expect("corpus format serializes");
        let review = translate_arr_custom_formats(application, &json, &HashMap::new())
            .unwrap_or_else(|refusal| panic!("{}: {refusal:?}", entry.path));
        assert_eq!(review.formats.len(), 1, "{}", entry.path);
        let scores = HashMap::from([(review.formats[0].id.clone(), 100)]);
        let result = translate_arr_custom_formats(application, &json, &scores)
            .unwrap_or_else(|refusal| panic!("{}: {refusal:?}", entry.path));
        let format = &result.formats[0];
        let status = match format.status {
            ArrFormatStatus::Translated => "translated",
            ArrFormatStatus::Disabled => "disabled",
        };
        if status != expected.status {
            status_mismatches.push(format!("{}: {status} != {}", entry.path, expected.status));
        }
        let diagnostics: Vec<FixtureDiagnostic> = format
            .diagnostics
            .iter()
            .map(|item| FixtureDiagnostic {
                code: item.code.to_string(),
                level: severity_name(item.severity).to_string(),
                message: item.message.clone(),
                location: item.location.clone(),
                format_id: item.format_id.clone(),
                specification_index: item.specification_index,
            })
            .collect();
        if diagnostics != expected.diagnostics {
            diagnostic_mismatches.push(format!(
                "{}:\n  server {diagnostics:?}\n  web    {:?}",
                entry.path, expected.diagnostics
            ));
        }
        if result.rego_source == expected.rego_source {
            identical += 1;
            continue;
        }
        let rule_id = "arr_import_corpus";
        let rewritten = rewrite_package_declaration(&result.rego_source, rule_id);
        let validation = match validate_user_rule_with_limits(&rewritten, rule_id, &limits) {
            Ok(result) if result.valid => "valid".to_string(),
            Ok(result) => format!("REJECTED: {}", result.errors.join("; ")),
            Err(error) => format!("REJECTED: {error}"),
        };
        let first_difference = result
            .rego_source
            .lines()
            .zip(expected.rego_source.lines())
            .position(|(left, right)| left != right);
        source_differences.push(format!(
            "{} ({validation}), first differing line {first_difference:?}",
            entry.path
        ));
    }
    eprintln!(
        "byte-identical Rego: {identical}/{}; differing: {}",
        source.formats.len(),
        source_differences.len()
    );
    for difference in &source_differences {
        eprintln!("  {difference}");
    }
    assert!(
        status_mismatches.is_empty(),
        "status mismatches:\n{}",
        status_mismatches.join("\n")
    );
    assert!(
        diagnostic_mismatches.is_empty(),
        "diagnostic mismatches:\n{}",
        diagnostic_mismatches.join("\n")
    );
    assert!(
        source_differences
            .iter()
            .all(|line| !line.contains("REJECTED")),
        "Regorus rejected differing output:\n{}",
        source_differences.join("\n")
    );
    assert!(
        source_differences.is_empty(),
        "Rego differs from the checked-in corpus:\n{}",
        source_differences.join("\n")
    );
}

#[test]
fn translated_arr_corpus_passes_regorus_validation() {
    // The release-scoring budget is one second per evaluation in every build.
    // Several translated formats blow through that in an unoptimised test
    // binary (positional matchers re-scan the title per character), so the
    // corpus dry run widens the budget: this test guards translation
    // correctness, not interpreter speed.
    let limits = corpus_limits();
    let corpus: Corpus =
        serde_json::from_str(include_str!("fixtures/arr_custom_formats_rego.json"))
            .expect("checked-in translator output must be valid JSON");
    assert_eq!(
        corpus.trash_revision,
        "56cb176ef4b59d734d3643287e66aafd7c809bd4"
    );
    assert_eq!(
        corpus.formats.len(),
        478,
        "keep all source formats in the denominator"
    );
    let mut accepted = [0_usize; 2];
    let mut totals = [0_usize; 2];
    let mut disabled = 0;
    let mut failures = Vec::new();
    let mut paths = HashSet::new();
    for format in corpus.formats {
        eprintln!("Validating {}", format.path);
        assert!(
            paths.insert(format.path.clone()),
            "duplicate corpus path: {}",
            format.path
        );
        let source = match format.source.as_str() {
            "sonarr" => 0,
            "radarr" => 1,
            other => panic!("unexpected source application: {other}"),
        };
        totals[source] += 1;
        assert!(matches!(format.status.as_str(), "translated" | "disabled"));
        disabled += usize::from(format.status == "disabled");
        // Match the editor: restore the package, then compile and dry-run the
        // policy and its runtime wrapper with the registered Scryer builtins.
        let rule_id = "arr_import_corpus";
        let rewritten = rewrite_package_declaration(&format.rego_source, rule_id);
        match validate_user_rule_with_limits(&rewritten, rule_id, &limits) {
            Ok(result) if result.valid => {
                if format.status == "translated" {
                    accepted[source] += 1;
                }
            }
            Ok(result) => failures.push(format!("{}: {}", format.path, result.errors.join("; "))),
            Err(error) => failures.push(format!("{}: {error}", format.path)),
        }
    }
    assert_eq!(totals, [236, 242]);
    let count: usize = accepted.iter().sum();
    eprintln!(
        "Regorus-validated translations: Sonarr {}/{}, Radarr {}/{}, total {}/478 ({:.2}%); {} disabled, {} rejected",
        accepted[0],
        totals[0],
        accepted[1],
        totals[1],
        count,
        count as f64 * 100.0 / 478.0,
        disabled,
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "Regorus rejected generated output:\n{}",
        failures.join("\n\n")
    );
    assert!(
        count * 100 >= 478 * 97,
        "only {count}/478 complete formats passed Regorus"
    );
}
