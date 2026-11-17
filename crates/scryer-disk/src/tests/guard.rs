//! A source guard: outside the exempt code, a line that reaches the
//! filesystem directly must sit under a `disk-guard:` comment saying why.
//! Clippy's disallowed-methods list is the full rule; this reads the sources
//! as text so the rule also holds where clippy has not run.
//!
//! It sees the `std::fs` and `tokio::fs` modules however they are imported,
//! the platform extension traits, `fs_err`, the `File` and `OpenOptions`
//! constructors, and the path methods `try_exists`, `canonicalize`,
//! `read_link`, `symlink_metadata` and `read_dir`. Four entries of the clippy
//! list stay clippy-only: the path methods `exists`, `is_file`, `is_dir` and
//! `metadata`. The service's reading type has methods of the same names, and
//! text cannot tell the two receivers apart.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The disk service itself, and the crates clippy.toml names as not depending
/// on `scryer-application`.
const EXEMPT_CRATES: [&str; 4] = [
    "scryer-disk",
    "scryer-launcher",
    "scryer-mediainfo",
    "scryer-mock-apis",
];
/// Directories that hold only test, bench or example code.
const EXEMPT_DIRS: [&str; 5] = ["tests", "lib_tests", "benches", "examples", "target"];
/// `disk-guard:` comments on production code in the guarded sources today. A
/// marker on a test-gated item or allowance is not counted. Lower it when one
/// is removed; it never goes up.
const MARKERS: usize = 80;
/// Files holding direct filesystem lines that carry no marker yet, with how
/// many. An entry may shrink or go; none is added and none grows.
const UNMARKED: [(&str, usize); 11] = [
    ("scryer-application/src/file_source_signature.rs", 2),
    ("scryer-application/src/folder_ownership/scan_cache.rs", 1),
    ("scryer-application/src/import/archive_extractor.rs", 1),
    ("scryer-application/src/import/workflow/results.rs", 1),
    ("scryer-application/src/library/discovery.rs", 1),
    (
        "scryer-application/src/library/leftover_title_folders.rs",
        2,
    ),
    ("scryer-application/src/location/execution.rs", 2),
    ("scryer-application/src/subtitles/external_probe.rs", 1),
    (
        "scryer-infrastructure-datastore/src/migrations/assets.rs",
        1,
    ),
    (
        "scryer-infrastructure-library/src/media/libraries/scanner.rs",
        1,
    ),
    ("scryer/src/initial_auth.rs", 1),
];

fn exempt_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    !name.ends_with(".rs")
        || name == "build.rs"
        || name == "tests.rs"
        || name.ends_with("_tests.rs")
}

fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    // Another session may remove a directory or an entry mid-walk.
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries = listing
        .filter_map(|entry| Some(entry.ok()?.path()))
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_dir() {
            if !EXEMPT_DIRS.contains(&name) {
                sources(&path, found);
            }
        } else if !exempt_file(&path) {
            found.push(path);
        }
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn annotation(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("//") || line.starts_with("#[") || line.starts_with("#![")
}

/// An attribute that compiles its item for tests only. A comment naming one
/// is not one, and `cfg(any(test, ..))` also builds for production.
fn test_only(line: &str) -> bool {
    let line = line.trim_start();
    let gated = || {
        ["cfg(test", "cfg(all(test"]
            .iter()
            .any(|gate| line.contains(gate))
    };
    line == "#[test]" || line.starts_with("#[tokio::test") || line.starts_with("#[") && gated()
}

/// An attribute that is test-only, or allows something for tests only.
fn for_tests(line: &str) -> bool {
    let line = line.trim_start();
    test_only(line) || line.starts_with("#[cfg_attr(test,") || line.starts_with("#![cfg_attr(test,")
}

/// The line carries on a construct opened further up.
fn continues(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with([')', '}', ']', '{', '.']) || line.starts_with("where")
}

/// Whether line `at`, or anything enclosing it, has a marker or a test gate
/// in the comments and attributes directly above it.
fn covered(lines: &[&str], at: usize) -> bool {
    under(lines, at, |line| {
        line.contains("disk-guard:") || test_only(line)
    })
}

/// Whether the marker on line `at` stands on test-only code: the item it
/// annotates, or anything enclosing that item, is gated or allowed for tests.
fn test_marker(lines: &[&str], at: usize) -> bool {
    let item = (at..lines.len()).find(|at| !annotation(lines[*at]));
    under(lines, item.unwrap_or(at), for_tests)
}

/// Whether line `at`, or anything enclosing it, has a line `wanted` accepts
/// in the comments and attributes directly above it.
fn under(lines: &[&str], at: usize, wanted: impl Fn(&str) -> bool) -> bool {
    let (mut at, mut depth) = (at, indent(lines[at]));
    loop {
        let above = lines[..at].iter().rev();
        if above
            .take_while(|line| !line.trim().is_empty() && annotation(line))
            .any(|line| wanted(line))
        {
            return true;
        }
        if depth == 0 {
            return false;
        }
        let encloses = |line: &&str, depth: usize| !line.trim().is_empty() && indent(line) < depth;
        let Some(outer) = lines[..at].iter().rposition(|line| encloses(line, depth)) else {
            return false;
        };
        (at, depth) = (outer, indent(lines[outer]));
        while continues(lines[at]) {
            let opens = |line: &&str| !line.trim().is_empty() && indent(line) <= depth;
            let Some(opener) = lines[..at].iter().rposition(opens) else {
                break;
            };
            (at, depth) = (opener, indent(lines[opener]));
        }
    }
}

/// A line that names a filesystem call, or brings the means to make one into
/// scope. Types alone (`std::fs::File`, `std::fs::Metadata`) do not count.
fn direct(line: &str) -> bool {
    if line.trim_start().starts_with("//") {
        return false;
    }
    let calls = |module: &str| {
        line.match_indices(module).any(|(found, _)| {
            let rest = &line[found + module.len()..];
            rest.trim_start_matches('{').starts_with(lowercase)
        })
    };
    let imports = |module: &str| {
        line.contains(&format!("use {module};")) || line.contains(&format!("use {module} as "))
    };
    calls("std::fs::")
        || calls("tokio::fs::")
        || imports("std::fs")
        || imports("tokio::fs")
        || [
            "fs_err::",
            "std::os::unix::fs::",
            "std::os::windows::fs::",
            "File::open(",
            "File::create(",
            "File::create_new(",
            "File::options(",
            "OpenOptions::new(",
            ".try_exists()",
            ".canonicalize()",
            ".read_link()",
            ".symlink_metadata()",
            ".read_dir()",
        ]
        .iter()
        .any(|token| line.contains(token))
}

fn lowercase(next: char) -> bool {
    next.is_ascii_lowercase()
}

/// Whether `group`, text from inside the braces of a `use std::{` or
/// `use tokio::{`, brings the `fs` module or a function of it into scope.
fn groups_fs(group: &str) -> bool {
    group.match_indices("fs").any(|(found, _)| {
        let rest = &group[found + 2..];
        let starts = group[..found]
            .chars()
            .next_back()
            .is_none_or(|before| "{, ".contains(before));
        let module =
            rest.is_empty() || rest.starts_with([',', '}', ';']) || rest.starts_with(" as ");
        let function = rest
            .strip_prefix("::")
            .is_some_and(|rest| rest.trim_start_matches('{').starts_with(lowercase));
        starts && (module || function)
    })
}

/// A call through a module imported as plain `fs`: `fs::read(..)`.
fn bare_call(line: &str) -> bool {
    !line.trim_start().starts_with("//")
        && line.match_indices("fs::").any(|(found, _)| {
            let before = line[..found].chars().next_back();
            let named = |before: char| before == ':' || before == '_' || before.is_alphanumeric();
            !before.is_some_and(named) && line[found + 4..].starts_with(lowercase)
        })
}

/// For each line, whether it reaches the filesystem directly: [`direct`], a
/// grouped import of the `fs` module, or a bare `fs::` call in a file that
/// imports that module.
fn raw(lines: &[&str]) -> Vec<bool> {
    let mut grouped = false;
    let mut found = Vec::with_capacity(lines.len());
    for line in lines {
        let text = line.trim();
        let opens = ["use std::{", "use tokio::{"]
            .iter()
            .any(|open| text.contains(open));
        let group = match text.split_once("::{") {
            Some((_, group)) if opens => Some(group),
            _ => grouped.then_some(text),
        };
        grouped = (opens || grouped) && !text.contains(';');
        found.push(direct(line) || group.is_some_and(groups_fs));
    }
    if (0..lines.len()).any(|at| found[at] && lines[at].trim_start().starts_with("use ")) {
        for (at, line) in lines.iter().enumerate() {
            found[at] = found[at] || bare_call(line);
        }
    }
    found
}

/// The files a `#[cfg(test)] mod name;` in `source` declares.
fn test_modules(path: &Path, lines: &[&str], exempt: &mut HashSet<PathBuf>) {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    let parent = path.parent().expect("a source file has a directory");
    let home = match stem {
        "mod" | "lib" | "main" => parent.to_path_buf(),
        stem => parent.join(stem),
    };
    for (at, line) in lines.iter().enumerate() {
        let declared = line.trim().strip_suffix(';').and_then(|line| {
            line.rsplit_once("mod ")
                .filter(|(before, _)| before.is_empty() || before.starts_with("pub"))
        });
        let gated = lines[..at]
            .iter()
            .rev()
            .take_while(|line| annotation(line))
            .any(|line| test_only(line));
        if let (Some((_, name)), true) = (declared, gated) {
            exempt.insert(home.join(format!("{name}.rs")));
            exempt.insert(home.join(name));
        }
    }
}

#[test]
fn direct_filesystem_lines_carry_a_marker_and_markers_do_not_grow() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the disk crate lives below the crates directory");
    let mut files = Vec::new();
    let mut roots = std::fs::read_dir(crates)
        .expect("crates directory should be readable")
        .map(|entry| entry.expect("crate entry should be readable").path())
        .collect::<Vec<_>>();
    roots.sort();
    for root in roots {
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if root.is_dir() && !EXEMPT_CRATES.contains(&name) {
            sources(&root, &mut files);
        }
    }
    // A file that vanished, or a link to nothing, is not a source.
    let read = |path: PathBuf| Some((std::fs::read_to_string(&path).ok()?, path));
    let (texts, files): (Vec<_>, Vec<_>) = files.into_iter().filter_map(read).unzip();
    let mut test_files = HashSet::new();
    for (path, text) in files.iter().zip(&texts) {
        test_modules(path, &text.lines().collect::<Vec<_>>(), &mut test_files);
    }

    let (mut markers, mut unmarked) = (0, BTreeMap::<String, Vec<usize>>::new());
    for (path, text) in files.iter().zip(&texts) {
        if path.ancestors().any(|at| test_files.contains(at)) {
            continue;
        }
        let lines = text.lines().collect::<Vec<_>>();
        let marked = |at: &usize| lines[*at].contains("disk-guard:") && !test_marker(&lines, *at);
        markers += (0..lines.len()).filter(marked).count();
        let shown = path.strip_prefix(crates).unwrap_or(path);
        let shown = shown.to_string_lossy().replace('\\', "/");
        let raw = raw(&lines);
        for at in 0..lines.len() {
            if raw[at] && !covered(&lines, at) {
                unmarked.entry(shown.clone()).or_default().push(at + 1);
            }
        }
    }

    let mut offending = Vec::new();
    for (file, found) in &unmarked {
        let allowed = UNMARKED
            .iter()
            .find(|(known, _)| known == file)
            .map_or(0, |(_, allowed)| *allowed);
        if found.len() > allowed {
            offending.extend(found.iter().map(|line| format!("crates/{file}:{line}")));
        }
    }
    assert!(
        offending.is_empty(),
        "these lines reach the filesystem directly with no `disk-guard:` comment above them or \
         their item; use the disk service instead:\n{}",
        offending.join("\n")
    );
    for (file, allowed) in UNMARKED {
        let found = unmarked.get(file).map_or(0, Vec::len);
        assert_eq!(found, allowed, "lower the UNMARKED entry for {file}");
    }
    assert!(
        markers <= MARKERS,
        "{markers} `disk-guard:` comments, up from {MARKERS}: new code goes through the disk service"
    );
    assert_eq!(
        markers, MARKERS,
        "a marker was removed: lower MARKERS to {markers}"
    );
}

#[test]
fn the_guard_tells_marked_gated_and_bare_lines_apart() {
    let source = [
        "// disk-guard: an invented reason.",
        "#[allow(clippy::disallowed_methods)]",
        "fn marked(path: &Path) {",
        "    let _ = std::fs::read(path);",
        "}",
        "",
        "fn bare(",
        "    path: &Path,",
        ") -> std::fs::Metadata",
        "{",
        "    use std::os::unix::fs::MetadataExt;",
        "    tokio::fs::metadata(path)",
        "}",
        "",
        "#[cfg(test)]",
        "mod tests {",
        "    fn fixture() {",
        "        std::fs::write(\"invented\", b\"\").unwrap();",
        "    }",
        "}",
    ];
    let unmarked = (0..source.len())
        .filter(|at| direct(source[*at]) && !covered(&source, *at))
        .collect::<Vec<_>>();
    assert_eq!(unmarked, [10, 11]);
    assert!(direct("use std::fs as files;") && direct("    File::open(path)"));
    assert!(!direct("    // std::fs::read is not for callers") && !direct("file: std::fs::File,"));
    assert!(direct("    path.try_exists()") && direct("    File::options().read(true)"));
    assert!(direct("use std::fs::{read, File};") && !direct("use std::fs::{File, OpenOptions};"));
}

#[test]
fn the_guard_sees_a_grouped_import_and_the_bare_calls_it_allows() {
    let source = [
        "use std::{path::Path, sync::Arc};",
        "use tokio::{fs, io::AsyncReadExt};",
        "use std::{",
        "    collections::HashMap,",
        "    fs::{self, File},",
        "};",
        "fn bare(path: &Path) {",
        "    let _ = fs::read(path);",
        "    let _: fs::File = offs::read(path);",
        "}",
    ];
    assert_eq!(
        raw(&source),
        [
            false, true, false, false, true, false, false, true, false, false
        ]
    );
    // Without an import of the module, `fs::` names something else.
    assert_eq!(raw(&source[6..]), [false; 4]);
}

#[test]
fn only_a_test_gate_on_an_attribute_exempts_and_test_markers_are_not_counted() {
    assert!(test_only("#[cfg(test)]") && test_only("    #[cfg(all(test, unix))]"));
    assert!(!test_only("// built under #[cfg(test)] only") && !test_only("#[cfg(not(test))]"));
    assert!(!test_only("#[cfg(any(test, feature = \"invented\"))]"));
    let source = [
        "// disk-guard: test code may call the filesystem directly.",
        "#![cfg_attr(test, allow(clippy::disallowed_methods))]",
        "",
        "// disk-guard: an invented production reason.",
        "#[allow(clippy::disallowed_methods)]",
        "fn marked() {}",
        "",
        "#[cfg(test)]",
        "// disk-guard: test code may call the filesystem directly.",
        "#[allow(clippy::disallowed_methods)]",
        "mod tests {",
        "    // disk-guard: an invented fixture reason.",
        "    fn fixture() {}",
        "}",
    ];
    let counted = (0..source.len())
        .filter(|at| source[*at].contains("disk-guard:") && !test_marker(&source, *at))
        .collect::<Vec<_>>();
    assert_eq!(counted, [3]);
}
