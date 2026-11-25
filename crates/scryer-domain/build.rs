//! Publishes the ICU collation crate versions this build links against.
//!
//! `title_collation_data_version` fingerprints the collation data by hashing a
//! probe corpus, which is a sample: a data change that leaves every probe's
//! sort key byte-identical would slip past it. Folding the resolved crate
//! versions into that hash closes the gap from the other side — a bump moves
//! the fingerprint whether or not the probes notice — and a fingerprint change
//! is a projection rebuild, never a silent lookup miss.

// disk-guard: test, example and build-script code may call the filesystem directly.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};

const PACKAGES: &[&str] = &["icu_collator", "icu_collator_data"];

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo always sets CARGO_MANIFEST_DIR"),
    );

    let lock = workspace_lock(&manifest_dir);
    let versions = match lock
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
    {
        Some(contents) => resolved_versions(&contents),
        // A missing lock file is not a build failure: the probe corpus still
        // fingerprints the data, and this only narrows the sampling gap.
        None => "unresolved".to_string(),
    };

    if let Some(path) = lock {
        println!("cargo::rerun-if-changed={}", path.display());
    }
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-env=SCRYER_ICU_COLLATOR_VERSIONS={versions}");
}

fn workspace_lock(manifest_dir: &Path) -> Option<PathBuf> {
    manifest_dir
        .ancestors()
        .map(|directory| directory.join("Cargo.lock"))
        .find(|candidate| candidate.is_file())
}

/// `name = "x"` followed by `version = "y"` is the whole shape of a
/// `[[package]]` entry that matters here, so this reads the lock line by line
/// rather than pulling in a TOML parser for two strings.
fn resolved_versions(lock: &str) -> String {
    let mut found = Vec::new();
    let mut current: Option<&str> = None;
    for line in lock.lines() {
        let line = line.trim();
        if let Some(name) = line
            .strip_prefix("name = \"")
            .and_then(|rest| rest.strip_suffix('"'))
        {
            current = PACKAGES.contains(&name).then_some(name);
        } else if let Some(version) = line
            .strip_prefix("version = \"")
            .and_then(|rest| rest.strip_suffix('"'))
            && let Some(name) = current.take()
        {
            found.push(format!("{name}={version}"));
        }
    }
    found.sort();
    if found.is_empty() {
        "unresolved".to_string()
    } else {
        found.join(",")
    }
}
