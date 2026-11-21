use super::{
    Arguments, ToolContext, ToolEntry, ToolFuture, admit_diagnostics, diagnostics_error,
    invalid_argument, local_read, omit_empty, tool,
};
use crate::diagnostics_routes::{HostSection, host_identifying, host_sections};
use crate::mcp::server::json_result;
use scryer_application::diagnostics::{
    DiagnosticsHarvest, DiagnosticsSection, EnvironmentSection, Omission, SummarySection,
};
use scryer_application::{AppError, AppResult};
use serde_json::{Map, Value, json};

/// The Prometheus exposition, filtered to named families.
const METRICS: &str = "metrics";
const MAX_METRIC_FAMILIES: usize = 10;
const MAX_METRIC_LINES: usize = 60;
const MAX_QUALITY_PROFILES: usize = 10;
const MAX_FILESYSTEM_ROWS: usize = 50;
/// Serialized characters a section may hold; a larger one is withheld.
const SECTION_BUDGET_CHARS: usize = 18_000;

/// What one `section` value names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    App(DiagnosticsSection),
    Host(HostSection),
    Metrics,
}

/// The host's `metrics` section only says whether metrics are on, which the
/// `metrics` section here also says; the name means the exposition here.
fn host_sections_listed() -> impl Iterator<Item = HostSection> {
    HostSection::ALL
        .iter()
        .copied()
        .filter(|section| *section != HostSection::Metrics)
}

fn section_names() -> Vec<&'static str> {
    SummarySection::ALL
        .iter()
        .map(|section| section.as_str())
        .chain(
            EnvironmentSection::ALL
                .iter()
                .map(|section| section.as_str()),
        )
        .chain(host_sections_listed().map(HostSection::as_str))
        .chain(std::iter::once(METRICS))
        .collect()
}

fn parse_section(name: &str) -> Option<Section> {
    if name == METRICS {
        return Some(Section::Metrics);
    }
    SummarySection::ALL
        .iter()
        .find(|section| section.as_str() == name)
        .map(|section| Section::App(DiagnosticsSection::Summary(*section)))
        .or_else(|| {
            EnvironmentSection::ALL
                .iter()
                .find(|section| section.as_str() == name)
                .map(|section| Section::App(DiagnosticsSection::Environment(*section)))
        })
        .or_else(|| {
            host_sections_listed()
                .find(|section| section.as_str() == name)
                .map(Section::Host)
        })
}

pub(super) fn entry() -> ToolEntry {
    ToolEntry {
        tool: tool(
            "read_diagnostics_section",
            "One sanitized diagnostics bundle section; metrics takes up to 10 metric_families. Needs the system settings permission.",
            json!({
                "type": "object",
                "properties": {
                    "section": {"enum": section_names()},
                    "metric_families": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_METRIC_FAMILIES},
                },
                "required": ["section"],
            }),
            local_read(),
        ),
        accepts: &["section", "metric_families"],
        run,
        class: super::api_class,
    }
}

fn run(context: ToolContext, arguments: Arguments) -> ToolFuture {
    Box::pin(async move {
        let name = arguments.required_str("section", 100)?.to_string();
        let Some(section) = parse_section(&name) else {
            return Err(invalid_argument(
                "section",
                "must be one of the listed values",
            ));
        };
        let families = arguments.str_list("metric_families", MAX_METRIC_FAMILIES, 200)?;
        match (section, &families) {
            (Section::Metrics, None) => {
                return Err(invalid_argument(
                    "metric_families",
                    "is required for the metrics section",
                ));
            }
            (Section::App(_) | Section::Host(_), Some(_)) => {
                return Err(invalid_argument(
                    "metric_families",
                    "is only accepted for the metrics section",
                ));
            }
            _ => {}
        }
        if let Some(refused) = admit_diagnostics(&context).await {
            return Ok(refused);
        }

        let harvest = match harvest(&context).await {
            Ok(harvest) => harvest,
            Err(error) => return Ok(diagnostics_error(error)),
        };

        let mut result = Map::new();
        result.insert("section".into(), json!(name));
        match section {
            Section::Metrics => {
                metrics(
                    &context,
                    &harvest,
                    families.unwrap_or_default(),
                    &mut result,
                )
                .await?
            }
            Section::App(section) => {
                let read = match context
                    .auth
                    .app
                    .diagnostics_section(context.caller.user(), section)
                    .await
                {
                    Ok(read) => read,
                    Err(error) => return Ok(diagnostics_error(error)),
                };
                let mut value = read.value.unwrap_or(Value::Null);
                harvest.sanitizer.json_values(&mut value);
                let value = cap(section, value);
                insert_value(&mut result, value);
                result.insert("omissions".into(), omissions(&read.omissions));
            }
            Section::Host(section) => {
                let host = context.diagnostics.clone();
                let auth = context.auth.auth_runtime.snapshot();
                let Ok(facts) =
                    tokio::task::spawn_blocking(move || host_sections(&host, &auth, &[section]))
                        .await
                else {
                    return Ok(diagnostics_error(AppError::Repository(
                        "diagnostics host section task failed".into(),
                    )));
                };
                let mut value = facts
                    .environment
                    .get(section.as_str())
                    .cloned()
                    .unwrap_or(Value::Null);
                harvest.sanitizer.json_values(&mut value);
                insert_value(&mut result, value);
                result.insert(
                    "omissions".into(),
                    Value::Array(
                        facts
                            .omissions
                            .iter()
                            .map(|(item, reason)| {
                                json!({"item": format!("environment.{item}"), "reason": reason})
                            })
                            .collect(),
                    ),
                );
            }
        }
        Ok(json_result(Value::Object(omit_empty(result))))
    })
}

/// One harvest for this call, with the same inputs as the bundle's.
async fn harvest(context: &ToolContext) -> AppResult<DiagnosticsHarvest> {
    let host = context.diagnostics.clone();
    let Ok((values, paths)) = tokio::task::spawn_blocking(move || host_identifying(&host)).await
    else {
        return Err(AppError::Repository(
            "diagnostics identifying values task failed".into(),
        ));
    };
    context
        .auth
        .app
        .diagnostics_harvest(context.caller.user(), &values, &paths)
        .await
}

fn insert_value(result: &mut Map<String, Value>, value: Value) {
    let chars = serde_json::to_string(&value).map_or(0, |text| text.chars().count());
    if chars > SECTION_BUDGET_CHARS {
        result.insert("value".into(), Value::Null);
        result.insert("withheld_for_size_chars".into(), json!(chars));
    } else {
        result.insert("value".into(), value);
    }
}

fn omissions(omissions: &[Omission]) -> Value {
    Value::Array(
        omissions
            .iter()
            .map(|omission| json!({"item": omission.item, "reason": omission.reason}))
            .collect(),
    )
}

/// The per-section caps and projections.
fn cap(section: DiagnosticsSection, mut value: Value) -> Value {
    match section {
        DiagnosticsSection::Summary(SummarySection::QualityProfiles) => {
            cap_array(&mut value, "profiles", MAX_QUALITY_PROFILES);
            value
        }
        DiagnosticsSection::Environment(EnvironmentSection::Filesystems) => {
            cap_array(&mut value, "roots", MAX_FILESYSTEM_ROWS);
            cap_array(&mut value, "same_filesystem_pairs", MAX_FILESYSTEM_ROWS);
            value
        }
        DiagnosticsSection::Summary(SummarySection::JobRuns) => {
            // The latest status, start and trigger per job only.
            let by_job = value
                .get("by_job")
                .and_then(Value::as_object)
                .map(|jobs| {
                    jobs.iter()
                        .map(|(key, job)| {
                            (
                                key.clone(),
                                json!({
                                    "latest_status": job.get("latest_status"),
                                    "latest_started_at": job.get("latest_started_at"),
                                    "latest_trigger": job.get("latest_trigger"),
                                }),
                            )
                        })
                        .collect::<Map<_, _>>()
                })
                .unwrap_or_default();
            json!({"sampled": value.get("sampled"), "by_job": by_job})
        }
        _ => value,
    }
}

/// Cuts `value[key]` to `max` items and records how many were dropped in
/// `truncated.<key>`.
fn cap_array(value: &mut Value, key: &str, max: usize) {
    let Some(items) = value.get_mut(key).and_then(Value::as_array_mut) else {
        return;
    };
    if items.len() <= max {
        return;
    }
    let dropped = items.len() - max;
    items.truncate(max);
    if let Some(object) = value.as_object_mut() {
        let truncated = object
            .entry("truncated")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(truncated) = truncated.as_object_mut() {
            truncated.insert(key.to_string(), json!(dropped));
        }
    }
}

async fn metrics(
    context: &ToolContext,
    harvest: &DiagnosticsHarvest,
    families: Vec<String>,
    result: &mut Map<String, Value>,
) -> Result<(), rmcp::ErrorData> {
    let Some(handle) = context.diagnostics.metrics.clone() else {
        result.insert("enabled".into(), json!(false));
        return Ok(());
    };
    result.insert("enabled".into(), json!(true));
    // The bundle leaves metrics out while the harvest is incomplete, since
    // their labels could carry a value it missed.
    if let Some(reason) = harvest.incomplete_reason() {
        result.insert("metrics_unavailable".into(), json!(reason));
        return Ok(());
    }
    let Ok(exposition) = tokio::task::spawn_blocking(move || handle.render()).await else {
        result.insert("metrics_unavailable".into(), json!("render_failed"));
        return Ok(());
    };
    let kept = exposition
        .lines()
        .filter(|line| {
            metric_family(line)
                .is_some_and(|name| families.iter().any(|family| belongs(name, family)))
        })
        .collect::<Vec<_>>();
    let truncated = kept.len().saturating_sub(MAX_METRIC_LINES);
    let text = kept
        .into_iter()
        .take(MAX_METRIC_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    let sanitized = harvest.sanitizer.metrics(&text);
    let lines = sanitized
        .lines()
        .map(|line| scryer_application::diagnostics::cut_chars(line, 300).to_string())
        .collect::<Vec<_>>();
    result.insert("families".into(), json!(families));
    result.insert("returned".into(), json!(lines.len()));
    result.insert("lines".into(), json!(lines));
    if truncated > 0 {
        result.insert("truncated".into(), json!(truncated));
    }
    Ok(())
}

/// The metric name a line is about: the name in `# HELP` and `# TYPE`, or the
/// sample's name before its labels or value.
fn metric_family(line: &str) -> Option<&str> {
    let line = line.trim();
    if let Some(rest) = line
        .strip_prefix("# HELP ")
        .or_else(|| line.strip_prefix("# TYPE "))
    {
        return rest.split_whitespace().next();
    }
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    line.split(['{', ' ']).next()
}

/// Whether `name` is `family` or one of its histogram or summary samples.
fn belongs(name: &str, family: &str) -> bool {
    name == family
        || name.strip_prefix(family).is_some_and(|suffix| {
            matches!(
                suffix,
                "_bucket" | "_sum" | "_count" | "_created" | "_total"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_names_are_distinct_and_all_parse() {
        let names = section_names();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
        for name in names {
            assert!(parse_section(name).is_some(), "{name}");
        }
        assert_eq!(parse_section("metrics"), Some(Section::Metrics));
    }

    #[test]
    fn metric_lines_are_matched_by_family() {
        assert_eq!(
            metric_family("# HELP scryer_synthetic_total Things"),
            Some("scryer_synthetic_total")
        );
        assert_eq!(
            metric_family("scryer_synthetic_bucket{le=\"1\"} 3"),
            Some("scryer_synthetic_bucket")
        );
        assert_eq!(
            metric_family("scryer_synthetic 3"),
            Some("scryer_synthetic")
        );
        assert!(belongs("scryer_synthetic_bucket", "scryer_synthetic"));
        assert!(!belongs("scryer_synthetic_other", "scryer_synthetic"));
    }

    #[test]
    fn caps_record_what_they_drop() {
        let mut value = json!({"roots": (0..60).collect::<Vec<_>>(), "same_filesystem_pairs": []});
        cap_array(&mut value, "roots", MAX_FILESYSTEM_ROWS);
        assert_eq!(value["roots"].as_array().unwrap().len(), 50);
        assert_eq!(value["truncated"]["roots"], 10);
        let job_runs = cap(
            DiagnosticsSection::Summary(SummarySection::JobRuns),
            json!({"sampled": 3, "by_job": {"health_checks": {
                "statuses": {"completed": 3},
                "latest_status": "completed",
                "latest_started_at": "2026-11-01T00:00:00Z",
                "latest_trigger": "scheduled",
                "latest_error": "synthetic",
            }}}),
        );
        assert_eq!(
            job_runs["by_job"]["health_checks"],
            json!({
                "latest_status": "completed",
                "latest_started_at": "2026-11-01T00:00:00Z",
                "latest_trigger": "scheduled",
            })
        );
    }
}
