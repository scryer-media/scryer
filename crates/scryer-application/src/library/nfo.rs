use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use scryer_domain::{Episode, Title};
use std::io::Cursor;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Metadata extracted from a .nfo sidecar file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NfoMetadata {
    pub tvdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub title: Option<String>,
    pub year: Option<i32>,
}

impl NfoMetadata {
    pub(crate) fn has_external_ids(&self) -> bool {
        self.tvdb_id.is_some() || self.imdb_id.is_some() || self.tmdb_id.is_some()
    }

    pub(crate) fn is_empty(&self) -> bool {
        !self.has_external_ids()
            && self
                .title
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self.year.is_none()
    }
}

fn canonical_title_genres(title: &Title) -> Vec<String> {
    title
        .canonical_tags
        .iter()
        .filter(|tag| tag.category.eq_ignore_ascii_case("genre"))
        .map(|tag| tag.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NfoRootKind {
    Movie,
    TvShow,
    Episode,
    Other,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse an NFO file body into structured metadata.
///
/// Handles all common NFO variants:
/// - Kodi v17+: `<uniqueid type="tvdb">12345</uniqueid>`
/// - Jellyfin/Emby: `<tvdbid>`, `<imdbid>`, `<tmdbid>` tags
/// - Legacy: `<id>tt1234567</id>` or `<id>12345</id>`
/// - URL-only files: `imdb.com/title/tt...`, `thetvdb.com/?id=...`
///
/// Unknown elements are silently ignored — extra metadata in the NFO won't
/// cause failures.
pub(crate) fn parse_nfo(content: &str) -> NfoMetadata {
    let normalized = strip_utf8_bom(content);
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return NfoMetadata::default();
    }

    let mut meta = NfoMetadata::default();

    if trimmed.starts_with('<') {
        parse_xml_nfo(normalized, &mut meta);
    } else {
        apply_url_ids_from_text(normalized, &mut meta);
    }

    meta
}

pub(crate) fn detect_nfo_root_kind(content: &str) -> NfoRootKind {
    let normalized = strip_utf8_bom(content);
    let trimmed = normalized.trim();
    if trimmed.is_empty() || !trimmed.starts_with('<') {
        return NfoRootKind::Other;
    }

    let mut reader = Reader::from_str(normalized);
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref event)) | Ok(Event::Empty(ref event)) => {
                let name = event.name().as_ref().to_lowercase();
                return match name.as_str() {
                    "movie" => NfoRootKind::Movie,
                    "tvshow" => NfoRootKind::TvShow,
                    "episodedetails" => NfoRootKind::Episode,
                    _ => NfoRootKind::Other,
                };
            }
            Ok(Event::Eof) => return NfoRootKind::Other,
            Err(_) => return NfoRootKind::Other,
            _ => {}
        }
    }
}

#[cfg(test)]
pub(crate) fn looks_like_movie_nfo(content: &str) -> bool {
    detect_nfo_root_kind(content) == NfoRootKind::Movie
}

fn strip_utf8_bom(content: &str) -> &str {
    content.trim_start_matches('\u{feff}')
}

fn parse_xml_nfo(content: &str, meta: &mut NfoMetadata) {
    let mut reader = Reader::from_str(content);
    let root_kind = detect_nfo_root_kind(content);

    let mut current_tag = String::new();
    let mut current_text = String::new();
    let mut current_depth = 0usize;
    let mut depth = 0usize;
    let mut uniqueid_type: Option<String> = None;

    let mut url_fallback_text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                depth = depth.saturating_add(1);
                let name = e.name().as_ref().to_lowercase();
                if depth == 2 {
                    current_tag = name.clone();
                    current_text.clear();
                    current_depth = depth;
                    if name == "id" && root_kind != NfoRootKind::Episode {
                        apply_id_attribute_provider_ids(e, meta);
                    }
                    uniqueid_type = e
                        .attributes()
                        .filter_map(|a| a.ok())
                        .find(|a| a.key.as_ref() == "type")
                        .map(|a| a.value.to_lowercase())
                        .filter(|_| name == "uniqueid");
                }
            }
            Ok(Event::Text(ref e)) => {
                if current_depth == depth
                    && let Ok(decoded) = quick_xml::escape::unescape(e.as_ref())
                {
                    current_text.push_str(&decoded);
                }
            }
            Ok(Event::GeneralRef(ref e)) if current_depth == depth => {
                if let Ok(Some(ch)) = e.resolve_char_ref() {
                    current_text.push(ch);
                } else if let Some(entity) =
                    quick_xml::escape::resolve_predefined_entity(e.as_ref())
                {
                    current_text.push_str(entity);
                }
            }
            Ok(Event::Comment(ref e)) if depth <= 1 => {
                push_url_fallback_text(&mut url_fallback_text, e.as_ref());
            }
            Ok(Event::End(_)) => {
                if current_depth != depth {
                    depth = depth.saturating_sub(1);
                    continue;
                }
                let text = current_text.trim().to_string();
                if !text.is_empty() {
                    push_url_fallback_text(&mut url_fallback_text, &text);
                    match current_tag.as_str() {
                        "uniqueid" => {
                            if let Some(ref uid_type) = uniqueid_type {
                                match uid_type.as_str() {
                                    "tvdb"
                                        if root_kind != NfoRootKind::Episode
                                            && meta.tvdb_id.is_none()
                                            && looks_like_numeric_id(&text) =>
                                    {
                                        meta.tvdb_id = Some(text);
                                    }
                                    "imdb"
                                        if root_kind != NfoRootKind::Episode
                                            && meta.imdb_id.is_none() =>
                                    {
                                        meta.imdb_id = normalize_imdb(&text);
                                    }
                                    "tmdb"
                                        if root_kind != NfoRootKind::Episode
                                            && meta.tmdb_id.is_none()
                                            && looks_like_numeric_id(&text) =>
                                    {
                                        meta.tmdb_id = Some(text);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "tvdbid"
                            if root_kind != NfoRootKind::Episode
                                && meta.tvdb_id.is_none()
                                && looks_like_numeric_id(&text) =>
                        {
                            meta.tvdb_id = Some(text);
                        }
                        "imdbid" | "imdb_id"
                            if root_kind != NfoRootKind::Episode && meta.imdb_id.is_none() =>
                        {
                            meta.imdb_id = normalize_imdb(&text);
                        }
                        "tmdbid"
                            if root_kind != NfoRootKind::Episode
                                && meta.tmdb_id.is_none()
                                && looks_like_numeric_id(&text) =>
                        {
                            meta.tmdb_id = Some(text);
                        }
                        "title" if meta.title.is_none() => {
                            meta.title = Some(text);
                        }
                        "year" if meta.year.is_none() => {
                            meta.year = text
                                .parse::<i32>()
                                .ok()
                                .filter(|&y| (1888..=2100).contains(&y));
                        }
                        _ => {} // silently skip unknown elements
                    }
                }
                current_tag.clear();
                current_text.clear();
                current_depth = 0;
                uniqueid_type = None;
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => break,
            Err(_) => break, // graceful on malformed XML
            _ => {}
        }
    }

    if root_kind != NfoRootKind::Episode {
        apply_url_ids_from_text(&url_fallback_text, meta);
    }
}

fn apply_id_attribute_provider_ids(event: &BytesStart<'_>, meta: &mut NfoMetadata) {
    for attr in event.attributes().filter_map(|attr| attr.ok()) {
        let key = attr.key.as_ref().to_ascii_lowercase();
        let value = attr.value.trim().to_string();
        if value.is_empty() {
            continue;
        }

        match key.as_str() {
            "imdb" if meta.imdb_id.is_none() => {
                meta.imdb_id = normalize_imdb(&value);
            }
            "tmdb" if meta.tmdb_id.is_none() => {
                meta.tmdb_id = crate::normalize::normalize_numeric_id(&value);
            }
            "tvdb" if meta.tvdb_id.is_none() => {
                meta.tvdb_id = crate::normalize::normalize_numeric_id(&value);
            }
            _ => {}
        }
    }
}

pub(crate) fn parse_plexmatch(content: &str) -> NfoMetadata {
    let mut meta = NfoMetadata::default();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once(':') else {
            continue;
        };
        let key = raw_key.trim().to_ascii_lowercase();
        let value = raw_value.trim();
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "title" | "show" if meta.title.is_none() => meta.title = Some(value.to_string()),
            "year" if meta.year.is_none() => {
                meta.year = value
                    .parse::<i32>()
                    .ok()
                    .filter(|&year| (1888..=2100).contains(&year));
            }
            "imdbid" if meta.imdb_id.is_none() => {
                meta.imdb_id = normalize_plexmatch_imdb(value);
            }
            "tmdbid" if meta.tmdb_id.is_none() => {
                meta.tmdb_id = crate::normalize::normalize_numeric_id(value);
            }
            "tvdbid" if meta.tvdb_id.is_none() => {
                meta.tvdb_id = crate::normalize::normalize_numeric_id(value);
            }
            "guid" => apply_plexmatch_guid(value, &mut meta),
            _ => {}
        }
    }
    meta
}

fn apply_plexmatch_guid(value: &str, meta: &mut NfoMetadata) {
    let Some((scheme, raw_id)) = value.trim().split_once("://") else {
        return;
    };
    match scheme.trim().to_ascii_lowercase().as_str() {
        "imdb" if meta.imdb_id.is_none() => meta.imdb_id = normalize_plexmatch_imdb(raw_id),
        "tmdb" if meta.tmdb_id.is_none() => {
            meta.tmdb_id = crate::normalize::normalize_numeric_id(raw_id);
        }
        "tvdb" if meta.tvdb_id.is_none() => {
            meta.tvdb_id = crate::normalize::normalize_numeric_id(raw_id);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Render a Jellyfin/Kodi-compatible `<movie>` NFO for the given Title.
pub(crate) fn render_movie_nfo(title: &Title) -> String {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    write_xml_decl(&mut w);
    let movie = BytesStart::new("movie");
    w.write_event(Event::Start(movie)).ok();

    write_element(&mut w, "title", &title.name);

    if let Some(year) = title.year {
        write_element(&mut w, "year", &year.to_string());
    }
    write_optional_non_empty_element(&mut w, "plot", title.overview.as_deref());
    if let Some(runtime) = title.runtime_minutes.filter(|runtime| *runtime > 0) {
        write_element(&mut w, "runtime", &runtime.to_string());
    }
    for genre in canonical_title_genres(title) {
        if !genre.is_empty() {
            write_element(&mut w, "genre", &genre);
        }
    }
    write_optional_non_empty_element(&mut w, "studio", title.studio.as_deref());

    write_movie_uniqueids(&mut w, title);

    w.write_event(Event::End(BytesEnd::new("movie"))).ok();
    finish_xml(buf)
}

/// Render a Jellyfin/Kodi-compatible `<tvshow>` NFO for the given series Title.
pub(crate) fn render_tvshow_nfo(title: &Title) -> String {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    write_xml_decl(&mut w);
    let tvshow = BytesStart::new("tvshow");
    w.write_event(Event::Start(tvshow)).ok();

    write_element(&mut w, "title", &title.name);

    if let Some(year) = title.year {
        write_element(&mut w, "year", &year.to_string());
    }
    write_optional_non_empty_element(&mut w, "plot", title.overview.as_deref());
    for genre in canonical_title_genres(title) {
        if !genre.is_empty() {
            write_element(&mut w, "genre", &genre);
        }
    }
    write_optional_non_empty_element(&mut w, "studio", title.network.as_deref());

    write_tvshow_uniqueids(&mut w, title);

    w.write_event(Event::End(BytesEnd::new("tvshow"))).ok();
    finish_xml(buf)
}

/// Render a Kodi-compatible `<episodedetails>` NFO.
pub(crate) fn render_episode_nfo(title: &Title, episode: &Episode) -> String {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    write_xml_decl(&mut w);
    let tag = BytesStart::new("episodedetails");
    w.write_event(Event::Start(tag)).ok();

    write_element(&mut w, "showtitle", &title.name);
    write_optional_non_empty_element(&mut w, "title", episode.title.as_deref());
    if let Some(ref season) = episode.season_number {
        write_element(&mut w, "season", season);
    }
    if let Some(ref ep_num) = episode.episode_number {
        write_element(&mut w, "episode", ep_num);
    }
    write_optional_non_empty_element(&mut w, "plot", episode.overview.as_deref());
    write_optional_non_empty_element(&mut w, "aired", episode.air_date.as_deref());
    if let Some(duration_secs) = episode.duration_seconds {
        let minutes = duration_secs / 60;
        if minutes > 0 {
            write_element(&mut w, "runtime", &minutes.to_string());
        }
    }

    // Episode-level uniqueid: prefer the episode's own TVDB ID, fall back to series
    if let Some(tvdb_id) = &episode.tvdb_id {
        write_uniqueid(&mut w, "tvdb", tvdb_id, true);
    } else if let Some(tvdb_id) = title_external_id_value(title, "tvdb") {
        write_uniqueid(&mut w, "tvdb", tvdb_id, true);
    }

    w.write_event(Event::End(BytesEnd::new("episodedetails")))
        .ok();
    finish_xml(buf)
}

/// Render a Kodi/Jellyfin-compatible `<episodedetails>` NFO for a series movie.
///
/// Written as a season 0 special so media servers recognize it as part of the series.
/// Includes `<airsbefore_season>` for Jellyfin's "Display specials within seasons" feature.
pub(crate) fn render_series_movie_episode_nfo(
    movie: &scryer_domain::MovieEntity,
    season_episode: &str,
    after_season: Option<i32>,
) -> String {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    write_xml_decl(&mut w);
    let tag = BytesStart::new("episodedetails");
    w.write_event(Event::Start(tag)).ok();

    write_element(&mut w, "title", &movie.title);
    write_element(&mut w, "season", "0");

    if let Some(ep_str) = season_episode.strip_prefix("S00E")
        && let Ok(ep_num) = ep_str.parse::<i32>()
    {
        write_element(&mut w, "episode", &ep_num.to_string());
    }

    if let Some(overview) = movie.overview.as_deref().filter(|value| !value.is_empty()) {
        write_element(&mut w, "plot", overview);
    }
    if let Some(ref release_date) = movie.digital_release_date {
        write_element(&mut w, "aired", release_date);
    }
    if let Some(runtime_minutes) = movie.runtime_minutes.filter(|minutes| *minutes > 0) {
        write_element(&mut w, "runtime", &runtime_minutes.to_string());
    }

    if let Some(after_season) = after_season {
        write_element(&mut w, "airsbefore_season", &(after_season + 1).to_string());
        write_element(&mut w, "airsbefore_episode", "1");
    }

    if let Some(tvdb_id) = movie.tvdb_id.as_deref().filter(|value| !value.is_empty()) {
        write_uniqueid(&mut w, "tvdb", tvdb_id, true);
    }
    if let Some(imdb_id) = movie.imdb_id.as_deref().filter(|value| !value.is_empty()) {
        write_uniqueid(&mut w, "imdb", imdb_id, false);
    }
    if let Some(ref tmdb_id) = movie.tmdb_id {
        write_uniqueid(&mut w, "tmdb", tmdb_id, false);
    }

    w.write_event(Event::End(BytesEnd::new("episodedetails")))
        .ok();
    finish_xml(buf)
}

/// Render a Plex/Sonarr-style `.plexmatch` hint file for the given series Title.
///
/// Plain text key-value format. Lines are omitted when the value is empty.
/// Only applicable to TV series — Plex and Radarr do not define a movie
/// `.plexmatch` format.
pub(crate) fn render_plexmatch(title: &Title) -> String {
    let mut out = format!("Title: {}\n", title.name);

    if let Some(year) = title.year {
        out.push_str(&format!("Year: {year}\n"));
    }

    push_optional_non_empty_line(&mut out, "TvdbId", title_external_id_value(title, "tvdb"));
    push_optional_non_empty_line(&mut out, "ImdbId", title.imdb_id.as_deref());
    push_optional_non_empty_line(&mut out, "TmdbId", title_external_id_value(title, "tmdb"));

    out
}

// ---------------------------------------------------------------------------
// Helpers — parser
// ---------------------------------------------------------------------------

/// Returns true if the string looks like a numeric ID (non-empty, all ASCII digits).
fn looks_like_numeric_id(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
}

/// Normalize a raw string to a canonical IMDb ID. NFO provider fields must carry
/// a real `tt...` IMDb ID; all-digit values are often mislabeled TMDB/TVDB IDs.
fn normalize_imdb(raw: &str) -> Option<String> {
    let value = raw.trim().trim_matches('"').trim();
    if !value.to_ascii_lowercase().contains("tt") {
        return None;
    }
    crate::normalize::normalize_imdb_id(value)
}

fn normalize_plexmatch_imdb(raw: &str) -> Option<String> {
    crate::normalize::normalize_imdb_id(raw.trim().trim_matches('"').trim())
}

fn push_url_fallback_text(out: &mut String, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(text);
}

fn apply_url_ids_from_text(content: &str, meta: &mut NfoMetadata) {
    if meta.imdb_id.is_none() {
        meta.imdb_id = extract_imdb_url_id(content);
    }
    if meta.tvdb_id.is_none() {
        meta.tvdb_id = extract_tvdb_url_id(content);
    }
    if meta.tmdb_id.is_none() {
        meta.tmdb_id = extract_tmdb_url_id(content);
    }
}

/// Extract IMDb ID from URL pattern: `imdb.com/title/(tt\d+)`
fn extract_imdb_url_id(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let marker = "imdb.com/title/";
    let pos = lower.find(marker)? + marker.len();
    let rest = &content[pos..];
    if !rest.starts_with("tt") {
        return None;
    }
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if id.len() > 2 { Some(id) } else { None }
}

/// Extract TVDB ID from URL pattern: `thetvdb.com/...id=(\d+)`
fn extract_tvdb_url_id(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let domain_pos = lower.find("thetvdb.com")?;
    let after = &lower[domain_pos..];
    let id_pos = after.find("?id=").or_else(|| after.find("&id="))?;
    let digits_start = domain_pos + id_pos + 4;
    let digits: String = content[digits_start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// Extract TMDB ID from URL patterns like `themoviedb.org/movie/(\d+)` or `/tv/(\d+)`.
fn extract_tmdb_url_id(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    for marker in ["themoviedb.org/movie/", "themoviedb.org/tv/"] {
        let Some(pos) = lower.find(marker).map(|pos| pos + marker.len()) else {
            continue;
        };
        let digits: String = content[pos..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return Some(digits);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Helpers — writer
// ---------------------------------------------------------------------------

fn write_xml_decl<W: std::io::Write>(w: &mut Writer<W>) {
    w.write_event(Event::Decl(quick_xml::events::BytesDecl::new(
        "1.0",
        Some("UTF-8"),
        Some("yes"),
    )))
    .ok();
}

fn write_element<W: std::io::Write>(w: &mut Writer<W>, tag: &str, value: &str) {
    w.write_event(Event::Start(BytesStart::new(tag))).ok();
    w.write_event(Event::Text(BytesText::new(value))).ok();
    w.write_event(Event::End(BytesEnd::new(tag))).ok();
}

fn write_optional_non_empty_element<W: std::io::Write>(
    w: &mut Writer<W>,
    tag: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        write_element(w, tag, value);
    }
}

fn write_uniqueid<W: std::io::Write>(w: &mut Writer<W>, id_type: &str, value: &str, default: bool) {
    let mut tag = BytesStart::new("uniqueid");
    tag.push_attribute(("type", id_type));
    if default {
        tag.push_attribute(("default", "true"));
    }
    w.write_event(Event::Start(tag)).ok();
    w.write_event(Event::Text(BytesText::new(value))).ok();
    w.write_event(Event::End(BytesEnd::new("uniqueid"))).ok();
}

fn write_movie_uniqueids<W: std::io::Write>(w: &mut Writer<W>, title: &Title) {
    let tmdb_id = title_external_id_value(title, "tmdb");
    let imdb_id = title.imdb_id.as_deref().filter(|imdb| !imdb.is_empty());
    let tvdb_id = title_external_id_value(title, "tvdb");

    if let Some(tmdb_id) = tmdb_id {
        write_uniqueid(w, "tmdb", tmdb_id, true);
    }
    if let Some(imdb_id) = imdb_id {
        write_uniqueid(w, "imdb", imdb_id, tmdb_id.is_none());
    }
    if let Some(tvdb_id) = tvdb_id {
        write_uniqueid(w, "tvdb", tvdb_id, false);
    }

    write_optional_non_empty_element(w, "tmdbid", tmdb_id);
    write_optional_non_empty_element(w, "imdbid", imdb_id);
    write_optional_non_empty_element(w, "tvdbid", tvdb_id);
}

fn write_tvshow_uniqueids<W: std::io::Write>(w: &mut Writer<W>, title: &Title) {
    let tvdb_id = title_external_id_value(title, "tvdb");
    let tmdb_id = title_external_id_value(title, "tmdb");
    let imdb_id = title.imdb_id.as_deref().filter(|imdb| !imdb.is_empty());

    if let Some(tvdb_id) = tvdb_id {
        write_uniqueid(w, "tvdb", tvdb_id, true);
    }
    if let Some(tmdb_id) = tmdb_id {
        write_uniqueid(w, "tmdb", tmdb_id, tvdb_id.is_none());
    }
    if let Some(imdb_id) = imdb_id {
        write_uniqueid(w, "imdb", imdb_id, false);
    }

    write_optional_non_empty_element(w, "tvdbid", tvdb_id);
    write_optional_non_empty_element(w, "tmdbid", tmdb_id);
    write_optional_non_empty_element(w, "imdb_id", imdb_id);
}

fn title_external_id_value<'a>(title: &'a Title, source: &str) -> Option<&'a str> {
    title
        .external_ids
        .iter()
        .find(|external_id| external_id.source.eq_ignore_ascii_case(source))
        .map(|external_id| external_id.value.as_str())
        .filter(|value| !value.is_empty())
}

fn push_optional_non_empty_line(out: &mut String, key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        out.push_str(&format!("{key}: {value}\n"));
    }
}

fn finish_xml(buf: Cursor<Vec<u8>>) -> String {
    let bytes = buf.into_inner();
    let mut s = String::from_utf8(bytes).unwrap_or_default();
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::Instant;
    use std::{cmp, fs as stdfs};

    fn nightfall_tvshow_nfo() -> &'static str {
        r#"<?xml version="1.0" encoding="utf-8" standalone="yes"?>
<tvshow>
  <plot>Nightfall!! follows the remnant wardens of a ruined sky-kingdom as they try to stop a shard-born eclipse from swallowing the last inhabited cities.</plot>
  <outline>Nightfall!! follows the remnant wardens of a ruined sky-kingdom as they try to stop a shard-born eclipse from swallowing the last inhabited cities.</outline>
  <lockdata>false</lockdata>
  <dateadded>2026-04-21 04:22:41</dateadded>
  <title>Nightfall!!</title>
  <originaltitle>Nightfall!! Kage no Requiem</originaltitle>
  <trailer>plugin://plugin.video.youtube/play/?video_id=_Iqc-dG8peA</trailer>
  <trailer>plugin://plugin.video.youtube/play/?video_id=Vt4zSf3CfRA</trailer>
  <rating>5</rating>
  <year>2022</year>
  <mpaa>TV-MA</mpaa>
  <collectionnumber>186898</collectionnumber>
  <imdb_id>tt17736234</imdb_id>
  <tmdbid>156898</tmdbid>
  <premiered>1992-08-25</premiered>
  <releasedate>1992-08-25</releasedate>
  <enddate>1993-06-25</enddate>
  <runtime>25</runtime>
  <genre>Anime</genre>
  <genre>magic</genre>
  <genre>stereotypes</genre>
  <genre>super power</genre>
  <genre>violence</genre>
  <studio />
  <studio>Netflix</studio>
  <tag>anime</tag>
  <tag>based on manga</tag>
  <tag>combat</tag>
  <tag>dark fantasy</tag>
  <tag>ecchi</tag>
  <tag>heavy metal</tag>
  <tag>magic</tag>
  <tag>original net animation (ona)</tag>
  <tag>remake</tag>
  <tag>seinen</tag>
  <anidbid>10</anidbid>
  <tvdbid>415677</tvdbid>
  <tvdbslugid>nightfall-2022</tvdbslugid>
  <art>
    <poster>/config/metadata/library/df/df254e34942e2f83823ce24206a65630/poster.jpg</poster>
    <fanart>/config/metadata/library/df/df254e34942e2f83823ce24206a65630/backdrop.jpg</fanart>
  </art>
  <id>415677</id>
  <episodeguide>
    <url cache="415677.xml">http://www.thetvdb.com/api/1D62F2F90030C444/series/415677/all/en.zip</url>
  </episodeguide>
  <season>-1</season>
  <episode>-1</episode>
  <status>Ended</status>
</tvshow>"#
    }
    use chrono::Utc;
    use scryer_domain::{CanonicalMediaTag, ExternalId, MediaFacet};

    fn canonical_genre_tag(key: &str, name: &str) -> CanonicalMediaTag {
        CanonicalMediaTag {
            key: format!("canonical:genre:{key}"),
            category: "genre".to_string(),
            name: name.to_string(),
            confidence: Some(1.0),
            sources: Vec::new(),
            source_tag_keys: Vec::new(),
            is_adult: false,
            is_spoiler: false,
        }
    }

    fn make_title() -> Title {
        Title {
            id: "t1".into(),
            name: "Glass Harbor".into(),
            facet: MediaFacet::Movie,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![
                canonical_genre_tag("action", "Action"),
                canonical_genre_tag("sci-fi", "Sci-Fi"),
            ],
            external_ids: vec![
                ExternalId {
                    source: "tvdb".into(),
                    value: "12345".into(),
                },
                ExternalId {
                    source: "tmdb".into(),
                    value: "603".into(),
                },
            ],
            created_by: None,
            created_at: Utc::now(),
            year: Some(1999),
            overview: Some(
                "A courier uncovers the secret geometry beneath a flooded megacity.".into(),
            ),
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: Some("tt0133093".into()),
            runtime_minutes: Some(136),
            popularity: None,
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: Some("Aurora Gate".into()),
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn make_episode() -> Episode {
        Episode {
            id: "e1".into(),
            title_id: "t1".into(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".into()),
            season_number: Some("1".into()),
            episode_label: None,
            title: Some("Pilot".into()),
            air_date: Some("2008-01-20".into()),
            duration_seconds: Some(3480),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: Some("A high school chemistry teacher gets a diagnosis.".into()),
            tvdb_id: Some("349232".into()),
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_kodi_uniqueid_tvdb() {
        let nfo = r#"<?xml version="1.0" encoding="UTF-8"?>
<movie>
  <title>Glass Harbor</title>
  <uniqueid type="tvdb" default="true">12345</uniqueid>
  <uniqueid type="imdb">tt1160419</uniqueid>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("12345".into()));
        assert_eq!(meta.imdb_id, Some("tt1160419".into()));
        assert_eq!(meta.title, Some("Glass Harbor".into()));
    }

    #[test]
    fn parse_kodi_uniqueid_tmdb() {
        let nfo = r#"<movie>
  <uniqueid type="tmdb">438631</uniqueid>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tmdb_id, Some("438631".into()));
    }

    #[test]
    fn parse_jellyfin_tags() {
        let nfo = r#"<movie>
  <tvdbid>12345</tvdbid>
  <imdbid>tt999888</imdbid>
  <tmdbid>67890</tmdbid>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("12345".into()));
        assert_eq!(meta.imdb_id, Some("tt999888".into()));
        assert_eq!(meta.tmdb_id, Some("67890".into()));
    }

    #[test]
    fn ignore_bare_imdb_id_for_movie_root() {
        let nfo = "<movie><id>tt1234567</id></movie>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, None);
        assert_eq!(meta.tvdb_id, None);
    }

    #[test]
    fn parse_jellyfin_id_attributes() {
        let nfo = r#"<movie><id TMDB="2502" TVDB="842" IMDB="tt0372183">ignored</id></movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tmdb_id, Some("2502".into()));
        assert_eq!(meta.tvdb_id, Some("842".into()));
        assert_eq!(meta.imdb_id, Some("tt0372183".into()));
    }

    #[test]
    fn ignore_bare_numeric_id_for_tvshow_root() {
        let nfo = "<tvshow><id>12345</id></tvshow>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, None);
        assert_eq!(meta.imdb_id, None);
    }

    #[test]
    fn ignore_bare_imdb_id_for_tvshow_root() {
        let nfo = "<tvshow><id>tt0372183</id></tvshow>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, None);
        assert_eq!(meta.imdb_id, None);
    }

    #[test]
    fn parse_legacy_numeric_id_for_movie_root_is_not_authoritative() {
        let nfo = "<movie><id>438631</id></movie>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tmdb_id, None);
        assert_eq!(meta.tvdb_id, None);
    }

    #[test]
    fn parse_episode_details_ids_are_not_title_identity() {
        let nfo = r#"<episodedetails>
  <title>Pilot</title>
  <uniqueid type="tvdb" default="true">349232</uniqueid>
  <uniqueid type="imdb">tt0959621</uniqueid>
  <uniqueid type="tmdb">62085</uniqueid>
  <tvdbid>349232</tvdbid>
  <imdbid>tt0959621</imdbid>
  <tmdbid>62085</tmdbid>
  <id TVDB="349232" IMDB="tt0959621" TMDB="62085">349232</id>
</episodedetails>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.title, Some("Pilot".into()));
        assert_eq!(meta.tvdb_id, None);
        assert_eq!(meta.imdb_id, None);
        assert_eq!(meta.tmdb_id, None);
    }

    #[test]
    fn parse_imdb_underscore_tag() {
        let nfo = "<tvshow><imdb_id>tt1160419</imdb_id></tvshow>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, Some("tt1160419".into()));
    }

    #[test]
    fn parse_imdb_tag_rejects_numeric_only_values() {
        let nfo = "<movie><imdbid>438631</imdbid></movie>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, None);
    }

    #[test]
    fn parse_ignores_nested_provider_ids() {
        let nfo = r#"<movie>
  <title>Outer Movie</title>
  <actor>
    <name>Actor Name</name>
    <imdbid>tt0000001</imdbid>
    <tmdbid>999</tmdbid>
  </actor>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.title, Some("Outer Movie".into()));
        assert_eq!(meta.imdb_id, None);
        assert_eq!(meta.tmdb_id, None);
    }

    #[test]
    fn parse_title_and_year() {
        let nfo = "<movie><title>Movie Name</title><year>2024</year></movie>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.title, Some("Movie Name".into()));
        assert_eq!(meta.year, Some(2024));
    }

    #[test]
    fn parse_year_out_of_range() {
        let nfo = "<movie><year>9999</year></movie>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.year, None);
    }

    #[test]
    fn parse_url_only_imdb() {
        let nfo = "https://www.imdb.com/title/tt1234567/";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, Some("tt1234567".into()));
    }

    #[test]
    fn parse_url_only_tvdb() {
        let nfo = "https://www.thetvdb.com/?tab=movie&id=12345";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("12345".into()));
    }

    #[test]
    fn parse_url_only_tmdb() {
        let nfo = "https://www.themoviedb.org/movie/438631-glass-harbor";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tmdb_id, Some("438631".into()));
    }

    #[test]
    fn parse_url_only_tmdb_tv() {
        let nfo = "https://www.themoviedb.org/tv/94997-house-of-the-dragon";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tmdb_id, Some("94997".into()));
    }

    #[test]
    fn parse_xml_top_level_comment_url() {
        let nfo = r#"<movie><!-- https://www.imdb.com/title/tt1234567/ --></movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, Some("tt1234567".into()));
    }

    #[test]
    fn parse_xml_body_text_url() {
        let nfo =
            r#"<movie><plot>See https://www.imdb.com/title/tt1234567/ for details.</plot></movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, Some("tt1234567".into()));
    }

    #[test]
    fn parse_xml_explicit_provider_tag_overrides_comment_url() {
        let nfo = r#"<movie>
  <!-- https://www.imdb.com/title/tt0000001/ -->
  <imdbid>tt9999999</imdbid>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, Some("tt9999999".into()));
    }

    #[test]
    fn parse_xml_ignores_nested_comment_url() {
        let nfo = r#"<movie>
  <actor>
    <name>Actor Name</name>
    <!-- https://www.imdb.com/title/tt0000001/ -->
  </actor>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, None);
    }

    #[test]
    fn parse_xml_does_not_scan_nested_text_urls() {
        let nfo = r#"<movie>
  <actor>
    <name>https://www.themoviedb.org/movie/999-nested-person-url</name>
  </actor>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tmdb_id, None);
    }

    #[test]
    fn parse_plexmatch_provider_ids_and_guid() {
        let meta = parse_plexmatch(
            r#"
# comment
Show: Example Show
Year: 2024
Guid: imdb://tt1160419
tmdbid: 438631
tvdbid: 12345
bad line
"#,
        );
        assert_eq!(meta.title, Some("Example Show".into()));
        assert_eq!(meta.year, Some(2024));
        assert_eq!(meta.imdb_id, Some("tt1160419".into()));
        assert_eq!(meta.tmdb_id, Some("438631".into()));
        assert_eq!(meta.tvdb_id, Some("12345".into()));
    }

    #[test]
    fn parse_plexmatch_matches_plex_sonarr_series_shape() {
        let meta = parse_plexmatch(
            r#"
Title: Example Show
Year: 2024
TvdbId: 12345
ImdbId: 1160419
Episode: S01E01: Season 01/Pilot.mkv
Pattern: Bonus/Bonus {sp,1-3,+4}.mp4
"#,
        );
        assert_eq!(meta.title, Some("Example Show".into()));
        assert_eq!(meta.year, Some(2024));
        assert_eq!(meta.tvdb_id, Some("12345".into()));
        assert_eq!(meta.imdb_id, Some("tt1160419".into()));
        assert_eq!(meta.tmdb_id, None);
    }

    #[test]
    fn parse_plexmatch_ignores_unknown_guid() {
        let meta = parse_plexmatch("guid: plex://show/5d9c088e705e7d001f32b8f8");
        assert_eq!(meta, NfoMetadata::default());
    }

    #[test]
    fn parse_empty_content() {
        let meta = parse_nfo("");
        assert_eq!(meta, NfoMetadata::default());
    }

    #[test]
    fn parse_whitespace_only() {
        let meta = parse_nfo("   \n\t  ");
        assert_eq!(meta, NfoMetadata::default());
    }

    #[test]
    fn parse_binary_junk() {
        let meta = parse_nfo("\x00\x01\x02 random garbage 🎬");
        assert_eq!(meta, NfoMetadata::default());
    }

    #[test]
    fn parse_full_movie_nfo() {
        let nfo = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>
<movie>
  <title>Glass Harbor</title>
  <year>1999</year>
  <plot>A computer hacker learns about reality.</plot>
  <runtime>136</runtime>
  <genre>Action</genre>
  <genre>Sci-Fi</genre>
  <studio>Warner Bros.</studio>
  <uniqueid type="tvdb" default="true">12345</uniqueid>
  <uniqueid type="imdb">tt0133093</uniqueid>
  <uniqueid type="tmdb">603</uniqueid>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("12345".into()));
        assert_eq!(meta.imdb_id, Some("tt0133093".into()));
        assert_eq!(meta.tmdb_id, Some("603".into()));
        assert_eq!(meta.title, Some("Glass Harbor".into()));
        assert_eq!(meta.year, Some(1999));
    }

    #[test]
    fn parse_tvshow_nfo() {
        let nfo = r#"<tvshow>
  <title>Neon Divide</title>
  <year>2008</year>
  <uniqueid type="tvdb" default="true">81189</uniqueid>
</tvshow>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("81189".into()));
        assert_eq!(meta.title, Some("Neon Divide".into()));
        assert_eq!(meta.year, Some(2008));
    }

    #[test]
    fn parse_jellyfin_nightfall_tvshow_nfo() {
        let meta = parse_nfo(nightfall_tvshow_nfo());
        assert_eq!(meta.title.as_deref(), Some("Nightfall!!"));
        assert_eq!(meta.year, Some(2022));
        assert_eq!(meta.tvdb_id.as_deref(), Some("415677"));
        assert_eq!(meta.tmdb_id.as_deref(), Some("156898"));
    }

    #[test]
    fn parse_jellyfin_nightfall_tvshow_nfo_with_utf8_bom() {
        let prefixed = format!("\u{feff}{}", nightfall_tvshow_nfo());
        let meta = parse_nfo(&prefixed);
        assert_eq!(meta.title.as_deref(), Some("Nightfall!!"));
        assert_eq!(meta.year, Some(2022));
        assert_eq!(meta.tvdb_id.as_deref(), Some("415677"));
    }

    #[test]
    fn parse_episode_nfo() {
        let nfo = r#"<episodedetails>
  <title>Pilot</title>
  <season>1</season>
  <episode>1</episode>
  <uniqueid type="tvdb" default="true">349232</uniqueid>
</episodedetails>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, None);
        assert_eq!(meta.title, Some("Pilot".into()));
    }

    #[test]
    fn detect_movie_nfo_root_kind() {
        assert_eq!(
            detect_nfo_root_kind(r#"<movie><title>Glass Harbor</title></movie>"#),
            NfoRootKind::Movie
        );
        assert!(looks_like_movie_nfo(
            r#"<movie><title>Glass Harbor</title></movie>"#
        ));
    }

    #[test]
    fn reject_tvshow_and_episode_nfo_for_movie_detection() {
        assert_eq!(
            detect_nfo_root_kind(r#"<tvshow><title>Harbor Pals</title></tvshow>"#),
            NfoRootKind::TvShow
        );
        assert_eq!(
            detect_nfo_root_kind(r#"<episodedetails><title>Pilot</title></episodedetails>"#),
            NfoRootKind::Episode
        );
        assert!(!looks_like_movie_nfo(
            r#"<tvshow><title>Harbor Pals</title></tvshow>"#
        ));
        assert!(!looks_like_movie_nfo(
            r#"<episodedetails><title>Pilot</title></episodedetails>"#
        ));
    }

    #[test]
    fn detect_tvshow_root_kind_accepts_utf8_bom() {
        let prefixed = format!("\u{feff}{}", nightfall_tvshow_nfo());
        assert_eq!(detect_nfo_root_kind(&prefixed), NfoRootKind::TvShow);
    }

    #[test]
    fn parse_uniqueid_priority_over_legacy() {
        let nfo = r#"<movie>
  <id>99999</id>
  <uniqueid type="tvdb">12345</uniqueid>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("12345".into()));
    }

    #[test]
    fn parse_url_in_xml_nfo() {
        let nfo = r#"<movie>
  <title>Test</title>
  <!-- https://www.imdb.com/title/tt9876543/ -->
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, Some("tt9876543".into()));
        assert_eq!(meta.title, Some("Test".into()));
    }

    #[test]
    fn parse_ignores_unknown_elements() {
        let nfo = r#"<movie>
  <title>Test</title>
  <originaltitle>Original Test</originaltitle>
  <sorttitle>test</sorttitle>
  <rating>8.5</rating>
  <votes>12345</votes>
  <top250>42</top250>
  <outline>Short outline</outline>
  <tagline>Some tagline</tagline>
  <director>John Doe</director>
  <credits>Jane Writer</credits>
  <set><name>Test Collection</name></set>
  <thumb aspect="poster">http://example.com/poster.jpg</thumb>
  <fanart><thumb>http://example.com/fanart.jpg</thumb></fanart>
  <certification>PG-13</certification>
  <country>US</country>
  <premiered>2024-01-01</premiered>
  <fileinfo><streamdetails><video><codec>h264</codec></video></streamdetails></fileinfo>
  <uniqueid type="tvdb">99999</uniqueid>
  <year>2024</year>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.title, Some("Test".into()));
        assert_eq!(meta.tvdb_id, Some("99999".into()));
        assert_eq!(meta.year, Some(2024));
    }

    #[test]
    fn parse_xml_with_ampersand_entities() {
        let nfo = r#"<movie><title>Tom &amp; Jerry</title></movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.title, Some("Tom & Jerry".into()));
    }

    // -----------------------------------------------------------------------
    // Writer tests
    // -----------------------------------------------------------------------

    #[test]
    fn render_movie_full() {
        let title = make_title();
        let xml = render_movie_nfo(&title);
        assert!(xml.contains("<?xml"));
        assert!(xml.contains("<movie>"));
        assert!(xml.contains("<title>Glass Harbor</title>"));
        assert!(xml.contains("<year>1999</year>"));
        assert!(xml.contains("<plot>A courier uncovers the secret geometry"));
        assert!(xml.contains("<runtime>136</runtime>"));
        assert!(xml.contains("<genre>Action</genre>"));
        assert!(xml.contains("<genre>Sci-Fi</genre>"));
        assert!(xml.contains("<studio>Aurora Gate</studio>"));
        assert!(xml.contains(r#"<uniqueid type="tmdb" default="true">603</uniqueid>"#));
        assert!(xml.contains(r#"<uniqueid type="imdb">tt0133093</uniqueid>"#));
        assert!(xml.contains(r#"<uniqueid type="tvdb">12345</uniqueid>"#));
        assert!(xml.contains("<tmdbid>603</tmdbid>"));
        assert!(xml.contains("<imdbid>tt0133093</imdbid>"));
        assert!(!xml.contains(r#"<uniqueid type="tvdb" default="true">"#));
        assert!(!xml.contains("<id>"));
        assert!(xml.contains("</movie>"));
    }

    #[test]
    fn render_tvshow_full() {
        let mut title = make_title();
        title.network = Some("AMC".into());
        title.studio = None;
        let xml = render_tvshow_nfo(&title);
        assert!(xml.contains("<tvshow>"));
        assert!(xml.contains("<studio>AMC</studio>"));
        assert!(xml.contains(r#"<uniqueid type="tvdb" default="true">12345</uniqueid>"#));
        assert!(xml.contains(r#"<uniqueid type="tmdb">603</uniqueid>"#));
        assert!(xml.contains("<tvdbid>12345</tvdbid>"));
        assert!(xml.contains("<tmdbid>603</tmdbid>"));
        assert!(xml.contains("<imdb_id>tt0133093</imdb_id>"));
        assert!(xml.contains("</tvshow>"));
    }

    #[test]
    fn render_episode_full() {
        let title = make_title();
        let episode = make_episode();
        let xml = render_episode_nfo(&title, &episode);
        assert!(xml.contains("<episodedetails>"));
        assert!(xml.contains("<showtitle>Glass Harbor</showtitle>"));
        assert!(xml.contains("<title>Pilot</title>"));
        assert!(xml.contains("<season>1</season>"));
        assert!(xml.contains("<episode>1</episode>"));
        assert!(xml.contains("<aired>2008-01-20</aired>"));
        assert!(xml.contains("<runtime>58</runtime>"));
        assert!(xml.contains(r#"<uniqueid type="tvdb" default="true">349232</uniqueid>"#));
        assert!(xml.contains("</episodedetails>"));
    }

    #[test]
    fn render_movie_xml_escapes_special_chars() {
        let mut title = make_title();
        title.name = "Tom & Jerry <3".into();
        let xml = render_movie_nfo(&title);
        assert!(xml.contains("<title>Tom &amp; Jerry &lt;3</title>"));
    }

    #[test]
    fn render_plexmatch() {
        let title = make_title();
        let plex = super::render_plexmatch(&title);
        assert!(plex.contains("Title: Glass Harbor"));
        assert!(plex.contains("Year: 1999"));
        assert!(plex.contains("TvdbId: 12345"));
        assert!(plex.contains("ImdbId: tt0133093"));
        assert!(plex.contains("TmdbId: 603"));
        assert!(!plex.contains("Movie:"));
    }

    #[test]
    #[ignore = "diagnostic harness for local mounted media roots"]
    fn profile_real_media_root_nfo_parsing() {
        let roots = std::env::var("SCRYER_NFO_PROFILE_ROOTS").unwrap_or_else(|_| {
            "/Volumes/Media/Movies:/Volumes/Media/Anime:/Volumes/Media/TV".to_string()
        });
        let limit = std::env::var("SCRYER_NFO_PROFILE_LIMIT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok());

        for root in roots.split(':').filter(|root| !root.trim().is_empty()) {
            let root_path = PathBuf::from(root);
            if !root_path.is_dir() {
                eprintln!("NFO_ROOT\t{}\tmissing", root_path.display());
                continue;
            }

            let mut entries = stdfs::read_dir(&root_path)
                .expect("read root")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            entries.sort();
            if let Some(limit) = limit {
                entries.truncate(limit);
            }

            let nfo_name = if root_path.ends_with("Movies") {
                "movie.nfo"
            } else {
                "tvshow.nfo"
            };
            profile_nfo_entries(&root_path, &entries, nfo_name);
        }
    }

    #[derive(Clone)]
    struct NfoProfileRow {
        path: PathBuf,
        bytes: usize,
        read_ms: u128,
        parse_ms: u128,
        has_tvdb: bool,
        has_imdb: bool,
        has_tmdb: bool,
    }

    fn profile_nfo_entries(root: &Path, entries: &[PathBuf], nfo_name: &str) {
        let mut rows = Vec::new();
        let mut missing = 0usize;
        let mut total_read_ms = 0u128;
        let mut total_parse_ms = 0u128;
        let mut id_count = 0usize;

        for entry in entries {
            let nfo_path = entry.join(nfo_name);
            let read_started = Instant::now();
            let content = match stdfs::read_to_string(&nfo_path) {
                Ok(content) => content,
                Err(_) => {
                    missing = missing.saturating_add(1);
                    continue;
                }
            };
            let read_ms = read_started.elapsed().as_millis();
            total_read_ms = total_read_ms.saturating_add(read_ms);

            let parse_started = Instant::now();
            let parsed = parse_nfo(&content);
            let parse_ms = parse_started.elapsed().as_millis();
            total_parse_ms = total_parse_ms.saturating_add(parse_ms);
            if parsed.has_external_ids() {
                id_count = id_count.saturating_add(1);
            }

            rows.push(NfoProfileRow {
                path: nfo_path,
                bytes: content.len(),
                read_ms,
                parse_ms,
                has_tvdb: parsed.tvdb_id.is_some(),
                has_imdb: parsed.imdb_id.is_some(),
                has_tmdb: parsed.tmdb_id.is_some(),
            });
        }

        eprintln!(
            "NFO_SUMMARY\troot={}\tentries={}\tparsed={}\tmissing={}\tids={}\tread_total_ms={}\tparse_total_ms={}",
            root.display(),
            entries.len(),
            rows.len(),
            missing,
            id_count,
            total_read_ms,
            total_parse_ms
        );
        print_slowest_nfo_rows("NFO_READ_SLOW", &rows, |row| row.read_ms);
        print_slowest_nfo_rows("NFO_PARSE_SLOW", &rows, |row| row.parse_ms);
    }

    fn print_slowest_nfo_rows(
        label: &str,
        rows: &[NfoProfileRow],
        elapsed: impl Fn(&NfoProfileRow) -> u128,
    ) {
        let mut rows = rows.to_vec();
        rows.sort_by_key(|row| cmp::Reverse(elapsed(row)));
        for row in rows.into_iter().take(12) {
            eprintln!(
                "{}\tms={}\tread_ms={}\tparse_ms={}\tbytes={}\ttvdb={}\timdb={}\ttmdb={}\tpath={}",
                label,
                elapsed(&row),
                row.read_ms,
                row.parse_ms,
                row.bytes,
                row.has_tvdb,
                row.has_imdb,
                row.has_tmdb,
                row.path.display()
            );
        }
    }
}
