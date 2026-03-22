use scryer_domain::{Episode, Title};

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
/// All failures are graceful — unparseable content returns `NfoMetadata::default()`.
pub(crate) fn parse_nfo(content: &str) -> NfoMetadata {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return NfoMetadata::default();
    }

    let is_xml = trimmed.starts_with('<');

    let mut meta = NfoMetadata::default();

    if is_xml {
        // Kodi v17+ uniqueid tags (highest priority)
        meta.tvdb_id = extract_uniqueid(content, "tvdb");
        meta.imdb_id = extract_uniqueid(content, "imdb").and_then(|v| normalize_imdb(&v));
        meta.tmdb_id = extract_uniqueid(content, "tmdb").filter(|v| looks_like_numeric_id(v));

        // Jellyfin/Emby direct tags
        if meta.tvdb_id.is_none() {
            meta.tvdb_id = extract_element(content, "tvdbid").filter(|v| looks_like_numeric_id(v));
        }
        if meta.imdb_id.is_none() {
            meta.imdb_id = extract_element(content, "imdbid").and_then(|v| normalize_imdb(&v));
        }
        if meta.tmdb_id.is_none() {
            meta.tmdb_id = extract_element(content, "tmdbid").filter(|v| looks_like_numeric_id(v));
        }

        // Legacy <id> tag — IMDb if starts with "tt", TVDB if pure numeric
        if meta.tvdb_id.is_none()
            && meta.imdb_id.is_none()
            && let Some(id_val) = extract_element(content, "id")
        {
            let id_trimmed = id_val.trim();
            if id_trimmed.starts_with("tt") {
                meta.imdb_id = normalize_imdb(id_trimmed);
            } else if looks_like_numeric_id(id_trimmed) {
                meta.tvdb_id = Some(id_trimmed.to_string());
            }
        }

        // Title and year
        meta.title = extract_element(content, "title");
        meta.year = extract_element(content, "year")
            .and_then(|v| v.trim().parse::<i32>().ok())
            .filter(|&y| (1888..=2100).contains(&y));
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

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Render a Kodi-compatible `<movie>` NFO for the given Title.
pub(crate) fn render_movie_nfo(title: &Title) -> String {
    let mut out =
        String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>\n<movie>\n");

    push_element(&mut out, "title", &title.name);

    if let Some(year) = title.year {
        push_element(&mut out, "year", &year.to_string());
    }
    if let Some(ref overview) = title.overview
        && !overview.is_empty()
    {
        push_element(&mut out, "plot", overview);
    }
    if let Some(runtime) = title.runtime_minutes
        && runtime > 0
    {
        push_element(&mut out, "runtime", &runtime.to_string());
    }
    for genre in &title.genres {
        if !genre.is_empty() {
            push_element(&mut out, "genre", genre);
        }
    }
    if let Some(ref studio) = title.studio
        && !studio.is_empty()
    {
        push_element(&mut out, "studio", studio);
    }

    push_uniqueids(&mut out, title);

    out.push_str("</movie>\n");
    out
}

/// Render a Kodi-compatible `<tvshow>` NFO for the given series Title.
pub(crate) fn render_tvshow_nfo(title: &Title) -> String {
    let mut out =
        String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>\n<tvshow>\n");

    push_element(&mut out, "title", &title.name);

    if let Some(year) = title.year {
        push_element(&mut out, "year", &year.to_string());
    }
    if let Some(ref overview) = title.overview
        && !overview.is_empty()
    {
        push_element(&mut out, "plot", overview);
    }
    for genre in &title.genres {
        if !genre.is_empty() {
            push_element(&mut out, "genre", genre);
        }
    }
    if let Some(ref network) = title.network
        && !network.is_empty()
    {
        push_element(&mut out, "studio", network);
    }

    push_uniqueids(&mut out, title);

    out.push_str("</tvshow>\n");
    out
}

/// Render a Kodi-compatible `<episodedetails>` NFO.
pub(crate) fn render_episode_nfo(title: &Title, episode: &Episode) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>\n<episodedetails>\n",
    );

    if let Some(ref ep_title) = episode.title
        && !ep_title.is_empty()
    {
        push_element(&mut out, "title", ep_title);
    }
    if let Some(ref season) = episode.season_number {
        push_element(&mut out, "season", season);
    }
    if let Some(ref ep_num) = episode.episode_number {
        push_element(&mut out, "episode", ep_num);
    }
    if let Some(ref overview) = episode.overview
        && !overview.is_empty()
    {
        push_element(&mut out, "plot", overview);
    }
    if let Some(ref air_date) = episode.air_date
        && !air_date.is_empty()
    {
        push_element(&mut out, "aired", air_date);
    }
    if let Some(duration_secs) = episode.duration_seconds {
        let minutes = duration_secs / 60;
        if minutes > 0 {
            push_element(&mut out, "runtime", &minutes.to_string());
        }
    }

    // Episode-level uniqueid: prefer the episode's own TVDB ID, fall back to series TVDB ID
    if let Some(tvdb_id) = &episode.tvdb_id {
        out.push_str(&format!(
            "  <uniqueid type=\"tvdb\" default=\"true\">{}</uniqueid>\n",
            xml_escape(tvdb_id)
        ));
    } else if let Some(eid) = title
        .external_ids
        .iter()
        .find(|e| e.source.eq_ignore_ascii_case("tvdb"))
        && !eid.value.is_empty()
    {
        out.push_str(&format!(
            "  <uniqueid type=\"tvdb\" default=\"true\">{}</uniqueid>\n",
            xml_escape(&eid.value)
        ));
    }

    out.push_str("</episodedetails>\n");
    out
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
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>\n<episodedetails>\n",
    );

    push_element(&mut out, "title", &movie.name);
    push_element(&mut out, "season", "0");

    // Extract episode number from "S00E03" format
    if let Some(ep_str) = season_episode.strip_prefix("S00E")
        && let Ok(ep_num) = ep_str.parse::<i32>()
    {
        push_element(&mut out, "episode", &ep_num.to_string());
    }

    if !movie.overview.is_empty() {
        push_element(&mut out, "plot", &movie.overview);
    }
    if let Some(ref release_date) = movie.digital_release_date {
        push_element(&mut out, "aired", release_date);
    }
    if movie.runtime_minutes > 0 {
        push_element(&mut out, "runtime", &movie.runtime_minutes.to_string());
    }

    // airsbefore_season: collection_index "1.1" means after season 1 → airs before season 2
    if let Some(airs_before) = airs_before_season_from_collection_index(collection_index) {
        push_element(&mut out, "airsbefore_season", &airs_before.to_string());
        push_element(&mut out, "airsbefore_episode", "1");
    }

    // Unique IDs from the movie metadata
    if !movie.tvdb_id.is_empty() {
        out.push_str(&format!(
            "  <uniqueid type=\"tvdb\" default=\"true\">{}</uniqueid>\n",
            xml_escape(&movie.tvdb_id)
        ));
    }
    if !movie.imdb_id.is_empty() {
        out.push_str(&format!(
            "  <uniqueid type=\"imdb\">{}</uniqueid>\n",
            xml_escape(&movie.imdb_id)
        ));
    }
    if let Some(ref tmdb_id) = movie.movie_tmdb_id {
        out.push_str(&format!(
            "  <uniqueid type=\"tmdb\">{}</uniqueid>\n",
            xml_escape(tmdb_id)
        ));
    }

    out.push_str("</episodedetails>\n");
    out
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

    if let Some(eid) = title
        .external_ids
        .iter()
        .find(|e| e.source.eq_ignore_ascii_case("tvdb"))
        && !eid.value.is_empty()
    {
        out.push_str(&format!("tvdbid: {}\n", eid.value));
    }

    if let Some(ref imdb_id) = title.imdb_id
        && !imdb_id.is_empty()
    {
        out.push_str(&format!("imdbid: {imdb_id}\n"));
    }

    if let Some(eid) = title
        .external_ids
        .iter()
        .find(|e| e.source.eq_ignore_ascii_case("tmdb"))
        && !eid.value.is_empty()
    {
        out.push_str(&format!("tmdbid: {}\n", eid.value));
    }

    out
}

// ---------------------------------------------------------------------------
// Helpers — parser
// ---------------------------------------------------------------------------

/// Extract `<uniqueid type="TYPE_VAL">TEXT</uniqueid>` (case-insensitive).
fn extract_uniqueid(content: &str, type_val: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let needle = format!("type=\"{}\"", type_val.to_ascii_lowercase());

    // There may be multiple uniqueid tags; find the one with the right type.
    let mut search_from = 0;
    while let Some(attr_pos) = lower[search_from..].find(&needle) {
        let abs_attr_pos = search_from + attr_pos;

        // Scan backward for the opening <uniqueid
        let before = &lower[..abs_attr_pos];
        if let Some(tag_start) = before.rfind("<uniqueid") {
            // Find the > that closes the opening tag
            let rest = &content[tag_start..];
            if let Some(open_end) = rest.find('>') {
                let text_start = tag_start + open_end + 1;
                if let Some(close_pos) = lower[text_start..].find("</uniqueid>") {
                    let value = content[text_start..text_start + close_pos].trim();
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
        search_from = abs_attr_pos + needle.len();
    }
    None
}

/// Extract text content of the first `<TAG>TEXT</TAG>` (case-insensitive).
fn extract_element(content: &str, tag: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let open = format!("<{}>", tag.to_ascii_lowercase());
    let close = format!("</{}>", tag.to_ascii_lowercase());
    let start = lower.find(&open)? + open.len();
    let end = lower[start..].find(&close)? + start;
    let value = content[start..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

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

/// Extract TVDB ID from URL pattern: `thetvdb.com/...id=(\d+)` or `thetvdb.com/?tab=...&id=(\d+)`
fn extract_tvdb_url_id(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let domain_pos = lower.find("thetvdb.com")?;
    let after = &lower[domain_pos..];
    // Look for id= parameter
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

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn push_element(out: &mut String, tag: &str, value: &str) {
    out.push_str(&format!("  <{tag}>{}</{tag}>\n", xml_escape(value)));
}

fn push_uniqueids(out: &mut String, title: &Title) {
    if let Some(eid) = title
        .external_ids
        .iter()
        .find(|e| e.source.eq_ignore_ascii_case("tvdb"))
        && !eid.value.is_empty()
    {
        out.push_str(&format!(
            "  <uniqueid type=\"tvdb\" default=\"true\">{}</uniqueid>\n",
            xml_escape(&eid.value)
        ));
    }
    if let Some(ref imdb) = title.imdb_id
        && !imdb.is_empty()
    {
        out.push_str(&format!(
            "  <uniqueid type=\"imdb\">{}</uniqueid>\n",
            xml_escape(imdb)
        ));
    }
    if let Some(eid) = title
        .external_ids
        .iter()
        .find(|e| e.source.eq_ignore_ascii_case("tmdb"))
        && !eid.value.is_empty()
    {
        out.push_str(&format!(
            "  <uniqueid type=\"tmdb\">{}</uniqueid>\n",
            xml_escape(&eid.value)
        ));
    }
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
        // When both <uniqueid type="tvdb"> and <id> are present, uniqueid wins
        let nfo = r#"<movie>
  <id>99999</id>
  <uniqueid type="tvdb">12345</uniqueid>
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.tvdb_id, Some("12345".into()));
    }

    #[test]
    fn parse_url_in_xml_nfo() {
        // URL embedded inside an XML NFO as a comment or content
        let nfo = r#"<movie>
  <title>Test</title>
  <!-- https://www.imdb.com/title/tt9876543/ -->
</movie>"#;
        let meta = parse_nfo(nfo);
        assert_eq!(meta.imdb_id, Some("tt9876543".into()));
        assert_eq!(meta.title, Some("Test".into()));
    }

    // -----------------------------------------------------------------------
    // Writer tests
    // -----------------------------------------------------------------------

    #[test]
    fn render_movie_full() {
        let title = make_title();
        let xml = render_movie_nfo(&title);
        assert!(xml.starts_with("<?xml version="));
        assert!(xml.contains("<movie>"));
        assert!(xml.contains("<title>The Matrix</title>"));
        assert!(xml.contains("<year>1999</year>"));
        assert!(xml.contains("<plot>A computer hacker"));
        assert!(xml.contains("<runtime>136</runtime>"));
        assert!(xml.contains("<genre>Action</genre>"));
        assert!(xml.contains("<genre>Sci-Fi</genre>"));
        assert!(xml.contains("<studio>Warner Bros.</studio>"));
        assert!(xml.contains("<uniqueid type=\"tvdb\" default=\"true\">12345</uniqueid>"));
        assert!(xml.contains("<uniqueid type=\"imdb\">tt0133093</uniqueid>"));
        assert!(xml.contains("<uniqueid type=\"tmdb\">603</uniqueid>"));
        assert!(xml.contains("</movie>"));
    }

    #[test]
    fn render_movie_minimal() {
        let title = Title {
            id: "t1".into(),
            name: "Minimal".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        };
        let xml = render_movie_nfo(&title);
        assert!(xml.contains("<title>Minimal</title>"));
        assert!(!xml.contains("<year>"));
        assert!(!xml.contains("<plot>"));
        assert!(!xml.contains("<runtime>"));
        assert!(!xml.contains("<genre>"));
        assert!(!xml.contains("<studio>"));
        assert!(!xml.contains("<uniqueid"));
    }

    #[test]
    fn render_tvshow() {
        let mut title = make_title();
        title.facet = MediaFacet::Series;
        title.network = Some("HBO".into());
        let xml = render_tvshow_nfo(&title);
        assert!(xml.contains("<tvshow>"));
        assert!(xml.contains("<title>The Matrix</title>"));
        assert!(xml.contains("<studio>HBO</studio>"));
        assert!(xml.contains("</tvshow>"));
    }

    #[test]
    fn render_episode() {
        let title = make_title();
        let episode = make_episode();
        let xml = render_episode_nfo(&title, &episode);
        assert!(xml.contains("<episodedetails>"));
        assert!(xml.contains("<title>Pilot</title>"));
        assert!(xml.contains("<season>1</season>"));
        assert!(xml.contains("<episode>1</episode>"));
        assert!(xml.contains("<plot>A high school chemistry"));
        assert!(xml.contains("<aired>2008-01-20</aired>"));
        assert!(xml.contains("<runtime>58</runtime>"));
        // Episode's own TVDB ID takes precedence over series TVDB ID
        assert!(xml.contains("<uniqueid type=\"tvdb\" default=\"true\">349232</uniqueid>"));
        assert!(xml.contains("</episodedetails>"));
    }

    #[test]
    fn xml_escape_special_chars() {
        assert_eq!(xml_escape("A & B"), "A &amp; B");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(xml_escape("it's"), "it&apos;s");
    }

    #[test]
    fn round_trip_movie() {
        let title = make_title();
        let xml = render_movie_nfo(&title);
        let parsed = parse_nfo(&xml);
        assert_eq!(parsed.tvdb_id, Some("12345".into()));
        assert_eq!(parsed.imdb_id, Some("tt0133093".into()));
        assert_eq!(parsed.tmdb_id, Some("603".into()));
        assert_eq!(parsed.title, Some("The Matrix".into()));
        assert_eq!(parsed.year, Some(1999));
    }

    #[test]
    fn round_trip_tvshow() {
        let mut title = make_title();
        title.facet = MediaFacet::Series;
        let xml = render_tvshow_nfo(&title);
        let parsed = parse_nfo(&xml);
        assert_eq!(parsed.tvdb_id, Some("12345".into()));
        assert_eq!(parsed.title, Some("The Matrix".into()));
        assert_eq!(parsed.year, Some(1999));
    }

    #[test]
    fn render_plexmatch_full() {
        let title = make_title();
        let out = render_plexmatch(&title);
        assert!(out.contains("title: The Matrix\n"));
        assert!(out.contains("year: 1999\n"));
        assert!(out.contains("tvdbid: 12345\n"));
        assert!(out.contains("imdbid: tt0133093\n"));
        assert!(out.contains("tmdbid: 603\n"));
    }

    #[test]
    fn render_plexmatch_minimal() {
        let mut title = make_title();
        title.name = "Minimal Show".into();
        title.facet = MediaFacet::Series;
        title.year = None;
        title.imdb_id = None;
        title.external_ids = vec![];
        let out = render_plexmatch(&title);
        assert_eq!(out, "title: Minimal Show\n");
    }

    #[test]
    fn render_plexmatch_no_year() {
        let mut title = make_title();
        title.year = None;
        let out = render_plexmatch(&title);
        assert!(out.contains("title: The Matrix\n"));
        assert!(!out.contains("year:"));
        assert!(out.contains("tvdbid: 12345\n"));
    }

    // -----------------------------------------------------------------------
    // Interstitial movie NFO tests
    // -----------------------------------------------------------------------

    fn make_interstitial_movie() -> scryer_domain::InterstitialMovieMetadata {
        scryer_domain::InterstitialMovieMetadata {
            tvdb_id: "54321".into(),
            name: "Mugen Train".into(),
            slug: "mugen-train".into(),
            year: Some(2020),
            content_status: "released".into(),
            overview: "Tanjiro boards the Mugen Train.".into(),
            poster_url: String::new(),
            language: "jpn".into(),
            runtime_minutes: 117,
            sort_title: "Mugen Train".into(),
            imdb_id: "tt11032374".into(),
            genres: vec!["Action".into(), "Anime".into()],
            studio: "ufotable".into(),
            digital_release_date: Some("2020-10-16".into()),
            association_confidence: Some("high".into()),
            continuity_status: Some("canon".into()),
            movie_form: Some("movie".into()),
            confidence: Some("high".into()),
            signal_summary: Some("TVDB linked movie".into()),
            placement: Some("ordered".into()),
            movie_tmdb_id: Some("635302".into()),
            movie_mal_id: Some("40748".into()),
            movie_anidb_id: None,
        }
    }

    #[test]
    fn render_interstitial_movie_nfo_full() {
        let movie = make_interstitial_movie();
        let xml = render_interstitial_movie_nfo(&movie, "S00E01", "1.1");
        assert!(xml.contains("<episodedetails>"));
        assert!(xml.contains("<title>Mugen Train</title>"));
        assert!(xml.contains("<season>0</season>"));
        assert!(xml.contains("<episode>1</episode>"));
        assert!(xml.contains("<plot>Tanjiro boards the Mugen Train.</plot>"));
        assert!(xml.contains("<aired>2020-10-16</aired>"));
        assert!(xml.contains("<runtime>117</runtime>"));
        assert!(xml.contains("<airsbefore_season>2</airsbefore_season>"));
        assert!(xml.contains("<airsbefore_episode>1</airsbefore_episode>"));
        assert!(xml.contains("<uniqueid type=\"tvdb\" default=\"true\">54321</uniqueid>"));
        assert!(xml.contains("<uniqueid type=\"imdb\">tt11032374</uniqueid>"));
        assert!(xml.contains("<uniqueid type=\"tmdb\">635302</uniqueid>"));
        assert!(xml.contains("</episodedetails>"));
    }

    #[test]
    fn render_interstitial_movie_nfo_no_release_date() {
        let mut movie = make_interstitial_movie();
        movie.digital_release_date = None;
        let xml = render_interstitial_movie_nfo(&movie, "S00E03", "2.1");
        assert!(!xml.contains("<aired>"));
        assert!(xml.contains("<episode>3</episode>"));
        assert!(xml.contains("<airsbefore_season>3</airsbefore_season>"));
    }

    #[test]
    fn airs_before_season_basic() {
        assert_eq!(airs_before_season_from_collection_index("1.1"), Some(2));
        assert_eq!(airs_before_season_from_collection_index("2.1"), Some(3));
        assert_eq!(airs_before_season_from_collection_index("0.1"), Some(1));
        assert_eq!(airs_before_season_from_collection_index("5.2"), Some(6));
    }

    #[test]
    fn airs_before_season_invalid() {
        assert_eq!(airs_before_season_from_collection_index("abc"), None);
        assert_eq!(airs_before_season_from_collection_index(""), None);
    }
}
