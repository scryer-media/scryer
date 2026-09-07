use scryer_rules::{rewrite_package_declaration, validation::validate_user_rule};
use serde::Deserialize;
use std::collections::HashSet;

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
}

#[test]
fn translated_arr_corpus_passes_regorus_validation() {
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
        match validate_user_rule(&rewritten, rule_id) {
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
