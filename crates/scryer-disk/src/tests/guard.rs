//! A source guard: outside the exempt code, a line that reaches the
//! filesystem directly must sit under a `disk-guard:` comment saying why.
//! Clippy's disallowed-methods list is the full rule; this reads the sources
//! as text so the rule also holds where clippy has not run.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Crates that do not depend on the disk service.
const EXEMPT_CRATES: [&str; 4] = [
    "scryer-disk",
    "scryer-launcher",
    "scryer-mediainfo",
    "scryer-mock-apis",
];
/// Directories that hold only test, bench or example code.
const EXEMPT_DIRS: [&str; 5] = ["tests", "lib_tests", "benches", "examples", "target"];
/// `disk-guard:` comments in the guarded sources today. Lower it when one is
/// removed; it never goes up.
const MARKERS: usize = 100;
/// Files holding direct filesystem lines that carry no marker yet, with how
/// many. An entry may shrink or go; none is added and none grows.
const UNMARKED: [(&str, usize); 9] = [
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
        || path.ends_with("migrations/assets.rs")
}

fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(dir)
        .expect("source directory should be readable")
        .map(|entry| entry.expect("source entry should be readable").path())
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

fn test_only(line: &str) -> bool {
    let line = line.trim_start();
    ["cfg(test", "cfg(all(test", "cfg(any(test"]
        .iter()
        .any(|gate| line.contains(gate))
        || line == "#[test]"
        || line.starts_with("#[tokio::test")
}

/// The line carries on a construct opened further up.
fn continues(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with([')', '}', ']', '{', '.']) || line.starts_with("where")
}

/// Whether line `at`, or anything enclosing it, has a marker or a test gate
/// in the comments and attributes directly above it.
fn covered(lines: &[&str], at: usize) -> bool {
    let (mut at, mut depth) = (at, indent(lines[at]));
    loop {
        let above = lines[..at].iter().rev();
        if above
            .take_while(|line| !line.trim().is_empty() && annotation(line))
            .any(|line| line.contains("disk-guard:") || test_only(line))
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
            rest.starts_with(|next: char| next.is_ascii_lowercase()) || rest.starts_with("{self")
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
            "OpenOptions::new(",
        ]
        .iter()
        .any(|token| line.contains(token))
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
    let texts = files
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("Rust source should be readable"))
        .collect::<Vec<_>>();
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
        markers += lines
            .iter()
            .filter(|line| line.contains("disk-guard:"))
            .count();
        let shown = path.strip_prefix(crates).unwrap_or(path);
        let shown = shown.to_string_lossy().replace('\\', "/");
        for at in 0..lines.len() {
            if direct(lines[at]) && !covered(&lines, at) {
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
}
