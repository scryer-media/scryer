//! Offline checks for generated pack sources using the production evaluator.
//! Kept separate from typed-context parity until the host migration is integrated.

use crate::{PolicyOrigin, UserPolicy, UserRulesEngine, rewrite_package_declaration};
use serde_json::{Value, json};

fn pack_policies() -> Vec<UserPolicy> {
    let path = std::env::var_os("SCRYER_TRASH_PACK").expect("set SCRYER_TRASH_PACK");
    let pack: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    pack["rules"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            let id = format!("trash_validation_{index}");
            let source = rule
                .get("regoSource")
                .or_else(|| rule.get("rego_source"))
                .and_then(Value::as_str)
                .unwrap();
            UserPolicy {
                rego_source: rewrite_package_declaration(source, &id),
                id,
                name: rule["id"].as_str().unwrap().to_string(),
                origin: PolicyOrigin::User,
                applied_facets: vec![],
            }
        })
        .collect()
}

fn fixture(persona: &str, category: &str) -> Value {
    let mut release = json!({
            "raw_title": "Example.2024.1080p.WEB-DL.DDP5.1.H.264-Unknown",
            "normalized_tokens": ["EXAMPLE", "2024", "1080P", "WEB", "DL", "DDP5", "1", "H", "264", "UNKNOWN"],
            "quality": "1080P", "source": "WEB-DL", "video_codec": "H.264",
            "audio": "EAC3", "audio_codecs": ["EAC3"], "audio_channels": "5.1",
            "languages_audio": ["eng"], "languages_subtitles": [],
            "release_group": "Unknown", "has_release_group": true,
            "is_dual_audio": false, "is_atmos": false, "is_dolby_vision": false,
            "has_hdr_fallback": false,
            "detected_hdr": false, "is_hdr10plus": false, "is_hlg": false,
            "is_10bit": false, "is_uncensored": false, "is_dubs_only": false,
            "is_remux": false, "is_bd_disk": false, "is_proper_upload": false
    });
    release.as_object_mut().unwrap().extend(
        json!({
                "is_repack": false, "is_ai_enhanced": false, "is_hardcoded_subs": false,
                "is_password_protected": null, "is_obfuscated": false, "is_retagged": false,
                "streaming_service": null, "edition": null, "anime_version": null,
                "episode_release_type": null, "is_season_pack": false, "is_multi_episode": false,
                "year": 2024, "parse_confidence": 0.95, "size_bytes": null,
                "age_days": null, "thumbs_up": null, "thumbs_down": null, "extra": {}
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let profile = json!({
            "id": "test", "name": "Test", "scoring_persona": persona, "scoring_overrides": {},
            "quality_tiers": ["1080P", "2160P"], "archival_quality": null,
            "allow_unknown_quality": true, "source_allowlist": [], "source_blocklist": [],
            "video_codec_allowlist": [], "video_codec_blocklist": [],
            "audio_codec_allowlist": [], "audio_codec_blocklist": [],
            "atmos_preferred": false, "dolby_vision_allowed": true, "detected_hdr_allowed": true,
            "prefer_remux": false, "allow_bd_disk": false, "allow_upgrades": true,
            "prefer_dual_audio": false, "required_audio_languages": []
    });
    let context = json!({
            "category": category, "media_type": category, "title_id": "test-title",
            "library_name": "Test", "original_language": "en", "original_country": "US",
            "inferred_original_audio_language": "eng", "tags": [], "has_existing_file": false,
            "existing_score": null, "search_mode": "canonical", "runtime_minutes": 120,
            "is_anime": category == "anime", "is_filler": false,
            "coverage_total_runtime_minutes": 120, "coverage_member_runtime_minutes": 120,
            "coverage_member_count": 1
    });
    json!({
        "release": release, "profile": profile, "context": context,
        "builtin_score": {"total": 0, "blocked": false, "codes": []}, "file": null
    })
}

#[test]
#[ignore = "explicit generated-pack validation; requires SCRYER_TRASH_PACK"]
fn trash_pack_sources_evaluate_on_production_runtime() {
    let policies = pack_policies();
    assert!(!policies.is_empty());
    for policy in &policies {
        let validation =
            crate::validation::validate_user_rule(&policy.rego_source, &policy.id).unwrap();
        assert!(validation.valid, "{}: {:?}", policy.name, validation.errors);
    }
    let engine = UserRulesEngine::build(&policies).expect("generated pack must build");
    let mut evaluator = engine.evaluator();
    for persona in ["balanced", "audiophile", "efficient", "compatible"] {
        for category in ["movie", "series", "anime"] {
            for size in [None, Some(1_i64 << 30), Some(30_i64 << 30)] {
                let mut input = fixture(persona, category);
                input["release"]["size_bytes"] = json!(size);
                evaluator.engine.set_input(input.into());
                for policy in &policies {
                    let value = evaluator
                        .engine
                        .eval_rule(crate::score_entry_wrapper_rule_path(&policy.id))
                        .unwrap_or_else(|error| {
                            panic!(
                                "{} {persona}/{category} size={size:?}: {error}",
                                policy.name
                            )
                        });
                    if value == regorus::Value::Undefined {
                        continue;
                    }
                    let object = value.as_object().unwrap_or_else(|_| {
                        panic!("{} output must be an object: {value:?}", policy.name)
                    });
                    for (key, delta) in object.iter() {
                        assert!(
                            key.as_string().is_ok(),
                            "{} non-string score code",
                            policy.name
                        );
                        assert!(
                            delta.as_i64().is_ok(),
                            "{} non-integer score: {delta:?}",
                            policy.name
                        );
                    }
                }
            }
        }
    }
}

#[cfg(unix)]
fn resident_bytes() -> usize {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("read local test process RSS");
    assert!(output.status.success());
    std::str::from_utf8(&output.stdout)
        .unwrap()
        .trim()
        .parse::<usize>()
        .unwrap()
        * 1024
}

#[test]
#[cfg(unix)]
#[ignore = "explicit local resource benchmark; requires SCRYER_TRASH_PACK"]
fn benchmark_trash_pack_resources() {
    let baseline_only = std::env::var_os("SCRYER_TRASH_BENCH_BASELINE").is_some();
    let locales = std::env::var("SCRYER_TRASH_BENCH_LOCALES").unwrap_or_default();
    let mut policies = if baseline_only {
        vec![]
    } else {
        pack_policies()
    };
    policies.retain(|policy| {
        let locale = ["french-vf", "french-vo", "french-vostfr", "german", "asian"]
            .into_iter()
            .find(|name| policy.name == format!("trash-guides-{name}"));
        locale.is_none_or(|name| locales.split(',').any(|selected| selected == name))
    });
    if let Some(path) = std::env::var_os("SCRYER_SEADEX_PACK") {
        let pack: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for (index, rule) in pack["rules"].as_array().unwrap().iter().enumerate() {
            let id = format!("seadex_benchmark_{index}");
            let source = rule
                .get("regoSource")
                .or_else(|| rule.get("rego_source"))
                .and_then(Value::as_str)
                .unwrap();
            policies.push(UserPolicy {
                rego_source: rewrite_package_declaration(source, &id),
                id,
                name: "SeaDex".into(),
                origin: PolicyOrigin::User,
                applied_facets: vec![],
            });
        }
    }
    let source_bytes: usize = policies.iter().map(|policy| policy.rego_source.len()).sum();
    let started = std::time::Instant::now();
    let engine = UserRulesEngine::build(&policies).unwrap();
    let build_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut evaluator = engine.evaluator();
    let rule_paths = policies
        .iter()
        .map(|policy| crate::score_entry_wrapper_rule_path(&policy.id))
        .collect::<Vec<_>>();
    drop(policies);
    let mut input = fixture("balanced", "anime");
    input["release"]["size_bytes"] = json!(1_i64 << 30);
    let mut run = |index| {
        input["release"]["year"] = json!(2000 + index % 25);
        evaluator.engine.set_input(input.clone().into());
        for path in &rule_paths {
            std::hint::black_box(evaluator.engine.eval_rule(path.clone()).unwrap());
        }
    };
    let first = std::time::Instant::now();
    run(0);
    let first_ms = first.elapsed().as_secs_f64() * 1000.0;
    let warmed = std::time::Instant::now();
    for index in 0..100 {
        run(index);
    }
    let warm_ms = warmed.elapsed().as_secs_f64() * 10.0;
    println!(
        "baseline_only={baseline_only} locales={locales} source_bytes={source_bytes} build_ms={build_ms:.3} first_ms={first_ms:.3} warm_ms={warm_ms:.3} resident_bytes={}",
        resident_bytes()
    );
    drop(evaluator);
    drop(engine);
    println!("after_drop_resident_bytes={}", resident_bytes());
}

#[test]
#[cfg(unix)]
#[ignore = "explicit temporary-engine allocation check; requires SCRYER_TRASH_PACK"]
fn benchmark_trash_pack_rebuild_cycles() {
    let policies = pack_policies();
    let input = fixture("balanced", "anime");
    let mut retained = Vec::new();
    for revision in 0..12 {
        let mut revised_policies = policies.clone();
        for policy in &mut revised_policies {
            policy
                .rego_source
                .push_str(&format!("\nupdate_revision := {revision}\n"));
        }
        let engine = UserRulesEngine::build(&revised_policies).unwrap();
        let mut evaluator = engine.evaluator();
        evaluator.engine.set_input(input.clone().into());
        for policy in &policies {
            std::hint::black_box(
                evaluator
                    .engine
                    .eval_rule(crate::score_entry_wrapper_rule_path(&policy.id))
                    .unwrap(),
            );
        }
        drop(evaluator);
        drop(engine);
        drop(revised_policies);
        retained.push(resident_bytes());
    }
    println!("after_drop_resident_bytes_by_cycle={retained:?}");
    // Allocators can retain the first engines' pages. Continuing growth after
    // warm-up would indicate retained per-engine state, not an RSS baseline.
    assert!(
        retained[11] <= retained[5] + 2 * 1024 * 1024,
        "temporary engines retain increasing memory: {retained:?}"
    );
}
