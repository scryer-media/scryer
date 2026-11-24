// disk-guard: test, example and build-script code may call the filesystem directly.
#![allow(clippy::disallowed_methods)]

//! Every warning code is emitted through its constant, and every constant is
//! emitted somewhere, so the published list and the parser cannot drift.

use std::path::{Path, PathBuf};

use scryer_mediainfo::warning;

fn sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

/// The parser's own source, without test modules and test-only files.
fn parser_source() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files.sort();
    files
        .into_iter()
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name != "tests.rs" && name != "fixtures.rs"
        })
        .map(|path| {
            let mut text = std::fs::read_to_string(&path).unwrap();
            if let Some(start) = text.find("#[cfg(test)]\nmod tests {") {
                text.truncate(start);
            }
            (path, text)
        })
        .collect()
}

#[test]
fn every_code_is_emitted_through_its_constant() {
    let source = parser_source();
    for code in warning::ALL {
        let constant = format!("warning::{}", code.to_ascii_uppercase());
        assert!(
            source.iter().any(|(_, text)| text.contains(&constant)),
            "{constant} is published but never emitted"
        );
        let literal = format!("\"{code}\"");
        for (path, text) in &source {
            assert!(
                !text.contains(&literal),
                "{} spells {literal} instead of using {constant}",
                path.display()
            );
        }
    }
}

#[test]
fn no_warning_code_is_spelled_inline() {
    for (path, text) in parser_source() {
        assert!(
            !text.contains("code: \""),
            "{} builds a warning from an inline code",
            path.display()
        );
    }
}
