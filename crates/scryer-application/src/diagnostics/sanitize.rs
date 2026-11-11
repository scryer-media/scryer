//! The one sanitizer every byte of a diagnostics bundle passes through.
//!
//! Text is processed as a list of segments. A segment is either still raw or
//! already rewritten; every layer only looks at raw segments, so a pseudonym
//! or placeholder written by one layer is never re-read by a later one. The
//! layers run in a fixed order:
//!
//! 1. secrets recognised by shape (header values, credential parameters,
//!    bearer tokens, Scryer API keys, JWTs, magnet links);
//! 2. URLs, whose host, userinfo, path and query are dropped;
//! 3. email addresses;
//! 4. absolute Unix, Windows and UNC paths, whose directory portion is dropped
//!    (see [`PATH_POLICY`]);
//! 5. known values gathered from the instance (names, hosts, usernames,
//!    labels, secrets, paths), each replaced by a stable pseudonym;
//! 6. `host:port` pairs, bare host names under common top-level domains, and
//!    IP addresses.
//!
//! Structured log lines are additionally sanitized by field name, so a field
//! called `password` or `base_url` loses its value whatever it looks like.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

use regex::{Regex, RegexBuilder};
use serde_json::Value;

/// What happens to the final component of a path once its directories are
/// stripped. Directories are always removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPolicy {
    /// Keep a final component that looks like a file or release name (it
    /// contains a `.`); parse and import problems cannot be diagnosed without
    /// it. A final component without a `.` is treated as a directory.
    KeepFileName,
    /// Replace the whole path with a placeholder.
    RedactAll,
}

/// The single policy point for file and release names inside paths.
pub const PATH_POLICY: PathPolicy = PathPolicy::KeepFileName;

/// Names shorter than this are not replaced: a two-letter name would rewrite
/// every occurrence of those letters. The count of skipped values is reported.
const MIN_KNOWN_VALUE_CHARS: usize = 3;
/// Secrets at least this long are replaced wherever they appear; shorter ones
/// and every other kind only as a whole word.
const ANYWHERE_SECRET_CHARS: usize = 6;

/// HTTP routes served by Scryer itself. A path starting with one of these is a
/// request path, not a filesystem path, and is kept as written.
const SCRYER_ROUTE_PREFIXES: &[&str] = &[
    "/api/",
    "/api",
    "/graphql",
    "/images/",
    "/backups/",
    "/compat/",
    "/oauth/",
    "/health",
    "/metrics",
    "/authless-client",
    "/newznab",
    "/torznab",
];

/// The kind of a value, which decides how it is matched and rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueClass {
    /// A user-entered name, label, username or host: whole-word matches.
    Word,
    /// A secret: replaced anywhere once it is long enough.
    Secret,
    /// A filesystem path: matched as a path prefix and as a substring.
    Path,
}

/// Collects the instance's sensitive values and the pseudonyms that replace
/// them. The mapping never leaves this type.
#[derive(Default)]
pub struct KnownValues {
    /// Lowercased value -> (pseudonym, class).
    values: HashMap<String, (String, ValueClass)>,
    /// Entity key -> pseudonym, so every value of one entity shares a name.
    entities: HashMap<String, String>,
    counters: HashMap<String, usize>,
    /// (normalized path, pseudonym), for prefix matching inside paths.
    paths: Vec<(String, String)>,
    /// Code-defined vocabulary (provider types, client types, facets) that a
    /// user-entered name must not take over: an indexer named after its own
    /// provider type would otherwise rewrite every mention of that type.
    protected: std::collections::HashSet<String>,
    skipped_short: usize,
    skipped_protected: usize,
}

impl KnownValues {
    pub fn new() -> Self {
        Self::default()
    }

    /// The pseudonym for an entity, allocating `kind-N(qualifier)` on first use.
    pub fn entity(&mut self, kind: &str, qualifier: Option<&str>, entity_key: &str) -> String {
        let key = format!("{kind}\u{0}{entity_key}");
        if let Some(existing) = self.entities.get(&key) {
            return existing.clone();
        }
        let counter = self.counters.entry(kind.to_string()).or_insert(0);
        *counter += 1;
        let pseudonym = match qualifier.map(safe_qualifier).filter(|q| !q.is_empty()) {
            Some(qualifier) => format!("{kind}-{counter}({qualifier})"),
            None => format!("{kind}-{counter}"),
        };
        self.entities.insert(key, pseudonym.clone());
        pseudonym
    }

    /// Marks a code-defined word that must survive sanitizing.
    pub fn protect(&mut self, word: &str) {
        let word = word.trim().to_lowercase();
        if !word.is_empty() {
            self.protected.insert(word);
        }
    }

    /// A name, label or username belonging to an entity.
    pub fn name(&mut self, pseudonym: &str, value: &str) {
        self.insert(value, pseudonym, ValueClass::Word);
    }

    /// A secret value (API key, password, token).
    pub fn secret(&mut self, value: &str) {
        self.insert(value, "redacted", ValueClass::Secret);
    }

    /// A URL or bare host belonging to an entity; registers the host, the
    /// host with its port, and the URL itself.
    pub fn url(&mut self, pseudonym: &str, raw: &str) {
        let raw = raw.trim();
        if raw.is_empty() {
            return;
        }
        let parsed = url::Url::parse(raw)
            .ok()
            .filter(|url| url.host_str().is_some())
            .or_else(|| url::Url::parse(&format!("http://{raw}")).ok());
        if let Some(parsed) = parsed {
            if let Some(host) = parsed.host_str() {
                let host = host.trim_start_matches('[').trim_end_matches(']');
                if !is_local_host(host) {
                    self.insert(host, pseudonym, ValueClass::Word);
                    if let Some(port) = parsed.port() {
                        self.insert(&format!("{host}:{port}"), pseudonym, ValueClass::Word);
                    }
                }
            }
            if let Some(password) = parsed.password() {
                self.secret(password);
            }
            if !parsed.username().is_empty() {
                self.insert(parsed.username(), "user", ValueClass::Word);
            }
            for (name, value) in parsed.query_pairs() {
                if is_secret_key(&name) {
                    self.secret(&value);
                }
            }
        }
        if raw.contains("://") {
            self.insert(raw.trim_end_matches('/'), pseudonym, ValueClass::Secret);
        }
    }

    /// A filesystem path belonging to an entity (a root folder, a download
    /// directory, a path-mapping side).
    pub fn path(&mut self, pseudonym: &str, raw: &str) {
        let trimmed = raw.trim();
        let normalized = normalize_path(trimmed);
        if normalized.len() < 2 {
            return;
        }
        if !self.paths.iter().any(|(path, _)| *path == normalized) {
            self.paths.push((normalized, pseudonym.to_string()));
        }
        self.insert(
            trimmed.trim_end_matches(['/', '\\']),
            pseudonym,
            ValueClass::Path,
        );
    }

    /// Harvests every sensitive value out of an entity's free-form JSON
    /// configuration: credentials by key, URLs and hosts by key or by shape,
    /// usernames by key, and every absolute path, including both sides of
    /// `REMOTE => LOCAL` mapping lines.
    pub fn config_json(&mut self, pseudonym: &str, raw: &str) {
        match serde_json::from_str::<Value>(raw) {
            Ok(value) => self.config_value(pseudonym, None, &value),
            Err(_) => self.config_string(pseudonym, None, raw),
        }
    }

    fn config_value(&mut self, pseudonym: &str, key: Option<&str>, value: &Value) {
        match value {
            Value::Object(map) => {
                for (child_key, child) in map {
                    self.config_value(pseudonym, Some(child_key), child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.config_value(pseudonym, key, item);
                }
            }
            Value::String(text) => self.config_string(pseudonym, key, text),
            Value::Number(number) if key.is_some_and(is_secret_key) => {
                self.secret(&number.to_string());
            }
            _ => {}
        }
    }

    fn config_string(&mut self, pseudonym: &str, key: Option<&str>, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let class = key.map(classify_field).unwrap_or(FieldClass::Plain);
        match class {
            FieldClass::Secret => self.secret(text),
            FieldClass::Url | FieldClass::Host => self.url(pseudonym, text),
            FieldClass::User | FieldClass::EntityName => self.name(pseudonym, text),
            FieldClass::Path | FieldClass::Plain => {}
        }
        for line in text.lines() {
            for piece in line.split("=>") {
                let piece = piece.trim();
                if looks_like_absolute_path(piece) {
                    self.path(&format!("{pseudonym}-path"), piece);
                } else if piece.contains("://") {
                    self.url(pseudonym, piece);
                }
            }
        }
    }

    fn insert(&mut self, value: &str, pseudonym: &str, class: ValueClass) {
        let value = value.trim();
        if value.chars().count() < MIN_KNOWN_VALUE_CHARS {
            if !value.is_empty() {
                self.skipped_short += 1;
            }
            return;
        }
        let key = value.to_lowercase();
        // A name, host or path keeps its first, most specific pseudonym; a
        // secret registration never downgrades an entity pseudonym.
        match self.values.get(&key) {
            Some((_, ValueClass::Secret)) if class != ValueClass::Secret => {
                self.values.insert(key, (pseudonym.to_string(), class));
            }
            Some(_) => {}
            None => {
                self.values.insert(key, (pseudonym.to_string(), class));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn skipped_short(&self) -> usize {
        self.skipped_short
    }

    pub fn into_sanitizer(mut self) -> Sanitizer {
        let protected = std::mem::take(&mut self.protected);
        let before = self.values.len();
        self.values
            .retain(|value, (_, class)| *class != ValueClass::Word || !protected.contains(value));
        self.skipped_protected = before - self.values.len();
        Sanitizer::new(self)
    }
}

fn safe_qualifier(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// The class a field name puts its value in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldClass {
    Secret,
    Url,
    Host,
    Path,
    User,
    EntityName,
    Plain,
}

fn normalized_key(key: &str) -> String {
    key.chars()
        .filter(|c| !matches!(c, '_' | '-' | '.' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_secret_key(key: &str) -> bool {
    classify_field(key) == FieldClass::Secret
}

const SECRET_KEY_WORDS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "authorization",
    "cookie",
    "privatekey",
    "passphrase",
    "credential",
    "passkey",
    "authkey",
    "sessionid",
    "signature",
    "rsskey",
    "torrentpass",
    "xapikey",
];

const EXACT_SECRET_KEYS: &[&str] = &["auth", "key", "pass", "pwd", "sid", "session", "jwt"];

const URL_KEYS: &[&str] = &["endpoint", "href", "link", "origin", "referer", "referrer"];

const HOST_KEYS: &[&str] = &[
    "host",
    "hostname",
    "server",
    "address",
    "addr",
    "remote",
    "remoteaddr",
    "peer",
    "ip",
    "clientip",
    "peerip",
    "domain",
    "bind",
    "listen",
];

const USER_KEYS: &[&str] = &[
    "user",
    "username",
    "displayname",
    "email",
    "login",
    "owner",
    "account",
    "accountname",
    "actorname",
    "externaluserid",
];

/// Names that end in `name` but carry a code-defined value or a release name.
const NAME_ALLOWLIST: &[&str] = &[
    "filename",
    "releasename",
    "operationname",
    "jobname",
    "eventname",
    "fieldname",
    "typename",
    "pluginname",
    "metricname",
    "columnname",
    "tablename",
    "methodname",
    "spanname",
    "statename",
    "facetname",
];

const ENTITY_NAME_KEYS: &[&str] = &["name", "label", "labels", "tag", "tags", "keylabel"];

const PATH_KEY_WORDS: &[&str] = &[
    "path",
    "dir",
    "directory",
    "folder",
    "root",
    "location",
    "destination",
    "mount",
];

/// Decides what a field's name says about its value.
pub(crate) fn classify_field(key: &str) -> FieldClass {
    let key = normalized_key(key);
    if EXACT_SECRET_KEYS.contains(&key.as_str())
        || SECRET_KEY_WORDS.iter().any(|word| key.contains(word))
    {
        return FieldClass::Secret;
    }
    if key.ends_with("url") || key.ends_with("uri") || URL_KEYS.contains(&key.as_str()) {
        return FieldClass::Url;
    }
    if HOST_KEYS.contains(&key.as_str())
        || key.ends_with("host")
        || key.ends_with("hostname")
        || key.ends_with("ip")
        || key.ends_with("addr")
        || key.ends_with("address")
    {
        return FieldClass::Host;
    }
    if USER_KEYS.contains(&key.as_str())
        || key.ends_with("username")
        || key.ends_with("displayname")
        || key.ends_with("email")
    {
        return FieldClass::User;
    }
    if NAME_ALLOWLIST.contains(&key.as_str()) {
        return FieldClass::Plain;
    }
    if ENTITY_NAME_KEYS.contains(&key.as_str())
        || key.ends_with("name")
        || key.ends_with("label")
        || key.ends_with("tags")
    {
        return FieldClass::EntityName;
    }
    if PATH_KEY_WORDS.iter().any(|word| key.contains(word))
        || key == "file"
        || key == "source"
        || key == "target"
    {
        return FieldClass::Path;
    }
    FieldClass::Plain
}

/// A text segment: still raw, or already rewritten.
enum Segment {
    Raw(String),
    Done(String),
}

/// Applies one layer to every raw segment. The layer returns the pieces a
/// raw segment becomes.
fn apply_layer(
    segments: Vec<Segment>,
    mut layer: impl FnMut(&str) -> Vec<Segment>,
) -> Vec<Segment> {
    let mut out = Vec::with_capacity(segments.len());
    for segment in segments {
        match segment {
            Segment::Raw(text) if !text.is_empty() => out.extend(layer(&text)),
            other => out.push(other),
        }
    }
    out
}

fn join(segments: Vec<Segment>) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            Segment::Raw(text) | Segment::Done(text) => out.push_str(&text),
        }
    }
    out
}

/// Splits `text` around regex matches, letting `replace` decide each match's
/// pieces; returning `None` keeps the match raw.
fn split_matches(
    text: &str,
    regex: &Regex,
    mut replace: impl FnMut(&regex::Captures<'_>) -> Option<Vec<Segment>>,
) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut last = 0;
    for captures in regex.captures_iter(text) {
        let whole = captures.get(0).expect("group 0 always matches");
        let Some(pieces) = replace(&captures) else {
            continue;
        };
        if whole.start() > last {
            out.push(Segment::Raw(text[last..whole.start()].to_string()));
        }
        out.extend(pieces);
        last = whole.end();
    }
    if last < text.len() {
        out.push(Segment::Raw(text[last..].to_string()));
    }
    out
}

fn placeholder(name: &str) -> String {
    format!("<{name}>")
}

fn static_regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("sanitizer pattern compiles"))
}

fn header_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(
        &CELL,
        r#"(?i)\b(authorization|proxy-authorization|cookie|set-cookie|x-api-key|x-plex-token|x-emby-token|x-emby-authorization|x-mediabrowser-token)(\s*[:=]\s*)([^\r\n"']+)"#,
    )
}

fn credential_param_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(
        &CELL,
        r#"(?i)\b([a-z0-9_.-]*(?:api[_-]?key|apikey|token|password|passwd|secret|passkey|authkey|rss_?key|torrent_?pass|session_?id|signature|credential)[a-z0-9_-]*|auth|key|pass|pwd|sid)("?\s*[=:]\s*"?)("(?:[^"\\]|\\.)*"|'[^']*'|[^\s&,;"'<>)\]}]+)"#,
    )
}

fn bearer_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(&CELL, r"(?i)\b(bearer|basic)(\s+)([A-Za-z0-9._~+/=-]{6,})")
}

fn scryer_key_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(&CELL, r"\bska_[A-Za-z0-9_.-]{4,}")
}

fn jwt_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(
        &CELL,
        r"\beyJ[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}",
    )
}

fn magnet_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(&CELL, r#"(?i)\bmagnet:\?[^\s"'<>]+"#)
}

fn url_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(
        &CELL,
        r#"(?i)\b([a-z][a-z0-9+.-]{1,15})://([^\s"'<>`{}|\\^]*)"#,
    )
}

fn host_port_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(
        &CELL,
        r"(?i)\b((?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]{1,62}):(\d{2,5})\b",
    )
}

fn next_field_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(&CELL, r"[ ,;]+[A-Za-z_][A-Za-z0-9_.]*=")
}

fn bare_host_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(
        &CELL,
        r"(?i)\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+(?:com|net|org|io|dev|me|info|xyz|cc|to|tv|club|lan|local|home|internal|arpa|cloud|app|site|online|space|top|link|eu|uk|de|nl|fr|ca|us|au|se|ch|biz|ws|su|ru|is|li|lol|win|one|host|server|nas|box|duckdns\.org|ts\.net)\b",
    )
}

/// Endings that look like a top-level domain but are file extensions.
const FILE_EXTENSIONS: &[&str] = &[
    "rs", "mkv", "mp4", "avi", "m4v", "ts", "srt", "ass", "sub", "idx", "nzb", "torrent", "log",
    "json", "toml", "yaml", "yml", "txt", "gz", "zst", "zip", "rar", "par2", "nfo", "jpg", "png",
    "webp", "db", "sqlite", "wasm", "go", "py", "js", "tsx", "sql", "sh", "exe", "dll", "so",
];

fn email_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(
        &CELL,
        r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+",
    )
}

fn key_value_regex() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    static_regex(
        &CELL,
        r#"\b([A-Za-z_][A-Za-z0-9_.]*)=("(?:[^"\\]|\\.)*"|[^\s&,;"'<>]+)"#,
    )
}

fn is_local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
}

fn normalize_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut previous_separator = false;
    for character in path.chars() {
        let separator = matches!(character, '/' | '\\');
        if separator {
            if !previous_separator {
                out.push('/');
            }
        } else {
            out.extend(character.to_lowercase());
        }
        previous_separator = separator;
    }
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

fn looks_like_absolute_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    (bytes.first() == Some(&b'/') && bytes.get(1).is_some_and(|b| *b != b'/' && *b != b' '))
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
        || (bytes.len() >= 3 && bytes[0] == b'\\' && bytes[1] == b'\\')
}

/// Characters that end a path in free text.
fn is_path_delimiter(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | '\n' | '\r' | '\t' | '<' | '>' | '|' | '{' | '}' | '*' | '?'
    )
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// The sanitizer built from an instance's known values.
pub struct Sanitizer {
    known: HashMap<String, (String, ValueClass)>,
    anywhere: Option<Regex>,
    words: Option<Regex>,
    paths: Vec<(String, String)>,
    skipped_short: usize,
    skipped_protected: usize,
}

impl Sanitizer {
    fn new(known: KnownValues) -> Self {
        let mut anywhere = Vec::new();
        let mut words = Vec::new();
        for (value, (_, class)) in &known.values {
            match class {
                ValueClass::Secret if value.chars().count() >= ANYWHERE_SECRET_CHARS => {
                    anywhere.push(value.clone())
                }
                ValueClass::Path => anywhere.push(value.clone()),
                _ => words.push(value.clone()),
            }
        }
        let mut paths = known.paths;
        // Longest prefix first, so a nested root wins over its parent.
        paths.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        Self {
            anywhere: alternation(anywhere),
            words: alternation(words),
            known: known.values,
            paths,
            skipped_short: known.skipped_short,
            skipped_protected: known.skipped_protected,
        }
    }

    /// A sanitizer with no known values: only the pattern layers run.
    pub fn patterns_only() -> Self {
        KnownValues::new().into_sanitizer()
    }

    pub fn known_value_count(&self) -> usize {
        self.known.len()
    }

    pub fn skipped_short_values(&self) -> usize {
        self.skipped_short
    }

    pub fn skipped_protected_values(&self) -> usize {
        self.skipped_protected
    }

    /// Sanitizes free text: `key=value` pairs by key, then every value layer.
    pub fn text(&self, text: &str) -> String {
        join(self.text_segments(text))
    }

    fn text_segments(&self, text: &str) -> Vec<Segment> {
        // Header values first: `Cookie: a=b; c=d` is one secret, not two
        // fields.
        let segments = split_matches(text, header_regex(), |captures| {
            Some(vec![
                Segment::Done(format!("{}{}", &captures[1], &captures[2])),
                Segment::Done(placeholder("redacted")),
            ])
        });
        let segments = apply_layer(segments, |text| self.key_value_segments(text));
        self.value_layers(segments)
    }

    fn key_value_segments(&self, text: &str) -> Vec<Segment> {
        split_matches(text, key_value_regex(), |captures| {
            let key = captures.get(1)?.as_str();
            let class = classify_field(key);
            if class == FieldClass::Plain {
                return None;
            }
            let raw_value = captures.get(2)?.as_str();
            let (quoted, inner) = match raw_value
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
            {
                Some(inner) => (true, inner),
                None => (false, raw_value),
            };
            if class == FieldClass::Path && !quoted {
                // An unquoted path may contain spaces; the path layer finds
                // its full extent.
                return None;
            }
            let rendered = self.classified_string(class, inner);
            let rendered = if quoted {
                format!("\"{rendered}\"")
            } else {
                rendered
            };
            Some(vec![
                Segment::Raw(format!("{key}=")),
                Segment::Done(rendered),
            ])
        })
    }

    /// Runs the value layers over a single value.
    fn value(&self, text: &str) -> String {
        join(self.value_layers(vec![Segment::Raw(text.to_string())]))
    }

    fn value_layers(&self, segments: Vec<Segment>) -> Vec<Segment> {
        let segments = apply_layer(segments, |text| self.secret_layer(text));
        let segments = apply_layer(segments, |text| self.url_layer(text));
        let segments = apply_layer(segments, |text| {
            split_matches(text, email_regex(), |_| {
                Some(vec![Segment::Done(placeholder("email"))])
            })
        });
        let segments = apply_layer(segments, |text| self.path_layer(text));
        let segments = apply_layer(segments, |text| self.known_layer(text));
        let segments = apply_layer(segments, |text| self.host_port_layer(text));
        let segments = apply_layer(segments, |text| {
            split_matches(text, bare_host_regex(), |captures| {
                let whole = captures.get(0)?;
                let after = text[whole.end()..].chars().next();
                // `name.tv.mkv`: the match continues as a file name.
                if after.is_some_and(|c| c == '.' || c == '-' || is_word_char(c)) {
                    return None;
                }
                Some(vec![Segment::Done(placeholder("host"))])
            })
        });
        apply_layer(segments, ip_layer)
    }

    /// Sanitizes a value by what its field name says it holds.
    fn classified_string(&self, class: FieldClass, value: &str) -> String {
        match class {
            FieldClass::Secret => {
                if value.is_empty() {
                    String::new()
                } else {
                    placeholder("redacted")
                }
            }
            FieldClass::Url | FieldClass::Host | FieldClass::User | FieldClass::EntityName => {
                let sanitized = self.value(value);
                if sanitized != value || value.is_empty() || is_local_host(value) {
                    sanitized
                } else {
                    placeholder(match class {
                        FieldClass::Url => "url",
                        FieldClass::Host => "host",
                        FieldClass::User => "user",
                        _ => "name",
                    })
                }
            }
            FieldClass::Path => {
                if value.contains(['/', '\\']) {
                    join(self.render_path(value))
                } else {
                    self.value(value)
                }
            }
            FieldClass::Plain => self.text(value),
        }
    }

    /// Sanitizes a JSON value in place: object fields by name, then strings by
    /// value. `structural` names the top-level keys of a log record that are
    /// written by Scryer and are left untouched.
    pub fn json(&self, value: &mut Value) {
        self.json_inner(None, value, &[]);
    }

    /// Sanitizes a JSON document whose keys Scryer wrote: every key and
    /// string value goes through the value layers, without field-name rules.
    pub fn json_values(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                let entries = std::mem::take(map);
                for (key, mut child) in entries {
                    self.json_values(&mut child);
                    map.insert(self.value(&key), child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.json_values(item);
                }
            }
            Value::String(text) => *text = self.value(text),
            _ => {}
        }
    }

    /// Sanitizes a JSON log record, leaving its own envelope keys alone.
    pub fn json_log_record(&self, value: &mut Value) {
        self.json_inner(None, value, &["timestamp", "level", "target"]);
    }

    fn json_inner(&self, key: Option<&str>, value: &mut Value, structural: &[&str]) {
        let class = key.map(classify_field).unwrap_or(FieldClass::Plain);
        match value {
            Value::Object(map) => {
                for (child_key, child) in map.iter_mut() {
                    if structural.contains(&child_key.as_str()) && child.is_string() {
                        continue;
                    }
                    self.json_inner(Some(child_key), child, &[]);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.json_inner(key, item, &[]);
                }
            }
            Value::String(text) => {
                *text = self.classified_string(class, text);
            }
            Value::Number(_) | Value::Bool(_) if class == FieldClass::Secret => {
                *value = Value::String(placeholder("redacted"));
            }
            _ => {}
        }
    }

    /// Sanitizes one log line, structured or text.
    pub fn log_line(&self, line: &str) -> String {
        let trimmed = line.trim_start();
        if trimmed.starts_with('{')
            && let Ok(mut record) = serde_json::from_str::<Value>(trimmed)
            && record.is_object()
        {
            self.json_log_record(&mut record);
            return serde_json::to_string(&record).unwrap_or_else(|_| placeholder("unrenderable"));
        }
        self.text(&strip_control_sequences(line))
    }

    /// Sanitizes a Prometheus text exposition, label value by label value.
    pub fn metrics(&self, exposition: &str) -> String {
        let mut out = String::with_capacity(exposition.len());
        for line in exposition.lines() {
            out.push_str(&self.metrics_line(line));
            out.push('\n');
        }
        out
    }

    fn metrics_line(&self, line: &str) -> String {
        if line.starts_with('#') {
            return self.text(line);
        }
        let Some(open) = line.find('{') else {
            return self.text(line);
        };
        let Some(close) = line.rfind('}') else {
            return self.text(line);
        };
        if close < open {
            return self.text(line);
        }
        let labels = parse_prometheus_labels(&line[open + 1..close]);
        let Some(labels) = labels else {
            return self.text(line);
        };
        let rendered = labels
            .into_iter()
            .map(|(name, value)| {
                let class = classify_field(&name);
                let sanitized = self.classified_string(class, &value);
                format!("{name}=\"{}\"", escape_prometheus_label(&sanitized))
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{}{{{rendered}}}{}",
            self.text(&line[..open]),
            self.text(&line[close + 1..])
        )
    }

    fn secret_layer(&self, text: &str) -> Vec<Segment> {
        let mut segments = split_matches(text, header_regex(), |captures| {
            Some(vec![
                Segment::Done(format!("{}{}", &captures[1], &captures[2])),
                Segment::Done(placeholder("redacted")),
            ])
        });
        segments = apply_layer(segments, |text| {
            split_matches(text, bearer_regex(), |captures| {
                Some(vec![
                    Segment::Done(format!("{}{}", &captures[1], &captures[2])),
                    Segment::Done(placeholder("redacted")),
                ])
            })
        });
        segments = apply_layer(segments, |text| {
            split_matches(text, credential_param_regex(), |captures| {
                let value = &captures[3];
                let quote = if value.starts_with('"') {
                    "\""
                } else if value.starts_with('\'') {
                    "'"
                } else {
                    ""
                };
                Some(vec![
                    Segment::Done(format!("{}{}", &captures[1], &captures[2])),
                    Segment::Done(format!("{quote}{}{quote}", placeholder("redacted"))),
                ])
            })
        });
        segments = apply_layer(segments, |text| {
            split_matches(text, scryer_key_regex(), |_| {
                Some(vec![Segment::Done(placeholder("scryer-api-key"))])
            })
        });
        segments = apply_layer(segments, |text| {
            split_matches(text, jwt_regex(), |_| {
                Some(vec![Segment::Done(placeholder("jwt"))])
            })
        });
        apply_layer(segments, |text| {
            split_matches(text, magnet_regex(), |_| {
                Some(vec![Segment::Done(format!(
                    "magnet:{}",
                    placeholder("redacted")
                ))])
            })
        })
    }

    fn url_layer(&self, text: &str) -> Vec<Segment> {
        split_matches(text, url_regex(), |captures| {
            let scheme = captures.get(1)?.as_str();
            let mut rest = captures.get(2)?.as_str();
            let whole = captures.get(0)?;
            // Trailing prose punctuation is not part of the URL.
            let trimmed_len = rest
                .trim_end_matches(['.', ',', ';', ':', ')', ']', '!'])
                .len();
            let trailing = &rest[trimmed_len..];
            rest = &rest[..trimmed_len];
            let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let authority = &rest[..authority_end];
            let after = &rest[authority_end..];
            let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            let (host, port) = split_host_port(host_port);
            let host_label = if host.is_empty() {
                String::new()
            } else if is_local_host(host) {
                host.to_string()
            } else if let Some((pseudonym, _)) = self
                .known
                .get(&host.to_lowercase())
                .or_else(|| self.known.get(&host_port.to_lowercase()))
            {
                placeholder(pseudonym)
            } else {
                placeholder("host")
            };
            let mut rendered = format!("{scheme}://{host_label}");
            if let Some(port) = port {
                rendered.push(':');
                rendered.push_str(port);
            }
            if !after.is_empty() && after != "/" {
                rendered.push_str("/<…>");
            }
            let _ = whole;
            let mut pieces = vec![Segment::Done(rendered)];
            if !trailing.is_empty() {
                pieces.push(Segment::Raw(trailing.to_string()));
            }
            Some(pieces)
        })
    }

    fn path_layer(&self, text: &str) -> Vec<Segment> {
        let mut out = Vec::new();
        let mut last = 0;
        let mut index = 0;
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        while index < chars.len() {
            let (offset, _) = chars[index];
            if path_start_at(text, offset).is_none() {
                index += 1;
                continue;
            }
            let rest = &text[offset..];
            let mut extent_end = rest.find(is_path_delimiter).unwrap_or(rest.len());
            // A REMOTE => LOCAL mapping line holds two paths.
            if let Some(arrow) = rest.find("=>") {
                extent_end = extent_end.min(arrow);
            }
            let mut extent = &rest[..extent_end];
            // A request path ends at the first space.
            let first_token = extent.split(char::is_whitespace).next().unwrap_or_default();
            if is_scryer_route(first_token) {
                extent = first_token;
            }
            // A second path after a space starts a new path.
            if let Some(next_path) = extent.char_indices().find_map(|(index, character)| {
                (character.is_whitespace()
                    && path_start_at(extent, index + character.len_utf8()).is_some())
                .then_some(index)
            }) {
                extent = &extent[..next_path];
            }
            // A following `key=value` field is not part of the path.
            if let Some(next_field) = next_field_regex().find(extent) {
                extent = &extent[..next_field.start()];
            }
            let extent = self.cut_at_known_root(extent.trim_end());
            if offset > last {
                out.push(Segment::Raw(text[last..offset].to_string()));
            }
            out.extend(self.render_path(extent));
            last = offset + extent.len();
            while index < chars.len() && chars[index].0 < last {
                index += 1;
            }
        }
        if last < text.len() {
            out.push(Segment::Raw(text[last..].to_string()));
        }
        out
    }

    /// A path in prose runs to the next hard delimiter, so it can swallow the
    /// words after it. When the text up to a space is exactly a known root and
    /// what follows names no further path, the path ends there.
    fn cut_at_known_root<'a>(&self, extent: &'a str) -> &'a str {
        for (index, character) in extent.char_indices() {
            if !character.is_whitespace() {
                continue;
            }
            let normalized = normalize_path(&extent[..index]);
            if self.paths.iter().any(|(root, _)| *root == normalized)
                && !extent[index..].contains(['/', '\\'])
            {
                return &extent[..index];
            }
        }
        extent
    }

    /// Renders one path under [`PATH_POLICY`]: a known root keeps its
    /// pseudonym, directories are dropped, and a file name may stay.
    fn render_path(&self, path: &str) -> Vec<Segment> {
        if is_scryer_route(path) {
            return vec![Segment::Raw(path.to_string())];
        }
        let normalized = normalize_path(path);
        let root = self.paths.iter().find(|(root, _)| {
            normalized == *root
                || (normalized.starts_with(root.as_str())
                    && normalized[root.len()..].starts_with('/'))
                || (root == "/" && normalized.starts_with('/'))
        });
        let prefix = match root {
            Some((_, pseudonym)) => placeholder(pseudonym),
            None => placeholder("path"),
        };
        if root.is_some_and(|(root, _)| *root == normalized) {
            return vec![Segment::Done(prefix)];
        }
        let last_separator = path.rfind(['/', '\\']);
        let name = last_separator.map_or(path, |index| &path[index + 1..]);
        // Under a known root, `<…>` marks dropped directories below it; an
        // unknown `<path>` already stands for the whole directory.
        let nested = root.is_some_and(|(root, _)| {
            normalized
                .matches('/')
                .count()
                .saturating_sub(root.matches('/').count())
                > 1
        });
        let middle = if nested { "/<…>/" } else { "/" };
        match PATH_POLICY {
            PathPolicy::KeepFileName if name.contains('.') && !name.trim().is_empty() => vec![
                Segment::Done(format!("{prefix}{middle}")),
                Segment::Raw(name.to_string()),
            ],
            PathPolicy::KeepFileName if name.is_empty() => vec![Segment::Done(prefix)],
            PathPolicy::KeepFileName => {
                vec![Segment::Done(format!(
                    "{prefix}{middle}{}",
                    placeholder("dir")
                ))]
            }
            PathPolicy::RedactAll => vec![Segment::Done(prefix)],
        }
    }

    fn known_layer(&self, text: &str) -> Vec<Segment> {
        let mut matches: Vec<(usize, usize)> = Vec::new();
        if let Some(regex) = &self.anywhere {
            matches.extend(regex.find_iter(text).map(|m| (m.start(), m.end())));
        }
        if let Some(regex) = &self.words {
            let mut position = 0;
            while position <= text.len() {
                let Some(found) = regex.find_at(text, position) else {
                    break;
                };
                let before = text[..found.start()].chars().next_back();
                let after = text[found.end()..].chars().next();
                if before.is_none_or(|c| !is_word_char(c)) && after.is_none_or(|c| !is_word_char(c))
                {
                    matches.push((found.start(), found.end()));
                    position = found.end();
                } else {
                    position = found.start()
                        + text[found.start()..]
                            .chars()
                            .next()
                            .map_or(1, char::len_utf8);
                }
                if found.end() == found.start() {
                    position += 1;
                }
            }
        }
        if matches.is_empty() {
            return vec![Segment::Raw(text.to_string())];
        }
        // Earliest first, longest at the same start; drop overlaps.
        matches.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let mut out = Vec::new();
        let mut last = 0;
        for (start, end) in matches {
            if start < last {
                continue;
            }
            let Some((pseudonym, _)) = self.known.get(&text[start..end].to_lowercase()) else {
                continue;
            };
            if start > last {
                out.push(Segment::Raw(text[last..start].to_string()));
            }
            out.push(Segment::Done(placeholder(pseudonym)));
            last = end;
        }
        if last < text.len() {
            out.push(Segment::Raw(text[last..].to_string()));
        }
        out
    }

    fn host_port_layer(&self, text: &str) -> Vec<Segment> {
        split_matches(text, host_port_regex(), |captures| {
            let host = captures.get(1)?.as_str();
            let tld = host
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if is_local_host(host) || FILE_EXTENSIONS.contains(&tld.as_str()) {
                return None;
            }
            Some(vec![Segment::Done(format!(
                "{}:{}",
                placeholder("host"),
                &captures[2]
            ))])
        })
    }
}

fn alternation(mut values: Vec<String>) -> Option<Regex> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    let pattern = values
        .iter()
        .map(|value| regex::escape(value))
        .collect::<Vec<_>>()
        .join("|");
    RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .size_limit(64 * 1024 * 1024)
        .dfa_size_limit(64 * 1024 * 1024)
        .build()
        .ok()
}

fn split_host_port(host_port: &str) -> (&str, Option<&str>) {
    if let Some(rest) = host_port.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            return (host, tail.strip_prefix(':').filter(|p| !p.is_empty()));
        }
        return (host_port, None);
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host, Some(port).filter(|p| !p.is_empty()))
        }
        _ => (host_port, None),
    }
}

fn is_scryer_route(path: &str) -> bool {
    SCRYER_ROUTE_PREFIXES.iter().any(|prefix| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || prefix.ends_with('/') || rest.starts_with('/'))
    })
}

/// Whether an absolute path starts at `offset`; returns the length of its
/// leading marker.
fn path_start_at(text: &str, offset: usize) -> Option<usize> {
    let before = text[..offset].chars().next_back();
    let rest = &text[offset..];
    let bytes = rest.as_bytes();
    let boundary = before.is_none_or(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '=' | '"' | '\'' | '(' | '[' | ',' | ':' | '`' | '{' | ';'
            )
    });
    if !boundary {
        return None;
    }
    // UNC: two or more backslashes, then a server name.
    if bytes.first() == Some(&b'\\') {
        let run = bytes.iter().take_while(|b| **b == b'\\').count();
        if run >= 2 && bytes.get(run).is_some_and(|b| b.is_ascii_alphanumeric()) {
            return Some(run);
        }
        return None;
    }
    // Windows drive.
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
    {
        return Some(3);
    }
    // Unix absolute path; `//` belongs to a URL and is handled there.
    if bytes.first() == Some(&b'/')
        && bytes
            .get(1)
            .is_some_and(|b| !matches!(*b, b'/' | b' ' | b'\n' | b'\t' | b'"' | b'\''))
    {
        return Some(1);
    }
    None
}

/// Rewrites IPv4 and IPv6 literals, keeping loopback and unspecified ones.
fn ip_layer(text: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut last = 0;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let is_candidate = |b: u8| b.is_ascii_hexdigit() || b == b':' || b == b'.';
        if !is_candidate(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        let mut end = index;
        while end < bytes.len() && is_candidate(bytes[end]) {
            end += 1;
        }
        index = end;
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if before.is_some_and(|c| is_word_char(c) || c == '.') || after.is_some_and(is_word_char) {
            continue;
        }
        let token = text[start..end].trim_end_matches(['.', ':']);
        let token_end = start + token.len();
        let Some(rendered) = render_ip_token(token) else {
            continue;
        };
        if start > last {
            out.push(Segment::Raw(text[last..start].to_string()));
        }
        out.push(Segment::Done(rendered));
        last = token_end;
    }
    if last < text.len() {
        out.push(Segment::Raw(text[last..].to_string()));
    }
    out
}

fn render_ip_token(token: &str) -> Option<String> {
    if let Ok(ip) = token.parse::<Ipv4Addr>() {
        return (!ip.is_loopback() && !ip.is_unspecified()).then(|| placeholder("ip"));
    }
    if token.matches(':').count() >= 2
        && let Ok(ip) = token.parse::<Ipv6Addr>()
    {
        return (!ip.is_loopback() && !ip.is_unspecified()).then(|| placeholder("ip"));
    }
    // IPv4 with a port.
    if let Some((address, port)) = token.rsplit_once(':')
        && !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
        && let Ok(ip) = address.parse::<Ipv4Addr>()
    {
        return (!ip.is_loopback() && !ip.is_unspecified())
            .then(|| format!("{}:{port}", placeholder("ip")));
    }
    None
}

/// Removes ANSI escape sequences so they cannot hide a value from the layers.
fn strip_control_sequences(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(character);
    }
    out
}

/// Parses `name="value",...`; `None` when the block is not well formed.
fn parse_prometheus_labels(block: &str) -> Option<Vec<(String, String)>> {
    let mut labels = Vec::new();
    let mut chars = block.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ',' || c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            return Some(labels);
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' {
                break;
            }
            name.push(c);
            chars.next();
        }
        if chars.next() != Some('=') || chars.next() != Some('"') {
            return None;
        }
        let mut value = String::new();
        loop {
            match chars.next()? {
                '\\' => match chars.next()? {
                    'n' => value.push('\n'),
                    other => value.push(other),
                },
                '"' => break,
                other => value.push(other),
            }
        }
        labels.push((name.trim().to_string(), value));
    }
}

fn escape_prometheus_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
