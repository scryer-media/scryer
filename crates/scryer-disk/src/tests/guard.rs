//! A source guard: outside the exempt code, a line that reaches the
//! filesystem directly must sit under a `disk-guard:` comment saying why.
//! Clippy's disallowed-methods list is the full rule; this reads the sources
//! as text so the rule also holds where clippy has not run.
//!
//! It sees the `std::fs` and `tokio::fs` modules however they are imported,
//! under whatever name, the functions imported from them, by name or by `*`,
//! and the calls made through either, the platform extension traits, `fs_err`, the `File` and `OpenOptions`
//! constructors, and the path methods `try_exists`, `canonicalize`,
//! `read_link`, `symlink_metadata` and `read_dir`. Four entries of the clippy
//! list stay clippy-only: the path methods `exists`, `is_file`, `is_dir` and
//! `metadata`. The service's reading type has methods of the same names, and
//! text cannot tell the two receivers apart.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The disk service itself, and the three crates clippy.toml names as exempt.
const EXEMPT_CRATES: [&str; 4] = [
    "scryer-disk",
    "scryer-launcher",
    "scryer-mediainfo",
    "scryer-mock-apis",
];
/// Directories that hold only test, bench or example code.
const EXEMPT_DIRS: [&str; 5] = ["tests", "lib_tests", "benches", "examples", "target"];
/// `disk-guard:` comments on production code in the guarded sources today. A
/// marker on a test-gated item, or one whose own attribute allows the direct
/// calls for tests only, is not counted. Lower it when one is removed; it
/// never goes up.
const MARKERS: usize = 78;
/// Files holding direct filesystem lines that carry no marker yet, with how
/// many. An entry may shrink or go; none is added and none grows.
const UNMARKED: [(&str, usize); 10] = [
    ("scryer-application/src/file_source_signature.rs", 2),
    ("scryer-application/src/folder_ownership/scan_cache.rs", 1),
    ("scryer-application/src/import/archive_extractor.rs", 1),
    ("scryer-application/src/import/workflow/results.rs", 1),
    ("scryer-application/src/library/discovery.rs", 1),
    (
        "scryer-application/src/library/leftover_title_folders.rs",
        2,
    ),
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

/// `read`, or `None` when the thing read went away after it was listed:
/// another session may remove a directory or a file mid-walk. Anything else
/// that cannot be read fails the test rather than going unchecked.
fn unless_gone<T>(path: &Path, read: std::io::Result<T>) -> Option<T> {
    match read {
        Ok(read) => Some(read),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("{} should be readable: {error}", path.display()),
    }
}

fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let Some(listing) = unless_gone(dir, std::fs::read_dir(dir)) else {
        return;
    };
    let mut entries = listing
        .filter_map(|entry| Some(unless_gone(dir, entry)?.path()))
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

/// What a line of source starts inside.
#[derive(Clone, Copy)]
enum Within {
    Code,
    /// A string literal; a raw one closes with a quote and its `#`s and has
    /// no escapes.
    Text {
        raw: bool,
        hashes: usize,
    },
    /// Block comments, this many deep.
    Comment(usize),
}

/// Where the string `rest` starts inside closes: the offset just past it.
fn text_end(rest: &str, raw: bool, hashes: usize) -> Option<usize> {
    let close = format!("\"{}", "#".repeat(hashes));
    let escaped = |at: usize| {
        let before = &rest[..at];
        !raw && (before.len() - before.trim_end_matches('\\').len()) % 2 == 1
    };
    let mut from = 0;
    loop {
        match rest[from..].find(&close) {
            Some(at) if escaped(from + at) => from += at + 1,
            Some(at) => return Some(from + at + close.len()),
            None => return None,
        }
    }
}

/// The lines of `text` as code. What a string literal or a block comment
/// holds is not code: a line that starts inside one is emptied up to where
/// it closes, and code after that point is kept at the indentation of the
/// line the string or comment opened on, which is what encloses it.
fn code(text: &str) -> Vec<Cow<'_, str>> {
    let mut lines = Vec::new();
    let (mut within, mut opened_at) = (Within::Code, 0);
    for line in text.lines() {
        let inside = !matches!(within, Within::Code);
        if !inside {
            opened_at = indent(line);
        }
        let (mut rest, mut tail) = (line, None);
        loop {
            match within {
                Within::Code => {}
                Within::Text { raw, hashes } => {
                    let Some(end) = text_end(rest, raw, hashes) else {
                        break;
                    };
                    (within, rest) = (Within::Code, &rest[end..]);
                }
                Within::Comment(depth) => {
                    let (open, close) = (rest.find("/*"), rest.find("*/"));
                    let (deeper, at) = match (open, close) {
                        (Some(open), Some(close)) if open < close => (true, open),
                        (_, Some(close)) => (false, close),
                        (Some(open), None) => (true, open),
                        (None, None) => break,
                    };
                    within = match (deeper, depth) {
                        (true, depth) => Within::Comment(depth + 1),
                        (false, 1) => Within::Code,
                        (false, depth) => Within::Comment(depth - 1),
                    };
                    rest = &rest[at + 2..];
                    continue;
                }
            }
            if inside && tail.is_none() {
                tail = Some(rest);
            }
            let Some(at) = rest.find(['"', '\'', '/']) else {
                break;
            };
            let (before, opened) = (&rest[..at], &rest[at..]);
            if opened.starts_with("//") {
                break;
            }
            if let Some(comment) = opened.strip_prefix("/*") {
                (within, rest) = (Within::Comment(1), comment);
            } else if let Some(text) = opened.strip_prefix('"') {
                let hashes = before.len() - before.trim_end_matches('#').len();
                let raw = before[..before.len() - hashes].ends_with('r');
                let hashes = if raw { hashes } else { 0 };
                (within, rest) = (Within::Text { raw, hashes }, text);
            } else if opened.starts_with("'\\") {
                // An escaped character: past the escape, then its quote.
                let end = opened[3..].find('\'').map_or(opened.len(), |end| end + 4);
                rest = &opened[end..];
            } else if opened.starts_with('\'') && opened[1..].chars().nth(1) == Some('\'') {
                let quoted = opened[1..].chars().next().map_or(0, char::len_utf8);
                rest = &opened[quoted + 2..];
            } else {
                // A lifetime, or a division.
                rest = &opened[1..];
            }
        }
        lines.push(match tail.map(str::trim) {
            _ if !inside => Cow::Borrowed(line),
            Some(tail) if !tail.is_empty() => {
                Cow::Owned(format!("{}{tail}", " ".repeat(opened_at)))
            }
            _ => Cow::Borrowed(""),
        });
    }
    lines
}

/// The lines of [`code`], borrowed for the functions that read them.
fn borrowed<'a>(lines: &'a [Cow<'_, str>]) -> Vec<&'a str> {
    lines.iter().map(|line| line.as_ref()).collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn annotation(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("//") || line.starts_with("#[") || line.starts_with("#![")
}

/// An attribute that compiles its item for tests only: `cfg(test)` or
/// `cfg(all(test, ..))`. A comment naming one is not one, `cfg(any(test, ..))`
/// also builds for production, and a longer name such as `cfg(testing)` is
/// another gate.
fn test_only(line: &str) -> bool {
    let line = line.trim_start();
    let gated = || {
        ["cfg(test", "cfg(all(test"].iter().any(|gate| {
            line.match_indices(gate)
                .any(|(at, _)| line[at + gate.len()..].starts_with([')', ',']))
        })
    };
    let attribute = line.starts_with("#[") || line.starts_with("#![");
    line == "#[test]" || line.starts_with("#[tokio::test") || attribute && gated()
}

fn attribute_body(line: &str) -> &str {
    line.trim_start()
        .trim_start_matches("#!")
        .trim_start_matches('#')
}

/// An attribute that allows the direct calls for tests only. Any other
/// `cfg_attr(test, ..)` says nothing about what the item under it is for.
fn allows_for_tests(line: &str) -> bool {
    attribute_body(line).starts_with("[cfg_attr(test, allow(clippy::disallowed_")
}

/// An attribute that allows the direct calls in every build.
fn allows_always(line: &str) -> bool {
    attribute_body(line).starts_with("[allow(clippy::disallowed_")
}

/// The line carries on a construct opened further up.
fn continues(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with([')', '}', ']', '{', '.']) || line.starts_with("where")
}

/// Whether line `at`, or anything enclosing it, is gated for tests.
fn gated(lines: &[&str], at: usize) -> bool {
    under(lines, at, |line| test_only(lines[line]))
}

/// The lines holding a marker that stands on production code.
fn counted(lines: &[&str]) -> HashSet<usize> {
    let marked = |at: &usize| lines[*at].contains("disk-guard:") && !test_marker(lines, *at);
    (0..lines.len()).filter(marked).collect()
}

/// Whether line `at` is test code, or it or anything enclosing it has one of
/// the `counted` markers in the comments and attributes directly above it. A
/// marker that stands on test-only code covers no production line.
fn covered(lines: &[&str], at: usize, counted: &HashSet<usize>) -> bool {
    gated(lines, at) || under(lines, at, |line| counted.contains(&line))
}

/// Whether the marker on line `at` stands on test-only code: the attribute
/// it explains allows the direct calls for tests only and no attribute of
/// the same item allows them in every build, or the item it annotates, or
/// anything enclosing that item, is gated for tests.
fn test_marker(lines: &[&str], at: usize) -> bool {
    let item = (at..lines.len()).find(|at| !annotation(lines[*at]));
    let above = &lines[at..item.unwrap_or(lines.len())];
    let explained = above
        .iter()
        .find(|line| !line.trim_start().starts_with("//"));
    let for_tests = explained.is_some_and(|line| allows_for_tests(line))
        && !above.iter().any(|line| allows_always(line));
    for_tests || gated(lines, item.unwrap_or(at))
}

/// Whether line `at`, or anything enclosing it, has a line `wanted` accepts
/// in the comments and attributes directly above it.
fn under(lines: &[&str], at: usize, wanted: impl Fn(usize) -> bool) -> bool {
    let (mut at, mut depth) = (at, indent(lines[at]));
    loop {
        let annotates =
            |above: &usize| !lines[*above].trim().is_empty() && annotation(lines[*above]);
        if (0..at).rev().take_while(annotates).any(&wanted) {
            return true;
        }
        if depth == 0 {
            return inner(lines, 0).any(&wanted);
        }
        let encloses = |line: &&str, depth: usize| !line.trim().is_empty() && indent(line) < depth;
        let Some(outer) = lines[..at].iter().rposition(|line| encloses(line, depth)) else {
            return inner(lines, 0).any(&wanted);
        };
        if inner(lines, outer + 1).any(&wanted) {
            return true;
        }
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

/// The inner attributes that open the body starting at line `from`: `#![..]`
/// lines, and the comments among them, before its first item.
fn inner<'a>(lines: &'a [&'a str], from: usize) -> impl Iterator<Item = usize> + 'a {
    let heads = |at: &usize| {
        let line = lines[*at].trim_start();
        line.is_empty() || line.starts_with("//") || line.starts_with("#![")
    };
    let attribute = |at: &usize| lines[*at].trim_start().starts_with("#![");
    (from..lines.len()).take_while(heads).filter(attribute)
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

/// A call through `module`, a name the file gave an `fs` module:
/// `fs::read(..)`.
fn module_call(line: &str, module: &str) -> bool {
    let path = format!("{module}::");
    line.match_indices(&path).any(|(found, _)| {
        !line[..found].ends_with(named) && line[found + path.len()..].starts_with(lowercase)
    })
}

/// A call of `function`, a name the file imported from an `fs` module:
/// `remove_file(..)`. A method, a path to another function of that name and
/// a definition are not one.
fn function_call(line: &str, function: &str) -> bool {
    let call = format!("{function}(");
    line.match_indices(&call).any(|(found, _)| {
        let before = &line[..found];
        !before.ends_with(named) && !before.ends_with(['.', ':']) && !before.ends_with("fn ")
    })
}

fn named(before: char) -> bool {
    before == ':' || before == '_' || before.is_alphanumeric()
}

/// The functions of `std::fs` and `tokio::fs`, which a `*` import brings in.
const FS_FUNCTIONS: [&str; 23] = [
    "canonicalize",
    "copy",
    "create_dir",
    "create_dir_all",
    "exists",
    "hard_link",
    "metadata",
    "read",
    "read_dir",
    "read_link",
    "read_to_string",
    "remove_dir",
    "remove_dir_all",
    "remove_file",
    "rename",
    "set_permissions",
    "soft_link",
    "symlink",
    "symlink_dir",
    "symlink_file",
    "symlink_metadata",
    "try_exists",
    "write",
];

/// What a file's `use` statements let it call without naming `std::fs` or
/// `tokio::fs` in full.
#[derive(Debug, Default, PartialEq)]
struct Imported {
    /// Names the `fs` modules go by.
    modules: Vec<String>,
    /// Functions of those modules, by the names they go by.
    functions: Vec<String>,
}

impl Imported {
    fn any(&self) -> bool {
        !(self.modules.is_empty() && self.functions.is_empty())
    }

    fn calls(&self, line: &str) -> bool {
        !line.trim_start().starts_with("//")
            && (self.modules.iter().any(|name| module_call(line, name))
                || self.functions.iter().any(|name| function_call(line, name)))
    }

    /// Takes what one `use` tree brings in. `tree` is what follows `use `, up
    /// to its `;`.
    fn take(&mut self, prefix: &str, tree: &str) {
        let tree = tree.trim();
        if let Some((head, rest)) = tree.split_once('{') {
            let group = rest.rsplit_once('}').map_or(rest, |(group, _)| group);
            let (mut depth, mut from) = (0usize, 0);
            for (at, next) in group.char_indices() {
                match next {
                    '{' => depth += 1,
                    '}' => depth = depth.saturating_sub(1),
                    ',' if depth == 0 => {
                        self.take(&format!("{prefix}{head}"), &group[from..at]);
                        from = at + 1;
                    }
                    _ => {}
                }
            }
            return self.take(&format!("{prefix}{head}"), &group[from..]);
        }
        let path = format!("{prefix}{tree}");
        let (path, alias) = match path.split_once(" as ") {
            Some((path, alias)) => (path.trim(), Some(alias.trim())),
            None => (path.trim(), None),
        };
        let path = path.trim_start_matches("::");
        let path = path.strip_suffix("::self").unwrap_or(path);
        // Everything of the crate, its `fs` module included.
        if path == "std::*" || path == "tokio::*" {
            return self.modules.push("fs".to_owned());
        }
        for module in ["std::fs", "tokio::fs"] {
            let name = |own: &str| alias.unwrap_or(own).to_owned();
            match path.strip_prefix(module) {
                Some("") => self.modules.push(name("fs")),
                Some(rest) => match rest.strip_prefix("::") {
                    Some("*") => {
                        let functions = FS_FUNCTIONS.iter().map(|function| (*function).to_owned());
                        self.functions.extend(functions);
                    }
                    Some(function) if function.starts_with(lowercase) => {
                        self.functions.push(name(function));
                    }
                    _ => {}
                },
                None => {}
            }
        }
    }
}

/// Where a `use` statement starts on `line`: after any attributes written
/// on the same line and any visibility.
fn use_tree(line: &str) -> Option<&str> {
    let mut line = line.trim_start();
    while line.starts_with("#[") {
        let mut depth = 0usize;
        let closes = |(at, next): (usize, char)| {
            match next {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
            (depth == 0 && next == ']').then_some(at)
        };
        let end = line.char_indices().find_map(closes)?;
        line = line[end + 1..].trim_start();
    }
    let line = match line.strip_prefix("pub") {
        Some(rest) if rest.starts_with('(') => rest.split_once(')')?.1,
        Some(rest) => rest,
        None => line,
    };
    line.trim_start().strip_prefix("use ")
}

/// For each line, whether it reaches the filesystem directly: [`direct`], an
/// import of an `fs` module or of a function of one, in a group or under
/// another name, or a call through such an import.
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

    let mut imported = Imported::default();
    let mut at = 0;
    while at < lines.len() {
        let Some(tree) = use_tree(lines[at]) else {
            at += 1;
            continue;
        };
        let end = (at..lines.len())
            .find(|end| lines[*end].contains(';'))
            .unwrap_or(lines.len() - 1);
        let mut statement = tree.to_owned();
        for line in &lines[at + 1..=end] {
            statement.push_str(line.split("//").next().unwrap_or("").trim());
        }
        let statement = statement.split(';').next().unwrap_or("");
        let mut brought = Imported::default();
        brought.take("", statement);
        // An import no line of which reads as direct still is one.
        if brought.any() && !found[at..=end].contains(&true) {
            found[at] = true;
        }
        imported.modules.append(&mut brought.modules);
        imported.functions.append(&mut brought.functions);
        at = end + 1;
    }
    for (at, line) in lines.iter().enumerate() {
        found[at] = found[at] || imported.calls(line);
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

/// The guarded production sources: every file outside the exempt crates and
/// directories, read, with the files a test-gated `mod` declares left out.
/// Each comes with its path as shown under the crates directory.
fn guarded_sources(crates: &Path) -> Vec<(String, String)> {
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
    let read = |path: PathBuf| {
        let text = unless_gone(&path, std::fs::read_to_string(&path))?;
        Some((text, path))
    };
    let (texts, files): (Vec<_>, Vec<_>) = files.into_iter().filter_map(read).unzip();
    let mut test_files = HashSet::new();
    for (path, text) in files.iter().zip(&texts) {
        test_modules(path, &borrowed(&code(text)), &mut test_files);
    }
    files
        .into_iter()
        .zip(texts)
        .filter(|(path, _)| !path.ancestors().any(|at| test_files.contains(at)))
        .map(|(path, text)| {
            let shown = path.strip_prefix(crates).unwrap_or(&path);
            (shown.to_string_lossy().replace('\\', "/"), text)
        })
        .collect()
}

fn crates_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the disk crate lives below the crates directory")
}

#[test]
fn direct_filesystem_lines_carry_a_marker_and_markers_do_not_grow() {
    let (mut markers, mut unmarked) = (0, BTreeMap::<String, Vec<usize>>::new());
    for (shown, text) in guarded_sources(crates_dir()) {
        let lines = code(&text);
        let lines = borrowed(&lines);
        let counted = counted(&lines);
        markers += counted.len();
        for (at, is_raw) in raw(&lines).into_iter().enumerate() {
            if is_raw && !covered(&lines, at, &counted) {
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

/// The calls by which code outside the disk service compares two reads of a
/// file by hand: two marks, a mark's file number, or two stored version
/// strings. The disk service's verbs answer these questions; a caller that
/// asks one of them itself is a standalone guard.
const HAND_COMPARISONS: [&str; 7] = [
    "still_marks",
    "numbered_as",
    "is_untouched",
    "same_file",
    "file_version",
    "same_content_version",
    "same_inode_version",
];
/// Production files that still make hand comparisons, with how many calls.
/// An entry may shrink or go; none is added and none grows. A length
/// compared with `==` is not seen here: text cannot tell a file's length from
/// any other number.
const COMPARED_BY_HAND: [(&str, usize); 8] = [
    ("scryer-application/src/import/workflow/mod.rs", 1),
    ("scryer-application/src/library/recycle_bin.rs", 1),
    ("scryer-application/src/location/backfill.rs", 2),
    ("scryer-application/src/location/conflicts.rs", 36),
    ("scryer-application/src/location/execution.rs", 6),
    ("scryer-application/src/location/resolution.rs", 12),
    (
        "scryer-application/src/location/title_folder_recovery.rs",
        1,
    ),
    (
        "scryer-infrastructure-workflow/src/workflow/file_importer.rs",
        5,
    ),
];

/// How many of [`HAND_COMPARISONS`] `line` calls, as a method or a function;
/// a definition is not a call.
fn hand_comparisons(line: &str) -> usize {
    HAND_COMPARISONS
        .iter()
        .map(|name| {
            let call = format!("{name}(");
            line.match_indices(&call)
                .filter(|(found, _)| {
                    let before = &line[..*found];
                    let joined = before
                        .chars()
                        .next_back()
                        .is_some_and(|last| last == '_' || last.is_alphanumeric());
                    !joined && !before.ends_with("fn ")
                })
                .count()
        })
        .sum()
}

#[test]
fn hand_comparisons_of_file_reads_only_shrink() {
    let mut found = BTreeMap::<String, usize>::new();
    for (shown, text) in guarded_sources(crates_dir()) {
        let lines = code(&text);
        let lines = borrowed(&lines);
        let calls: usize = (0..lines.len())
            .map(|at| (at, hand_comparisons(lines[at])))
            .filter(|(at, calls)| *calls > 0 && !gated(&lines, *at))
            .map(|(_, calls)| calls)
            .sum();
        if calls > 0 {
            found.insert(shown, calls);
        }
    }
    let allowed = |file: &str| {
        COMPARED_BY_HAND
            .iter()
            .find(|(known, _)| *known == file)
            .map_or(0, |(_, allowed)| *allowed)
    };
    let grown = found
        .iter()
        .filter(|(file, calls)| **calls > allowed(file))
        .map(|(file, calls)| format!("crates/{file}: {calls}, allowed {}", allowed(file)))
        .collect::<Vec<_>>();
    assert!(
        grown.is_empty(),
        "these files compare file reads by hand more than before; ask the disk service \
         (check, check_against, full_hash, entry_alias_of) instead:\n{}",
        grown.join("\n")
    );
    for (file, allowed) in COMPARED_BY_HAND {
        let calls = found.get(file).copied().unwrap_or(0);
        assert_eq!(
            calls, allowed,
            "lower the COMPARED_BY_HAND entry for {file} to {calls}"
        );
    }
}

#[test]
fn a_hand_comparison_is_a_call_and_not_a_definition_or_a_longer_name() {
    assert_eq!(hand_comparisons("    if !seen.still_marks(&now) {"), 1);
    assert_eq!(
        hand_comparisons("file_version(&a).await? == super::resolution::file_version(&b)"),
        2
    );
    assert_eq!(
        hand_comparisons("pub(super) async fn file_version(path: &Path)"),
        0
    );
    assert_eq!(hand_comparisons("    regular_file_same_file(a, b)"), 0);
    assert_eq!(hand_comparisons("    let same_file = true;"), 0);
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
        .filter(|at| direct(source[*at]) && !covered(&source, *at, &counted(&source)))
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
fn the_guard_follows_imported_functions_other_names_and_visible_imports() {
    let source = [
        "use std::fs::{File, remove_file};",
        "use tokio::fs::{",
        "    File as Handle,",
        "    read as slurp,",
        "};",
        "use std::fs as stdfs;",
        "use std::{",
        "    fs,",
        "    path::Path,",
        "};",
        "fn bare(path: &Path) {",
        "    remove_file(path).ok();",
        "    self.remove_file(path);",
        "    other::remove_file(path);",
        "    let _ = slurp(path);",
        "    let _ = stdfs::metadata(path);",
        "    let _ = fs::read(path);",
        "    let _: stdfs::File = made();",
        "}",
        "fn remove_file(path: &Path) {}",
    ];
    let expected = [0, 1, 5, 7, 11, 14, 15, 16];
    let found = raw(&source);
    let found = (0..source.len()).filter(|at| found[*at]);
    assert_eq!(found.collect::<Vec<_>>(), expected);
    assert_eq!(
        raw(&["pub(crate) use std::fs;", "fs::read(path)"]),
        [true; 2]
    );
    assert_eq!(raw(&["use std::fs::File;", "read(path)"]), [false; 2]);

    let mut imported = Imported::default();
    imported.take("", "std::{fs::{self as files, read, File}, io}");
    imported.take("", "::tokio::fs::write as put");
    let expected = Imported {
        modules: vec!["files".to_owned()],
        functions: vec!["read".to_owned(), "put".to_owned()],
    };
    assert_eq!(imported, expected);
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
    assert_eq!(counted(&source), HashSet::from([3]));

    // What a string holds neither encloses the code after it nor is code.
    let text = [
        "#[cfg(test)]",
        "mod tests {",
        "    const INVENTED: &str = r#\"",
        "<invented quote=\"one\">",
        "\"#;",
        "    const PLAIN: &str = \"a \\\" quote and a '\\\\' then",
        "std::fs::read(path)\";",
        "    fn helper<'a>(mark: char) -> bool {",
        "        mark == '\"' || std::fs::read(\"invented\").is_ok()",
        "    }",
        "}",
    ]
    .join("\n");
    let lines = code(&text);
    let lines = borrowed(&lines);
    // Only what follows the closing quote is left, where its string opened.
    assert_eq!(lines[3..5], ["", "    ;"]);
    assert_eq!(lines[6], "    ;");
    assert!(raw(&lines)[8] && covered(&lines, 8, &counted(&lines)));

    // A test-only derive says nothing about the item, so its marker counts.
    let derived = [
        "// disk-guard: an invented production reason.",
        "#[cfg_attr(test, derive(Debug))]",
        "#[allow(clippy::disallowed_methods)]",
        "fn marked() {}",
    ];
    assert!(!test_marker(&derived, 0));
    // A file or a module gated from the inside is test code throughout.
    let file = ["#![cfg(test)]", "", "fn fixture() {", "    read();", "}"];
    assert!(gated(&file, 3) && gated(&file, 2));
    let module = [
        "mod fixtures {",
        "    #![cfg(all(test, unix))]",
        "    fn fixture() {}",
        "}",
    ];
    assert!(gated(&module, 2) && !gated(&module[..1], 0));
}

#[test]
fn a_marker_for_tests_covers_no_production_line_and_an_unconditional_allow_is_counted() {
    assert!(test_only("#[cfg(test)]") && test_only("#[cfg(test,)]"));
    assert!(test_only("#[cfg(all(test, unix))]") && test_only("#![cfg(all(test))]"));
    assert!(!test_only("#[cfg(testing)]") && !test_only("#[cfg(all(tested, unix))]"));

    // The item is allowed the direct calls in every build, so the marker
    // stands on production code whatever else it says about tests.
    let both = [
        "// disk-guard: an invented reason.",
        "#[cfg_attr(test, allow(clippy::disallowed_types))]",
        "#[allow(clippy::disallowed_methods)]",
        "fn marked() {",
        "    std::fs::read(path);",
        "}",
    ];
    assert!(!test_marker(&both, 0));
    assert_eq!(counted(&both), HashSet::from([0]));
    assert!(covered(&both, 4, &counted(&both)));

    // A marker that explains a tests-only allow covers the tests alone.
    let for_tests = [
        "// disk-guard: test code may call the filesystem directly.",
        "#[cfg_attr(test, allow(clippy::disallowed_methods))]",
        "mod both {",
        "    fn production(path: &Path) {",
        "        std::fs::read(path);",
        "    }",
        "    #[cfg(test)]",
        "    fn fixture(path: &Path) {",
        "        std::fs::read(path);",
        "    }",
        "}",
    ];
    let marks = counted(&for_tests);
    assert!(test_marker(&for_tests, 0) && marks.is_empty());
    assert!(!covered(&for_tests, 4, &marks) && covered(&for_tests, 8, &marks));
}

#[test]
fn the_guard_sees_glob_imports_and_imports_under_an_attribute() {
    let source = [
        "use std::fs::*;",
        "fn bare(path: &Path) {",
        "    let _ = read(path);",
        "    let _ = self.read(path);",
        "    remove_file(path).ok();",
        "}",
    ];
    assert_eq!(raw(&source), [true, false, true, false, true, false]);
    let grouped = ["use tokio::{fs::*, io};", "    rename(from, to).await"];
    assert_eq!(raw(&grouped), [true; 2]);
    let whole = ["use std::*;", "    fs::read(path)", "    io::stdin()"];
    assert_eq!(raw(&whole), [true, true, false]);
    assert_eq!(raw(&["use std::io::*;", "    read(path)"]), [false; 2]);

    let attributed = [
        "#[cfg(unix)] use std::fs::remove_file;",
        "#[cfg(any(unix, target_os = \"invented\"))] pub(crate) use tokio::fs::read as slurp;",
        "    remove_file(path).ok();",
        "    slurp(path).await",
    ];
    assert_eq!(raw(&attributed), [true; 4]);
}

#[test]
fn strings_and_block_comments_are_not_code_and_what_follows_them_is() {
    let lines = |text: &str| {
        code(text)
            .into_iter()
            .map(Cow::into_owned)
            .collect::<Vec<_>>()
    };
    // A raw string with no `#` has no escapes: the backslash ends nothing.
    let raw_text = ["let sep = r\"\\\";", "std::fs::read(path);"].join("\n");
    assert_eq!(lines(&raw_text)[1], "std::fs::read(path);");
    // A quote inside a block comment opens no string.
    let commented = ["let a = 1; /* \" */", "std::fs::read(path);"].join("\n");
    assert_eq!(lines(&commented)[1], "std::fs::read(path);");
    // A block comment's own lines are not code, nested or not.
    let block = [
        "    /* outer /* inner",
        "std::fs::read(path)",
        "*/ still */ live();",
    ]
    .join("\n");
    assert_eq!(lines(&block)[1..], ["", "    live();"]);
    // Code after the quote that closes a string is still code.
    let tailed = ["    let text = \"first", "second\"; std::fs::read(path);"].join("\n");
    let tailed = lines(&tailed);
    assert_eq!(tailed[1], "    ; std::fs::read(path);");
    assert!(direct(&tailed[1]));
}
