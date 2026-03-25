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
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return NfoMetadata::default();
    }

    let mut meta = NfoMetadata::default();

    if trimmed.starts_with('<') {
        parse_xml_nfo(content, &mut meta);
    }

    // URL fallback (works for both XML and plain-text NFO files)
    if meta.imdb_id.is_none() {
        meta.imdb_id = extract_imdb_url_id(content);
    }
    if meta.tvdb_id.is_none() {
        meta.tvdb_id = extract_tvdb_url_id(content);
    }
    if meta.tmdb_id.is_none() {
        meta.tmdb_id = extract_tmdb_url_id(content);
    }

    meta
}

fn parse_xml_nfo(content: &str, meta: &mut NfoMetadata) {
    let mut reader = Reader::from_str(content);

    let mut current_tag = String::new();
    let mut uniqueid_type: Option<String> = None;

    // Legacy <id> is lowest priority — only used if uniqueid/jellyfin tags don't
    // provide the same ID. Defer until after the full parse.
    let mut legacy_id: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                current_tag = name.clone();

                if name == "uniqueid" {
                    uniqueid_type = e
                        .attributes()
                        .filter_map(|a| a.ok())
                        .find(|a| a.key.as_ref() == b"type")
                        .and_then(|a| String::from_utf8(a.value.to_vec()).ok())
                        .map(|v| v.to_lowercase());
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e
                    .unescape()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                if text.is_empty() {
                    continue;
                }

                match current_tag.as_str() {
                    "uniqueid" => {
                        if let Some(ref uid_type) = uniqueid_type {
                            match uid_type.as_str() {
                                "tvdb" if meta.tvdb_id.is_none() => {
                                    meta.tvdb_id = Some(text);
                                }
                                "imdb" if meta.imdb_id.is_none() => {
                                    meta.imdb_id = normalize_imdb(&text);
                                }
                                "tmdb" if meta.tmdb_id.is_none() => {
                                    if looks_like_numeric_id(&text) {
                                        meta.tmdb_id = Some(text);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "tvdbid" if meta.tvdb_id.is_none() => {
                        if looks_like_numeric_id(&text) {
                            meta.tvdb_id = Some(text);
                        }
                    }
                    "imdbid" if meta.imdb_id.is_none() => {
                        meta.imdb_id = normalize_imdb(&text);
                    }
                    "tmdbid" if meta.tmdb_id.is_none() => {
                        if looks_like_numeric_id(&text) {
                            meta.tmdb_id = Some(text);
                        }
                    }
                    "id" if legacy_id.is_none() => {
                        legacy_id = Some(text);
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
            Ok(Event::End(_)) => {
                current_tag.clear();
                uniqueid_type = None;
            }
            Ok(Event::Eof) => break,
            Err(_) => break, // graceful on malformed XML
            _ => {}
        }
    }

    // Apply legacy <id> only if higher-priority tags didn't provide the value.
    if let Some(id_val) = legacy_id {
        if id_val.starts_with("tt") && meta.imdb_id.is_none() {
            meta.imdb_id = normalize_imdb(&id_val);
        } else if looks_like_numeric_id(&id_val) && meta.tvdb_id.is_none() {
            meta.tvdb_id = Some(id_val);
        }
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Render a Kodi-compatible `<movie>` NFO for the given Title.
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
    for genre in &title.genres {
        if !genre.is_empty() {
            write_element(&mut w, "genre", genre);
        }
    }
    write_optional_non_empty_element(&mut w, "studio", title.studio.as_deref());

    write_uniqueids(&mut w, title);

    w.write_event(Event::End(BytesEnd::new("movie"))).ok();
    finish_xml(buf)
}

/// Render a Kodi-compatible `<tvshow>` NFO for the given series Title.
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
    for genre in &title.genres {
        if !genre.is_empty() {
            write_element(&mut w, "genre", genre);
        }
    }
    write_optional_non_empty_element(&mut w, "studio", title.network.as_deref());

    write_uniqueids(&mut w, title);

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

/// Render a Kodi/Jellyfin-compatible `<episodedetails>` NFO for an interstitial anime movie.
///
/// Written as a season 0 special so media servers recognize it as part of the series.
/// Includes `<airsbefore_season>` for Jellyfin's "Display specials within seasons" feature.
pub(crate) fn render_interstitial_movie_nfo(
    movie: &scryer_domain::InterstitialMovieMetadata,
    season_episode: &str,
    collection_index: &str,
) -> String {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    write_xml_decl(&mut w);
    let tag = BytesStart::new("episodedetails");
    w.write_event(Event::Start(tag)).ok();

    write_element(&mut w, "title", &movie.name);
    write_element(&mut w, "season", "0");

    if let Some(ep_str) = season_episode.strip_prefix("S00E")
        && let Ok(ep_num) = ep_str.parse::<i32>()
    {
        write_element(&mut w, "episode", &ep_num.to_string());
    }

    if !movie.overview.is_empty() {
        write_element(&mut w, "plot", &movie.overview);
    }
    if let Some(ref release_date) = movie.digital_release_date {
        write_element(&mut w, "aired", release_date);
    }
    if movie.runtime_minutes > 0 {
        write_element(&mut w, "runtime", &movie.runtime_minutes.to_string());
    }

    if let Some(airs_before) = airs_before_season_from_collection_index(collection_index) {
        write_element(&mut w, "airsbefore_season", &airs_before.to_string());
        write_element(&mut w, "airsbefore_episode", "1");
    }

    if !movie.tvdb_id.is_empty() {
        write_uniqueid(&mut w, "tvdb", &movie.tvdb_id, true);
    }
    if !movie.imdb_id.is_empty() {
        write_uniqueid(&mut w, "imdb", &movie.imdb_id, false);
    }
    if let Some(ref tmdb_id) = movie.movie_tmdb_id {
        write_uniqueid(&mut w, "tmdb", tmdb_id, false);
    }

    w.write_event(Event::End(BytesEnd::new("episodedetails")))
        .ok();
    finish_xml(buf)
}

/// Derive the Jellyfin `airsbefore_season` value from a collection index.
///
/// Collection index "1.1" means the movie airs after season 1, so it should display
/// before season 2. Returns `Some(2)` for "1.1", `Some(3)` for "2.1", etc.
fn airs_before_season_from_collection_index(index: &str) -> Option<i32> {
    let before_dot = index.split('.').next()?;
    let after_season: i32 = before_dot.parse().ok()?;
    Some(after_season + 1)
}

/// Render a Plex `.plexmatch` hint file for the given series Title.
///
/// Plain text key-value format. Lines are omitted when the value is empty.
/// Only applicable to TV series — Plex does not support `.plexmatch` for movies.
pub(crate) fn render_plexmatch(title: &Title) -> String {
    let mut out = format!("title: {}\n", title.name);

    if let Some(year) = title.year {
        out.push_str(&format!("year: {year}\n"));
    }

    push_optional_non_empty_line(&mut out, "tvdbid", title_external_id_value(title, "tvdb"));
    push_optional_non_empty_line(&mut out, "imdbid", title.imdb_id.as_deref());
    push_optional_non_empty_line(&mut out, "tmdbid", title_external_id_value(title, "tmdb"));

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

/// Normalize a raw string to a canonical IMDb ID (tt-prefixed, 7+ digits).
fn normalize_imdb(raw: &str) -> Option<String> {
    let s = raw.trim().trim_matches('"').trim();
    if s.starts_with("tt") && s.len() > 2 {
        Some(s.to_string())
    } else {
        None
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

/// Extract TMDB ID from URL pattern: `themoviedb.org/movie/(\d+)`
fn extract_tmdb_url_id(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let marker = "themoviedb.org/movie/";
    let pos = lower.find(marker)? + marker.len();
    let digits: String = content[pos..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
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

fn write_uniqueids<W: std::io::Write>(w: &mut Writer<W>, title: &Title) {
    if let Some(tvdb_id) = title_external_id_value(title, "tvdb") {
        write_uniqueid(w, "tvdb", tvdb_id, true);
    }
    if let Some(imdb_id) = title.imdb_id.as_deref().filter(|imdb| !imdb.is_empty()) {
        write_uniqueid(w, "imdb", imdb_id, false);
    }
    if let Some(tmdb_id) = title_external_id_value(title, "tmdb") {
        write_uniqueid(w, "tmdb", tmdb_id, false);
    }
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
    use chrono::Utc;
    use scryer_domain::{ExternalId, MediaFacet};

    fn make_title() -> Title {
        Title {
            id: "t1".into(),
            name: "The Matrix".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
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
            overview: Some("A computer hacker learns about the true nature of reality.".into()),
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: Some("tt0133093".into()),
            runtime_minutes: Some(136),
            genres: vec!["Action".into(), "Sci-Fi".into()],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: Some("Warner Bros.".into()),
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
  <title>Dune</title>
  <uniqueid type="tvdb" default="true">12345</uniqueid>
  <uniqueid type="imdb">tt1160419</uniqueid>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("12345".into()));
        assert_eq!(meta.imdb_id, Some("tt1160419".into()));
        assert_eq!(meta.title, Some("Dune".into()));
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
    fn parse_legacy_id_imdb() {
        let nfo = "<movie><id>tt1234567</id></movie>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, Some("tt1234567".into()));
        assert_eq!(meta.tvdb_id, None);
    }

    #[test]
    fn parse_legacy_id_tvdb() {
        let nfo = "<movie><id>12345</id></movie>";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("12345".into()));
        assert_eq!(meta.imdb_id, None);
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
        let nfo = "https://www.themoviedb.org/movie/438631-dune";
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tmdb_id, Some("438631".into()));
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
  <title>The Matrix</title>
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
        assert_eq!(meta.title, Some("The Matrix".into()));
        assert_eq!(meta.year, Some(1999));
    }

    #[test]
    fn parse_tvshow_nfo() {
        let nfo = r#"<tvshow>
  <title>Breaking Bad</title>
  <year>2008</year>
  <uniqueid type="tvdb" default="true">81189</uniqueid>
</tvshow>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("81189".into()));
        assert_eq!(meta.title, Some("Breaking Bad".into()));
        assert_eq!(meta.year, Some(2008));
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
        assert_eq!(meta.tvdb_id, Some("349232".into()));
        assert_eq!(meta.title, Some("Pilot".into()));
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
        assert!(xml.contains("<title>The Matrix</title>"));
        assert!(xml.contains("<year>1999</year>"));
        assert!(xml.contains("<plot>A computer hacker"));
        assert!(xml.contains("<runtime>136</runtime>"));
        assert!(xml.contains("<genre>Action</genre>"));
        assert!(xml.contains("<genre>Sci-Fi</genre>"));
        assert!(xml.contains("<studio>Warner Bros.</studio>"));
        assert!(xml.contains(r#"<uniqueid type="tvdb" default="true">12345</uniqueid>"#));
        assert!(xml.contains(r#"<uniqueid type="imdb">tt0133093</uniqueid>"#));
        assert!(xml.contains(r#"<uniqueid type="tmdb">603</uniqueid>"#));
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
        assert!(xml.contains("</tvshow>"));
    }

    #[test]
    fn render_episode_full() {
        let title = make_title();
        let episode = make_episode();
        let xml = render_episode_nfo(&title, &episode);
        assert!(xml.contains("<episodedetails>"));
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
        assert!(plex.contains("title: The Matrix"));
        assert!(plex.contains("year: 1999"));
        assert!(plex.contains("tvdbid: 12345"));
        assert!(plex.contains("imdbid: tt0133093"));
        assert!(plex.contains("tmdbid: 603"));
    }
}
