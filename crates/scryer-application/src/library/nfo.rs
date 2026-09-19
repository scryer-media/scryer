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

/// Everything a renderer needs beyond the catalog row it is rendering.
///
/// Every field is optional. A caller with nothing else in hand passes
/// `NfoContext::default()` and each renderer simply omits the elements those
/// facts would have filled — the sidecar stays well-formed either way.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NfoContext<'a> {
    /// External ratings the catalog holds for the title, in the metadata
    /// gateway's own order.
    pub ratings: &'a [scryer_domain::TitleExternalRating],
    /// Cast and crew in billing order. Empty whenever credits are not
    /// hydrated (the feature ships dark), so every use must tolerate that.
    pub credits: &'a [scryer_domain::TitleCredit],
    /// The persisted record for the file this sidecar sits beside; the source
    /// of the `<fileinfo><streamdetails>` block.
    pub media_file: Option<&'a crate::TitleMediaFile>,
    /// When the file was imported, written as `<dateadded>`.
    pub date_added: Option<chrono::DateTime<chrono::Utc>>,
}

/// Render a Jellyfin/Kodi-compatible `<movie>` NFO for the given Title.
pub(crate) fn render_movie_nfo(title: &Title, context: &NfoContext<'_>) -> String {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    write_xml_decl(&mut w);
    let movie = BytesStart::new("movie");
    w.write_event(Event::Start(movie)).ok();

    write_element(&mut w, "title", &title.name);
    write_optional_non_empty_element(&mut w, "originaltitle", title_original_title(title));
    write_optional_non_empty_element(&mut w, "sorttitle", title.sort_title.as_deref());
    write_ratings(&mut w, context.ratings, primary_rating_source(title));
    write_optional_non_empty_element(&mut w, "plot", title.overview.as_deref());
    if let Some(runtime) = title.runtime_minutes.filter(|runtime| *runtime > 0) {
        write_element(&mut w, "runtime", &runtime.to_string());
    }
    write_optional_non_empty_element(&mut w, "premiered", movie_premiered_date(title));
    if let Some(year) = title.year {
        write_element(&mut w, "year", &year.to_string());
    }
    for genre in canonical_title_genres(title) {
        write_element(&mut w, "genre", &genre);
    }
    for tag in operator_title_tags(title) {
        write_element(&mut w, "tag", &tag);
    }
    write_optional_non_empty_element(&mut w, "country", title.country.as_deref());
    write_optional_non_empty_element(&mut w, "studio", title.studio.as_deref());
    write_optional_non_empty_element(&mut w, "status", title.content_status.as_deref());

    write_movie_uniqueids(&mut w, title);

    write_artwork(&mut w, title);
    write_credits(&mut w, context.credits);
    write_stream_details(&mut w, context.media_file);
    write_date_added(&mut w, context.date_added);

    w.write_event(Event::End(BytesEnd::new("movie"))).ok();
    finish_xml(buf)
}

/// Render a Jellyfin/Kodi-compatible `<tvshow>` NFO for the given series Title.
pub(crate) fn render_tvshow_nfo(title: &Title, context: &NfoContext<'_>) -> String {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    write_xml_decl(&mut w);
    let tvshow = BytesStart::new("tvshow");
    w.write_event(Event::Start(tvshow)).ok();

    write_element(&mut w, "title", &title.name);
    // Sonarr emits `showtitle` on the series document too, and Jellyfin's
    // series reader tolerates it; Kodi ignores what it does not know.
    write_element(&mut w, "showtitle", &title.name);
    write_optional_non_empty_element(&mut w, "originaltitle", title_original_title(title));
    write_optional_non_empty_element(&mut w, "sorttitle", title.sort_title.as_deref());
    write_ratings(&mut w, context.ratings, primary_rating_source(title));
    write_optional_non_empty_element(&mut w, "plot", title.overview.as_deref());
    write_optional_non_empty_element(&mut w, "premiered", title.first_aired.as_deref());
    if let Some(year) = title.year {
        write_element(&mut w, "year", &year.to_string());
    }
    write_optional_non_empty_element(&mut w, "status", title.content_status.as_deref());
    // Kodi and Jellyfin both treat `studio` as repeatable. The network is the
    // one Sonarr writes; a distinct production studio is added after it rather
    // than replacing it.
    write_optional_non_empty_element(&mut w, "studio", title.network.as_deref());
    if let Some(studio) = title
        .studio
        .as_deref()
        .filter(|studio| !studio.is_empty())
        .filter(|studio| title.network.as_deref() != Some(*studio))
    {
        write_element(&mut w, "studio", studio);
    }
    write_optional_non_empty_element(&mut w, "country", title.country.as_deref());
    for genre in canonical_title_genres(title) {
        write_element(&mut w, "genre", &genre);
    }
    for tag in operator_title_tags(title) {
        write_element(&mut w, "tag", &tag);
    }

    write_tvshow_uniqueids(&mut w, title);
    write_episode_guide(&mut w, title);

    write_artwork(&mut w, title);
    write_credits(&mut w, context.credits);
    write_date_added(&mut w, context.date_added);

    w.write_event(Event::End(BytesEnd::new("tvshow"))).ok();
    finish_xml(buf)
}

/// Render a Kodi-compatible `<episodedetails>` NFO for a single episode.
#[cfg(test)]
pub(crate) fn render_episode_nfo(
    title: &Title,
    episode: &Episode,
    context: &NfoContext<'_>,
) -> String {
    render_episodes_nfo(title, std::slice::from_ref(episode), context)
}

/// Render the `.nfo` body for one video file that carries `episodes`.
///
/// A file mapped to several episodes gets one `<episodedetails>` root per
/// episode, concatenated in the one file. That is the Kodi convention and it is
/// what Jellyfin's `EpisodeNfoParser` expects: it splits the file on
/// `</episodedetails>` and merges the blocks, which is how it recovers the
/// episode range a multi-episode file covers.
pub(crate) fn render_episodes_nfo(
    title: &Title,
    episodes: &[Episode],
    context: &NfoContext<'_>,
) -> String {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    write_xml_decl(&mut w);
    for episode in episodes {
        w.write_event(Event::Start(BytesStart::new("episodedetails")))
            .ok();

        write_optional_non_empty_element(&mut w, "title", episode.title.as_deref());
        write_element(&mut w, "showtitle", &title.name);
        if let Some(season) = episode.season_number.as_deref() {
            write_element(&mut w, "season", season);
        }
        if let Some(episode_number) = episode.episode_number.as_deref() {
            write_element(&mut w, "episode", episode_number);
        }
        write_optional_non_empty_element(&mut w, "aired", episode.air_date.as_deref());
        write_optional_non_empty_element(&mut w, "plot", episode.overview.as_deref());
        if let Some(minutes) = episode
            .duration_seconds
            .map(|seconds| seconds / 60)
            .filter(|minutes| *minutes > 0)
        {
            write_element(&mut w, "runtime", &minutes.to_string());
        }

        // Episode-level identity only. The series TVDB id is *not* an episode
        // id: writing it here tells the media server this file is the show,
        // which mislabels every episode of the series identically. When the
        // episode has no id of its own, nothing is written.
        if let Some(tvdb_id) = episode
            .tvdb_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            write_uniqueid(&mut w, "tvdb", tvdb_id, true);
        }

        if let Some(image_url) = episode
            .image_url
            .as_deref()
            .filter(|url| is_absolute_url(url))
        {
            write_element(&mut w, "thumb", image_url);
        }
        write_credits(&mut w, context.credits);
        write_stream_details(&mut w, context.media_file);
        write_date_added(&mut w, context.date_added);

        w.write_event(Event::End(BytesEnd::new("episodedetails")))
            .ok();
    }

    finish_xml(buf)
}

/// Render a Kodi/Jellyfin-compatible `<episodedetails>` NFO for a series movie.
///
/// Written as a season 0 special so media servers recognize it as part of the
/// series. `airsbefore_season`/`airsbefore_episode` is Kodi's spelling and
/// `displayseason`/`displayepisode` is the one Jellyfin's `EpisodeNfoParser`
/// lists first; both map to the same placement, so both are written with the
/// same values rather than betting on which reader sees the file.
pub(crate) fn render_series_movie_episode_nfo(
    movie: &scryer_domain::MovieEntity,
    season_episode: &str,
    after_season: Option<i32>,
    series_title: Option<&str>,
    context: &NfoContext<'_>,
) -> String {
    let mut buf = Cursor::new(Vec::new());
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    write_xml_decl(&mut w);
    let tag = BytesStart::new("episodedetails");
    w.write_event(Event::Start(tag)).ok();

    write_element(&mut w, "title", &movie.title);
    write_optional_non_empty_element(&mut w, "showtitle", series_title);
    write_optional_non_empty_element(&mut w, "sorttitle", movie.sort_title.as_deref());
    write_ratings(
        &mut w,
        movie
            .ratings
            .as_ref()
            .map(|ratings| ratings.external_ratings.as_slice())
            .unwrap_or_default(),
        Some("tmdb"),
    );
    write_element(&mut w, "season", "0");

    if let Some(ep_str) = season_episode.strip_prefix("S00E")
        && let Ok(ep_num) = ep_str.parse::<i32>()
    {
        write_element(&mut w, "episode", &ep_num.to_string());
    }

    if let Some(overview) = movie.overview.as_deref().filter(|value| !value.is_empty()) {
        write_element(&mut w, "plot", overview);
    }
    if let Some(release_date) = movie
        .digital_release_date
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        write_element(&mut w, "aired", release_date);
    }
    if let Some(runtime_minutes) = movie.runtime_minutes.filter(|minutes| *minutes > 0) {
        write_element(&mut w, "runtime", &runtime_minutes.to_string());
    }

    if let Some(after_season) = after_season {
        let before_season = (after_season + 1).to_string();
        write_element(&mut w, "airsbefore_season", &before_season);
        write_element(&mut w, "airsbefore_episode", "1");
        write_element(&mut w, "displayseason", &before_season);
        write_element(&mut w, "displayepisode", "1");
    }

    if let Some(tvdb_id) = movie.tvdb_id.as_deref().filter(|value| !value.is_empty()) {
        write_uniqueid(&mut w, "tvdb", tvdb_id, true);
    }
    if let Some(imdb_id) = movie.imdb_id.as_deref().filter(|value| !value.is_empty()) {
        write_uniqueid(&mut w, "imdb", imdb_id, false);
    }
    if let Some(tmdb_id) = movie.tmdb_id.as_deref().filter(|value| !value.is_empty()) {
        write_uniqueid(&mut w, "tmdb", tmdb_id, false);
    }

    write_credits(&mut w, movie.credits.as_deref().unwrap_or(context.credits));
    write_stream_details(&mut w, context.media_file);
    write_date_added(&mut w, context.date_added);

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

/// The original-language title, when the catalog actually holds one.
///
/// Jellyfin reads `<originaltitle>` as a real fact about the work, so it is
/// only written when a tagged alias is in the title's own original language and
/// differs from the display name. Guessing one out of the untagged alias bag
/// would label a random regional title as the original.
fn title_original_title(title: &Title) -> Option<&str> {
    let language = title.language.as_deref().map(str::trim)?;
    if language.is_empty() {
        return None;
    }
    title
        .tagged_aliases
        .iter()
        .find(|alias| {
            alias.language.trim().eq_ignore_ascii_case(language)
                && !alias.name.trim().is_empty()
                && alias.name.trim() != title.name.trim()
        })
        .map(|alias| alias.name.as_str())
}

/// Operator-facing tags only. The `scryer:` namespace inside `Title::tags` is
/// reserved for structured per-title settings and means nothing to a media
/// server, so it never reaches the sidecar.
fn operator_title_tags(title: &Title) -> Vec<String> {
    title
        .tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty() && !tag.starts_with("scryer:"))
        .map(str::to_string)
        .collect()
}

/// A movie's `<premiered>`: its release date, preferring the digital release
/// the catalog records for movies over the generic first-aired date.
fn movie_premiered_date(title: &Title) -> Option<&str> {
    title
        .digital_release_date
        .as_deref()
        .or(title.first_aired.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The provider a title's identity is anchored to, which is the rating that
/// gets `default="true"` when the title carries one from that source.
fn primary_rating_source(title: &Title) -> Option<&'static str> {
    match title.facet {
        scryer_domain::MediaFacet::Movie => Some("tmdb"),
        _ => Some("tvdb"),
    }
}

/// Kodi's `<ratings>` block, as Radarr writes it.
///
/// `name` uses Kodi's own vocabulary where it has one (`themoviedb`,
/// `tomatometerallcritics`); everything else passes through lowercased.
/// Percent-scale sources are declared `max="100"` and written out of a hundred,
/// because Jellyfin routes a `tomato*` rating into `CriticRating`, which it
/// reads on that scale; everything else is written on the 0–10 scale
/// `max="10"` promises.
fn write_ratings<W: std::io::Write>(
    w: &mut Writer<W>,
    ratings: &[scryer_domain::TitleExternalRating],
    default_source: Option<&str>,
) {
    let rendered = ratings
        .iter()
        .filter(|rating| !rating.source.trim().is_empty())
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        return;
    }

    // Exactly one rating may claim the default. Prefer the title's own
    // provider; otherwise the first rating the gateway returned, which is its
    // own order of preference.
    let default_index = default_source
        .and_then(|source| {
            rendered
                .iter()
                .position(|rating| rating.source.trim().eq_ignore_ascii_case(source))
        })
        .unwrap_or(0);

    w.write_event(Event::Start(BytesStart::new("ratings"))).ok();
    for (index, rating) in rendered.iter().enumerate() {
        let source = rating.source.trim().to_ascii_lowercase();
        let name = kodi_rating_name(&source);
        let percent_scale = rating_name_is_percent_scale(name);
        let mut tag = BytesStart::new("rating");
        tag.push_attribute(("name", name));
        tag.push_attribute(("max", if percent_scale { "100" } else { "10" }));
        if index == default_index {
            tag.push_attribute(("default", "true"));
        }
        w.write_event(Event::Start(tag)).ok();
        let value = if percent_scale {
            format!("{:.0}", rating.normalized * 10.0)
        } else {
            format!("{:.1}", rating.normalized)
        };
        write_element(w, "value", &value);
        if let Some(votes) = rating.votes.filter(|votes| *votes > 0) {
            write_element(w, "votes", &votes.to_string());
        }
        w.write_event(Event::End(BytesEnd::new("rating"))).ok();
    }
    w.write_event(Event::End(BytesEnd::new("ratings"))).ok();
}

/// Scryer's provider keys in Kodi's own `name=` vocabulary.
fn kodi_rating_name(source: &str) -> &str {
    match source {
        "tmdb" | "themoviedb" => "themoviedb",
        "rottentomatoes" | "rotten_tomatoes" | "tomatoes" => "tomatometerallcritics",
        "audience" | "popcorn" | "popcornmeter" => "tomatometerallaudience",
        "mcuser" | "metacriticuser" => "metacritic",
        other => other,
    }
}

/// Whether a rating goes out on the 0-100 scale rather than 0-10.
///
/// Kodi honours whatever `max=` says, but Jellyfin does not: its
/// `BaseNfoParser` routes a rating into `CriticRating` (0-100) only when the
/// name contains "tomato" and neither "audience" nor "avg", and everything else
/// into `CommunityRating` (0-10) regardless of `max`. So the percent scale is
/// used for exactly the names that land in `CriticRating`; every other source,
/// Metacritic and the Rotten Tomatoes audience score included, is written on
/// the 0-10 scale both readers will agree about.
fn rating_name_is_percent_scale(name: &str) -> bool {
    name.contains("tomato") && !name.contains("audience") && !name.contains("avg")
}

/// Poster and fanart, as remote URLs. Only absolute http(s) URLs are written: a
/// media server cannot resolve a Scryer-local cache path, and a relative path
/// would point at whatever happens to sit beside the file.
fn write_artwork<W: std::io::Write>(w: &mut Writer<W>, title: &Title) {
    if let Some(poster) = title
        .poster_source_url
        .as_deref()
        .or(title.poster_url.as_deref())
        .filter(|url| is_absolute_url(url))
    {
        let mut tag = BytesStart::new("thumb");
        tag.push_attribute(("aspect", "poster"));
        w.write_event(Event::Start(tag)).ok();
        w.write_event(Event::Text(BytesText::new(poster))).ok();
        w.write_event(Event::End(BytesEnd::new("thumb"))).ok();
    }
    if let Some(fanart) = title
        .background_source_url
        .as_deref()
        .or(title.background_url.as_deref())
        .filter(|url| is_absolute_url(url))
    {
        w.write_event(Event::Start(BytesStart::new("fanart"))).ok();
        write_element(w, "thumb", fanart);
        w.write_event(Event::End(BytesEnd::new("fanart"))).ok();
    }
}

fn is_absolute_url(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("http://") || value.starts_with("https://")
}

/// `<actor>`, `<director>` and `<credits>` from the title's credit rows.
///
/// Jellyfin maps `<credits>` to a writer and `<director>` to a director, which
/// is also what Radarr writes; `<actor>` carries `name`/`role`/`order`/`thumb`,
/// the exact four fields Jellyfin's `GetPersonFromXmlNode` reads. Credits ship
/// dark, so an empty slice is the normal case and writes nothing.
fn write_credits<W: std::io::Write>(w: &mut Writer<W>, credits: &[scryer_domain::TitleCredit]) {
    for credit in credits {
        let name = credit.person_name.trim();
        if name.is_empty() {
            continue;
        }
        match credit.kind.trim().to_ascii_lowercase().as_str() {
            "actor" | "voice_actor" => {
                w.write_event(Event::Start(BytesStart::new("actor"))).ok();
                write_element(w, "name", name);
                let role = credit.character_name.trim();
                if !role.is_empty() {
                    write_element(w, "role", role);
                }
                write_element(w, "order", &credit.billing_order.to_string());
                if is_absolute_url(&credit.person_image_url) {
                    write_element(w, "thumb", credit.person_image_url.trim());
                }
                w.write_event(Event::End(BytesEnd::new("actor"))).ok();
            }
            "director" => write_element(w, "director", name),
            "writer" => write_element(w, "credits", name),
            _ => {}
        }
    }
}

/// Kodi v19+ `<episodeguide>`: a `uniqueid` child rather than the JSON blob
/// older Kodi builds wanted. Omitted entirely when the series has no TVDB id,
/// since an episode guide with no id to look up is worse than none.
fn write_episode_guide<W: std::io::Write>(w: &mut Writer<W>, title: &Title) {
    let Some(tvdb_id) = title_external_id_value(title, "tvdb") else {
        return;
    };
    w.write_event(Event::Start(BytesStart::new("episodeguide")))
        .ok();
    write_uniqueid(w, "tvdb", tvdb_id, true);
    w.write_event(Event::End(BytesEnd::new("episodeguide")))
        .ok();
}

/// `<dateadded>` in Kodi's `YYYY-MM-DD HH:MM:SS` form.
fn write_date_added<W: std::io::Write>(
    w: &mut Writer<W>,
    date_added: Option<chrono::DateTime<chrono::Utc>>,
) {
    if let Some(date_added) = date_added {
        write_element(
            w,
            "dateadded",
            &date_added.format("%Y-%m-%d %H:%M:%S").to_string(),
        );
    }
}

/// `<fileinfo><streamdetails>` for the file this sidecar sits beside.
///
/// Mirrors Sonarr's `XbmcMetadata` mapping. The whole block is omitted when the
/// record carries no analysis at all, and every individual element is omitted
/// when its value is unknown — a media server reading an empty `<codec/>`
/// records an empty codec rather than no codec.
fn write_stream_details<W: std::io::Write>(
    w: &mut Writer<W>,
    media_file: Option<&crate::TitleMediaFile>,
) {
    let Some(media_file) = media_file.filter(|file| media_file_has_analysis(file)) else {
        return;
    };

    w.write_event(Event::Start(BytesStart::new("fileinfo")))
        .ok();
    w.write_event(Event::Start(BytesStart::new("streamdetails")))
        .ok();

    w.write_event(Event::Start(BytesStart::new("video"))).ok();
    if let Some(codec) = media_file.video_codec.map(kodi_video_codec) {
        write_element(w, "codec", codec);
    }
    if let Some(aspect) = stream_aspect_ratio(media_file.video_width, media_file.video_height) {
        write_element(w, "aspect", &aspect);
    }
    if let Some(width) = media_file.video_width.filter(|width| *width > 0) {
        write_element(w, "width", &width.to_string());
    }
    if let Some(height) = media_file.video_height.filter(|height| *height > 0) {
        write_element(w, "height", &height.to_string());
    }
    if let Some(duration) = media_file.duration_seconds.filter(|value| *value > 0) {
        write_element(w, "durationinseconds", &duration.to_string());
    }
    // Kodi and Sonarr both carry the video bitrate in bits per second; Scryer
    // stores kbps, so the scale is converted rather than relabelled.
    if let Some(bitrate_kbps) = media_file.video_bitrate_kbps.filter(|value| *value > 0) {
        write_element(w, "bitrate", &(i64::from(bitrate_kbps) * 1000).to_string());
    }
    if let Some(hdr_type) = media_file
        .video_hdr_format
        .as_deref()
        .and_then(kodi_hdr_type)
    {
        write_element(w, "hdrtype", hdr_type);
    }
    if let Some(frame_rate) = media_file
        .video_frame_rate
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        write_element(w, "framerate", frame_rate);
    }
    w.write_event(Event::End(BytesEnd::new("video"))).ok();

    for stream in &media_file.audio_streams {
        w.write_event(Event::Start(BytesStart::new("audio"))).ok();
        if let Some(codec) = kodi_audio_codec(stream.codec.as_deref(), stream.profile.as_deref()) {
            write_element(w, "codec", &codec);
        }
        if let Some(language) = stream_language(stream.language.as_deref()) {
            write_element(w, "language", language);
        }
        if let Some(channels) = stream.channels.filter(|channels| *channels > 0) {
            write_element(w, "channels", &channels.to_string());
        }
        if let Some(bitrate_kbps) = stream.bitrate_kbps.filter(|value| *value > 0) {
            write_element(w, "bitrate", &(i64::from(bitrate_kbps) * 1000).to_string());
        }
        w.write_event(Event::End(BytesEnd::new("audio"))).ok();
    }

    for stream in &media_file.subtitle_streams {
        let Some(language) = stream_language(stream.language.as_deref()) else {
            continue;
        };
        w.write_event(Event::Start(BytesStart::new("subtitle")))
            .ok();
        write_element(w, "language", language);
        w.write_event(Event::End(BytesEnd::new("subtitle"))).ok();
    }

    w.write_event(Event::End(BytesEnd::new("streamdetails")))
        .ok();
    w.write_event(Event::End(BytesEnd::new("fileinfo"))).ok();
}

/// Whether the record carries enough analysis to describe a stream at all.
/// A row inserted but never scanned has every analysis field empty, and an
/// all-empty `<streamdetails>` is worse than none.
fn media_file_has_analysis(media_file: &crate::TitleMediaFile) -> bool {
    media_file.video_codec.is_some()
        || media_file.video_width.is_some()
        || media_file.video_height.is_some()
        || media_file.duration_seconds.is_some()
        || !media_file.audio_streams.is_empty()
        || !media_file.subtitle_streams.is_empty()
}

/// Kodi's codec vocabulary, which is ffmpeg's lowercase name rather than the
/// marketing one Scryer's parser records.
fn kodi_video_codec(codec: crate::release_parser::VideoCodec) -> &'static str {
    use crate::release_parser::VideoCodec;
    match codec {
        VideoCodec::H264 => "h264",
        VideoCodec::H265 => "hevc",
        VideoCodec::Av1 => "av1",
        VideoCodec::Vp9 => "vp9",
        VideoCodec::Vc1 => "vc1",
        VideoCodec::Mpeg2 => "mpeg2video",
        VideoCodec::Mpeg4 => "mpeg4",
        VideoCodec::Xvid => "xvid",
        VideoCodec::Divx => "divx",
        VideoCodec::Vvc => "vvc",
    }
}

/// Sonarr's `XbmcMetadataFormatter.FormatAudioCodec`: the analyzer's codec name
/// is already ffmpeg's, and a known profile refines it into Kodi's dedicated
/// code for that variant.
fn kodi_audio_codec(codec: Option<&str>, profile: Option<&str>) -> Option<String> {
    let codec = codec.map(str::trim).filter(|codec| !codec.is_empty())?;
    let codec = codec.to_ascii_lowercase();
    let profile = profile.map(str::trim).unwrap_or_default();
    let refined = match (codec.as_str(), profile) {
        ("dts", "DTS-HD HRA") => "dtshd_hra",
        ("dts", "DTS-HD MA") => "dtshd_ma",
        ("dts", "DTS-HD MA + DTS:X") => "dtshd_ma_x",
        ("dts", "DTS-HD MA + DTS:X IMAX") => "dtshd_ma_x_imax",
        ("truehd", "Dolby TrueHD + Dolby Atmos") => "truehd_atmos",
        ("eac3", "Dolby Digital Plus + Dolby Atmos") => "eac3_ddp_atmos",
        ("aac", "LC") => "aac_lc",
        ("aac", "HE-AAC") => "he_aac",
        ("aac", "HE-AACv2") => "he_aac_v2",
        ("aac", "SSR") => "aac_ssr",
        ("aac", "LTP") => "aac_ltp",
        _ => return Some(codec),
    };
    Some(refined.to_string())
}

/// Sonarr's `hdrtype` vocabulary. Sonarr writes an empty element for SDR; we
/// omit it, because an empty `<hdrtype/>` asserts an empty HDR format.
fn kodi_hdr_type(hdr_format: &str) -> Option<&'static str> {
    match hdr_format.trim().to_ascii_lowercase().as_str() {
        "dolby vision" | "dolbyvision" => Some("dolbyvision"),
        "hdr10" | "hdr10+" | "pq10" => Some("hdr10"),
        "hlg" => Some("hlg"),
        _ => None,
    }
}

/// Display aspect ratio as Kodi wants it: width over height, two decimals.
fn stream_aspect_ratio(width: Option<i32>, height: Option<i32>) -> Option<String> {
    let width = width.filter(|width| *width > 0)?;
    let height = height.filter(|height| *height > 0)?;
    Some(format!("{:.2}", f64::from(width) / f64::from(height)))
}

/// A stream's language tag, exactly as the analyzer read it from the container.
/// `und` is the container's own "unknown" and carries no more information than
/// omitting the element.
fn stream_language(language: Option<&str>) -> Option<&str> {
    language
        .map(str::trim)
        .filter(|language| !language.is_empty() && !language.eq_ignore_ascii_case("und"))
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

// ---------------------------------------------------------------------------
// Write behaviour
// ---------------------------------------------------------------------------

/// What one sidecar write actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NfoWriteOutcome {
    Written,
    /// A file was already there. Scryer never replaces one: whatever is on
    /// disk is either an operator's own metadata or a media server's, and
    /// neither is ours to overwrite.
    SkippedExisting,
    Failed,
}

/// Write `content` to `path` only if nothing is there.
///
/// The content is written in full to a uniquely named temp file in the same
/// directory and only then published to the final name, so a write that dies
/// halfway — a full disk, a mount that drops — never leaves a truncated
/// sidecar at the real path for every later import to mistake for an
/// operator's own file.
///
/// Publishing is a hard link rather than a rename: a rename would clobber a
/// file that appeared in the meantime, and `link` fails with `AlreadyExists`
/// instead. A second importer writing the same folder — or a media server
/// that wrote its own sidecar a millisecond ago — therefore cannot lose the
/// race and be overwritten. Filesystems that cannot hard link (some SMB,
/// FUSE and exFAT mounts) fall back to `create_new` on the destination
/// itself, which is no-clobber too but has to clean up after a failed write.
///
/// The only files this ever removes are the temp file it just created and,
/// on the fallback path, a destination file it created itself in this same
/// call. Never fails the import: a sidecar is metadata, not the media.
pub(crate) async fn write_nfo_if_absent(path: &std::path::Path, content: &str) -> NfoWriteOutcome {
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        tracing::debug!(path = %path.display(), "NFO sidecar already exists; leaving it as it is");
        return NfoWriteOutcome::SkippedExisting;
    }

    if let Some(parent) = path.parent()
        && let Err(error) = tokio::fs::create_dir_all(parent).await
    {
        tracing::warn!(
            error = %error,
            path = %path.display(),
            "failed to create the directory for an NFO sidecar"
        );
        return NfoWriteOutcome::Failed;
    }

    let temp_path = nfo_temp_path(path);
    let temp = match write_nfo_temp_file(&temp_path, content).await {
        Ok(()) => temp_path,
        Err(error) => {
            tracing::warn!(error = %error, path = %path.display(), "failed to write NFO sidecar");
            remove_best_effort(&temp_path).await;
            return NfoWriteOutcome::Failed;
        }
    };

    publish_staged_nfo(&temp, path, content).await
}

/// Link the staged content into place under the real name and drop the staging
/// file, whatever the outcome.
async fn publish_staged_nfo(
    temp: &std::path::Path,
    path: &std::path::Path,
    content: &str,
) -> NfoWriteOutcome {
    let published = tokio::fs::hard_link(temp, path).await;
    remove_best_effort(temp).await;
    match published {
        Ok(()) => {
            tracing::info!(path = %path.display(), "wrote NFO sidecar");
            NfoWriteOutcome::Written
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            tracing::debug!(
                path = %path.display(),
                "NFO sidecar appeared while it was being written; leaving it as it is"
            );
            NfoWriteOutcome::SkippedExisting
        }
        Err(error) => {
            // No hard links on this filesystem. `create_new` still refuses to
            // clobber; it just cannot promise the content lands whole.
            tracing::debug!(
                error = %error,
                path = %path.display(),
                "hard links are unavailable here; writing the NFO sidecar in place"
            );
            write_nfo_in_place(path, content).await
        }
    }
}

/// A temp name beside `path`, unique across the concurrent writers of one
/// process and across processes sharing the folder.
fn nfo_temp_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let stem = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sidecar".to_string());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = format!(".{stem}.scryer-{}-{sequence}.tmp", std::process::id());
    match path.parent() {
        Some(parent) => parent.join(name),
        None => std::path::PathBuf::from(name),
    }
}

/// Write the whole content to a freshly created temp file and get it onto the
/// device before anything links it into place.
async fn write_nfo_temp_file(temp_path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .await?;
    file.write_all(content.as_bytes()).await?;
    file.flush().await?;
    file.sync_all().await?;
    Ok(())
}

/// Fallback for filesystems without hard links: `create_new` the destination
/// and write into it. If the write fails, the half-written file is one this
/// call created moments ago and nobody else has seen, so it is removed again
/// rather than left to look like an operator's own sidecar.
async fn write_nfo_in_place(path: &std::path::Path, content: &str) -> NfoWriteOutcome {
    use tokio::io::AsyncWriteExt;

    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await;
    let mut file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            tracing::debug!(
                path = %path.display(),
                "NFO sidecar appeared while it was being written; leaving it as it is"
            );
            return NfoWriteOutcome::SkippedExisting;
        }
        Err(error) => {
            tracing::warn!(error = %error, path = %path.display(), "failed to write NFO sidecar");
            return NfoWriteOutcome::Failed;
        }
    };

    let written = async {
        file.write_all(content.as_bytes()).await?;
        file.flush().await
    }
    .await;
    if let Err(error) = written {
        tracing::warn!(error = %error, path = %path.display(), "failed to write NFO sidecar");
        remove_best_effort(path).await;
        return NfoWriteOutcome::Failed;
    }

    tracing::info!(path = %path.display(), "wrote NFO sidecar");
    NfoWriteOutcome::Written
}

/// Remove a file this call created itself. A failure here is worth a line in
/// the log and nothing more: the sidecar write has already been decided.
async fn remove_best_effort(path: &std::path::Path) {
    if let Err(error) = tokio::fs::remove_file(path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::debug!(
            error = %error,
            path = %path.display(),
            "failed to clean up a temporary NFO sidecar file"
        );
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
                ExternalId::new("tvdb", "12345"),
                ExternalId::new("tmdb", "603"),
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
        let xml = render_movie_nfo(&title, &NfoContext::default());
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
        let xml = render_tvshow_nfo(&title, &NfoContext::default());
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
        let xml = render_episode_nfo(&title, &episode, &NfoContext::default());
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
        let xml = render_movie_nfo(&title, &NfoContext::default());
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

    // -----------------------------------------------------------------------
    // Writer tests — fully populated titles
    // -----------------------------------------------------------------------

    /// A movie title with every field the renderers read actually filled, so
    /// one snapshot pins the whole element set rather than one element each.
    fn populated_movie_title() -> Title {
        let mut title = make_title();
        title.name = "Paper Lantern".into();
        title.sort_title = Some("Paper Lantern".into());
        title.language = Some("ja".into());
        title.tagged_aliases = vec![
            scryer_domain::TaggedAlias {
                name: "Kamiandon".into(),
                language: "ja".into(),
            },
            scryer_domain::TaggedAlias {
                name: "Lanterne de Papier".into(),
                language: "fr".into(),
            },
        ];
        title.tags = vec![
            "favourites".into(),
            "scryer:release-numbering:official".into(),
        ];
        title.country = Some("Japan".into());
        title.studio = Some("Harbor Pals Studio".into());
        title.content_status = Some("Released".into());
        title.digital_release_date = Some("2021-06-04".into());
        title.poster_url = Some("https://images.example.invalid/paper-lantern-poster.jpg".into());
        title.background_url =
            Some("https://images.example.invalid/paper-lantern-fanart.jpg".into());
        title
    }

    fn populated_series_title() -> Title {
        let mut title = populated_movie_title();
        title.facet = MediaFacet::Series;
        title.name = "Harbor Pals".into();
        title.sort_title = Some("Harbor Pals".into());
        title.network = Some("Lantern Broadcasting".into());
        title.first_aired = Some("2019-09-12".into());
        title.content_status = Some("Continuing".into());
        title
    }

    fn sample_ratings() -> Vec<scryer_domain::TitleExternalRating> {
        vec![
            scryer_domain::TitleExternalRating {
                source: "imdb".into(),
                value: Some(8.2),
                score: Some(8.2),
                normalized: 8.2,
                votes: Some(4211),
                url: String::new(),
            },
            scryer_domain::TitleExternalRating {
                source: "tmdb".into(),
                value: Some(7.6),
                score: Some(7.6),
                normalized: 7.6,
                votes: Some(910),
                url: String::new(),
            },
            scryer_domain::TitleExternalRating {
                source: "rottentomatoes".into(),
                value: Some(91.0),
                score: Some(91.0),
                normalized: 9.1,
                votes: None,
                url: String::new(),
            },
        ]
    }

    fn sample_credits() -> Vec<scryer_domain::TitleCredit> {
        vec![
            scryer_domain::TitleCredit {
                kind: "actor".into(),
                person_name: "Ines Calder".into(),
                character_name: "Wren".into(),
                billing_order: 0,
                person_image_url: "https://images.example.invalid/ines-calder.jpg".into(),
                ..Default::default()
            },
            scryer_domain::TitleCredit {
                kind: "actor".into(),
                person_name: "Bo Tamsin".into(),
                character_name: String::new(),
                billing_order: 1,
                ..Default::default()
            },
            scryer_domain::TitleCredit {
                kind: "director".into(),
                person_name: "Mira Vell".into(),
                billing_order: 0,
                ..Default::default()
            },
            scryer_domain::TitleCredit {
                kind: "writer".into(),
                person_name: "Otto Renn".into(),
                billing_order: 1,
                ..Default::default()
            },
            scryer_domain::TitleCredit {
                kind: "producer".into(),
                person_name: "Never Rendered".into(),
                billing_order: 2,
                ..Default::default()
            },
        ]
    }

    fn sample_media_file() -> crate::TitleMediaFile {
        crate::TitleMediaFile {
            id: "mf1".into(),
            title_id: "t1".into(),
            video_codec: Some(crate::release_parser::VideoCodec::H265),
            video_width: Some(3840),
            video_height: Some(2160),
            video_bitrate_kbps: Some(18_000),
            video_hdr_format: Some("Dolby Vision".into()),
            video_frame_rate: Some("23.976".into()),
            duration_seconds: Some(6120),
            audio_streams: vec![
                crate::AudioStreamDetail {
                    codec: Some("truehd".into()),
                    profile: Some("Dolby TrueHD + Dolby Atmos".into()),
                    channels: Some(8),
                    language: Some("eng".into()),
                    name: None,
                    bitrate_kbps: Some(4500),
                },
                crate::AudioStreamDetail {
                    codec: Some("aac".into()),
                    profile: Some("LC".into()),
                    channels: Some(2),
                    language: Some("und".into()),
                    name: None,
                    bitrate_kbps: None,
                },
            ],
            subtitle_streams: vec![
                crate::SubtitleStreamDetail {
                    codec: Some("subrip".into()),
                    language: Some("eng".into()),
                    name: None,
                    forced: false,
                    default: true,
                },
                crate::SubtitleStreamDetail {
                    codec: Some("subrip".into()),
                    language: Some("und".into()),
                    name: None,
                    forced: false,
                    default: false,
                },
            ],
            ..Default::default()
        }
    }

    fn sample_date_added() -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc.with_ymd_and_hms(2026, 3, 14, 9, 5, 1).unwrap()
    }

    fn populated_context<'a>(
        ratings: &'a [scryer_domain::TitleExternalRating],
        credits: &'a [scryer_domain::TitleCredit],
        media_file: &'a crate::TitleMediaFile,
    ) -> NfoContext<'a> {
        NfoContext {
            ratings,
            credits,
            media_file: Some(media_file),
            date_added: Some(sample_date_added()),
        }
    }

    #[test]
    fn render_movie_full_element_set() {
        let title = populated_movie_title();
        let (ratings, credits, media_file) =
            (sample_ratings(), sample_credits(), sample_media_file());
        let xml = render_movie_nfo(&title, &populated_context(&ratings, &credits, &media_file));

        assert_eq!(
            xml,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<movie>
  <title>Paper Lantern</title>
  <originaltitle>Kamiandon</originaltitle>
  <sorttitle>Paper Lantern</sorttitle>
  <ratings>
    <rating name="imdb" max="10">
      <value>8.2</value>
      <votes>4211</votes>
    </rating>
    <rating name="themoviedb" max="10" default="true">
      <value>7.6</value>
      <votes>910</votes>
    </rating>
    <rating name="tomatometerallcritics" max="100">
      <value>91</value>
    </rating>
  </ratings>
  <plot>A courier uncovers the secret geometry beneath a flooded megacity.</plot>
  <runtime>136</runtime>
  <premiered>2021-06-04</premiered>
  <year>1999</year>
  <genre>Action</genre>
  <genre>Sci-Fi</genre>
  <tag>favourites</tag>
  <country>Japan</country>
  <studio>Harbor Pals Studio</studio>
  <status>Released</status>
  <uniqueid type="tmdb" default="true">603</uniqueid>
  <uniqueid type="imdb">tt0133093</uniqueid>
  <uniqueid type="tvdb">12345</uniqueid>
  <tmdbid>603</tmdbid>
  <imdbid>tt0133093</imdbid>
  <tvdbid>12345</tvdbid>
  <thumb aspect="poster">https://images.example.invalid/paper-lantern-poster.jpg</thumb>
  <fanart>
    <thumb>https://images.example.invalid/paper-lantern-fanart.jpg</thumb>
  </fanart>
  <actor>
    <name>Ines Calder</name>
    <role>Wren</role>
    <order>0</order>
    <thumb>https://images.example.invalid/ines-calder.jpg</thumb>
  </actor>
  <actor>
    <name>Bo Tamsin</name>
    <order>1</order>
  </actor>
  <director>Mira Vell</director>
  <credits>Otto Renn</credits>
  <fileinfo>
    <streamdetails>
      <video>
        <codec>hevc</codec>
        <aspect>1.78</aspect>
        <width>3840</width>
        <height>2160</height>
        <durationinseconds>6120</durationinseconds>
        <bitrate>18000000</bitrate>
        <hdrtype>dolbyvision</hdrtype>
        <framerate>23.976</framerate>
      </video>
      <audio>
        <codec>truehd_atmos</codec>
        <language>eng</language>
        <channels>8</channels>
        <bitrate>4500000</bitrate>
      </audio>
      <audio>
        <codec>aac_lc</codec>
        <channels>2</channels>
      </audio>
      <subtitle>
        <language>eng</language>
      </subtitle>
    </streamdetails>
  </fileinfo>
  <dateadded>2026-03-14 09:05:01</dateadded>
</movie>
"#
        );
    }

    #[test]
    fn render_tvshow_full_element_set() {
        let title = populated_series_title();
        let (ratings, credits) = (sample_ratings(), sample_credits());
        let xml = render_tvshow_nfo(
            &title,
            &NfoContext {
                ratings: &ratings,
                credits: &credits,
                media_file: None,
                date_added: Some(sample_date_added()),
            },
        );

        assert_eq!(
            xml,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<tvshow>
  <title>Harbor Pals</title>
  <showtitle>Harbor Pals</showtitle>
  <originaltitle>Kamiandon</originaltitle>
  <sorttitle>Harbor Pals</sorttitle>
  <ratings>
    <rating name="imdb" max="10" default="true">
      <value>8.2</value>
      <votes>4211</votes>
    </rating>
    <rating name="themoviedb" max="10">
      <value>7.6</value>
      <votes>910</votes>
    </rating>
    <rating name="tomatometerallcritics" max="100">
      <value>91</value>
    </rating>
  </ratings>
  <plot>A courier uncovers the secret geometry beneath a flooded megacity.</plot>
  <premiered>2019-09-12</premiered>
  <year>1999</year>
  <status>Continuing</status>
  <studio>Lantern Broadcasting</studio>
  <studio>Harbor Pals Studio</studio>
  <country>Japan</country>
  <genre>Action</genre>
  <genre>Sci-Fi</genre>
  <tag>favourites</tag>
  <uniqueid type="tvdb" default="true">12345</uniqueid>
  <uniqueid type="tmdb">603</uniqueid>
  <uniqueid type="imdb">tt0133093</uniqueid>
  <tvdbid>12345</tvdbid>
  <tmdbid>603</tmdbid>
  <imdb_id>tt0133093</imdb_id>
  <episodeguide>
    <uniqueid type="tvdb" default="true">12345</uniqueid>
  </episodeguide>
  <thumb aspect="poster">https://images.example.invalid/paper-lantern-poster.jpg</thumb>
  <fanart>
    <thumb>https://images.example.invalid/paper-lantern-fanart.jpg</thumb>
  </fanart>
  <actor>
    <name>Ines Calder</name>
    <role>Wren</role>
    <order>0</order>
    <thumb>https://images.example.invalid/ines-calder.jpg</thumb>
  </actor>
  <actor>
    <name>Bo Tamsin</name>
    <order>1</order>
  </actor>
  <director>Mira Vell</director>
  <credits>Otto Renn</credits>
  <dateadded>2026-03-14 09:05:01</dateadded>
</tvshow>
"#
        );
    }

    #[test]
    fn render_episode_full_element_set() {
        let title = populated_series_title();
        let mut episode = make_episode();
        episode.title = Some("The Tide Gate".into());
        episode.overview = Some("Wren rows out past the breakwater.".into());
        episode.image_url = Some("https://images.example.invalid/harbor-pals-s01e01.jpg".into());
        let (ratings, credits, media_file) =
            (sample_ratings(), sample_credits(), sample_media_file());
        let xml = render_episode_nfo(
            &title,
            &episode,
            &populated_context(&ratings, &credits, &media_file),
        );

        assert_eq!(
            xml,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<episodedetails>
  <title>The Tide Gate</title>
  <showtitle>Harbor Pals</showtitle>
  <season>1</season>
  <episode>1</episode>
  <aired>2008-01-20</aired>
  <plot>Wren rows out past the breakwater.</plot>
  <runtime>58</runtime>
  <uniqueid type="tvdb" default="true">349232</uniqueid>
  <thumb>https://images.example.invalid/harbor-pals-s01e01.jpg</thumb>
  <actor>
    <name>Ines Calder</name>
    <role>Wren</role>
    <order>0</order>
    <thumb>https://images.example.invalid/ines-calder.jpg</thumb>
  </actor>
  <actor>
    <name>Bo Tamsin</name>
    <order>1</order>
  </actor>
  <director>Mira Vell</director>
  <credits>Otto Renn</credits>
  <fileinfo>
    <streamdetails>
      <video>
        <codec>hevc</codec>
        <aspect>1.78</aspect>
        <width>3840</width>
        <height>2160</height>
        <durationinseconds>6120</durationinseconds>
        <bitrate>18000000</bitrate>
        <hdrtype>dolbyvision</hdrtype>
        <framerate>23.976</framerate>
      </video>
      <audio>
        <codec>truehd_atmos</codec>
        <language>eng</language>
        <channels>8</channels>
        <bitrate>4500000</bitrate>
      </audio>
      <audio>
        <codec>aac_lc</codec>
        <channels>2</channels>
      </audio>
      <subtitle>
        <language>eng</language>
      </subtitle>
    </streamdetails>
  </fileinfo>
  <dateadded>2026-03-14 09:05:01</dateadded>
</episodedetails>
"#
        );
    }

    #[test]
    fn render_series_movie_full_element_set() {
        let movie = scryer_domain::MovieEntity {
            id: "m1".into(),
            title: "Harbor Pals: The Long Night".into(),
            sort_title: Some("Harbor Pals The Long Night".into()),
            slug: None,
            year: Some(2022),
            overview: Some("The crew winters over at the lighthouse.".into()),
            poster_url: None,
            background_url: None,
            language: None,
            runtime_minutes: Some(94),
            content_status: None,
            studio: None,
            digital_release_date: Some("2022-11-18".into()),
            imdb_id: Some("tt7654321".into()),
            tvdb_id: Some("998877".into()),
            tmdb_id: Some("445566".into()),
            mal_id: None,
            anidb_id: None,
            ratings: Some(scryer_domain::TitleRatingSummary {
                rating: Some(7.4),
                rating_sources: vec!["tmdb".into()],
                external_ratings: vec![scryer_domain::TitleExternalRating {
                    source: "tmdb".into(),
                    value: Some(7.4),
                    score: Some(7.4),
                    normalized: 7.4,
                    votes: Some(122),
                    url: String::new(),
                }],
            }),
            credits: Some(vec![scryer_domain::TitleCredit {
                kind: "actor".into(),
                person_name: "Ines Calder".into(),
                character_name: "Wren".into(),
                billing_order: 0,
                ..Default::default()
            }]),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let media_file = sample_media_file();
        let xml = render_series_movie_episode_nfo(
            &movie,
            "S00E03",
            Some(2),
            Some("Harbor Pals"),
            &NfoContext {
                media_file: Some(&media_file),
                date_added: Some(sample_date_added()),
                ..Default::default()
            },
        );

        assert_eq!(
            xml,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<episodedetails>
  <title>Harbor Pals: The Long Night</title>
  <showtitle>Harbor Pals</showtitle>
  <sorttitle>Harbor Pals The Long Night</sorttitle>
  <ratings>
    <rating name="themoviedb" max="10" default="true">
      <value>7.4</value>
      <votes>122</votes>
    </rating>
  </ratings>
  <season>0</season>
  <episode>3</episode>
  <plot>The crew winters over at the lighthouse.</plot>
  <aired>2022-11-18</aired>
  <runtime>94</runtime>
  <airsbefore_season>3</airsbefore_season>
  <airsbefore_episode>1</airsbefore_episode>
  <displayseason>3</displayseason>
  <displayepisode>1</displayepisode>
  <uniqueid type="tvdb" default="true">998877</uniqueid>
  <uniqueid type="imdb">tt7654321</uniqueid>
  <uniqueid type="tmdb">445566</uniqueid>
  <actor>
    <name>Ines Calder</name>
    <role>Wren</role>
    <order>0</order>
  </actor>
  <fileinfo>
    <streamdetails>
      <video>
        <codec>hevc</codec>
        <aspect>1.78</aspect>
        <width>3840</width>
        <height>2160</height>
        <durationinseconds>6120</durationinseconds>
        <bitrate>18000000</bitrate>
        <hdrtype>dolbyvision</hdrtype>
        <framerate>23.976</framerate>
      </video>
      <audio>
        <codec>truehd_atmos</codec>
        <language>eng</language>
        <channels>8</channels>
        <bitrate>4500000</bitrate>
      </audio>
      <audio>
        <codec>aac_lc</codec>
        <channels>2</channels>
      </audio>
      <subtitle>
        <language>eng</language>
      </subtitle>
    </streamdetails>
  </fileinfo>
  <dateadded>2026-03-14 09:05:01</dateadded>
</episodedetails>
"#
        );
    }

    // -----------------------------------------------------------------------
    // Writer tests — minimal data emits no empty elements
    // -----------------------------------------------------------------------

    fn bare_title(facet: MediaFacet) -> Title {
        let mut title = make_title();
        title.facet = facet;
        title.name = "Paper Lantern".into();
        title.year = None;
        title.overview = None;
        title.runtime_minutes = None;
        title.studio = None;
        title.canonical_tags = Vec::new();
        title.external_ids = Vec::new();
        title.imdb_id = None;
        title
    }

    /// Nothing an NFO reader can mistake for a known-empty fact: an element
    /// with no text asserts "this value is the empty string", which is a
    /// different claim from leaving it out.
    fn assert_no_empty_elements(xml: &str) {
        for line in xml.lines() {
            let line = line.trim();
            assert!(
                !line.contains("></"),
                "empty element in rendered NFO: {line}\n{xml}"
            );
            assert!(
                !line.ends_with("/>") || line.starts_with("<?xml"),
                "self-closing empty element in rendered NFO: {line}\n{xml}"
            );
        }
    }

    #[test]
    fn render_movie_minimal_emits_no_empty_elements() {
        let title = bare_title(MediaFacet::Movie);
        let xml = render_movie_nfo(&title, &NfoContext::default());
        assert_eq!(
            xml,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<movie>
  <title>Paper Lantern</title>
</movie>
"#
        );
        assert_no_empty_elements(&xml);
    }

    #[test]
    fn render_tvshow_minimal_emits_no_empty_elements() {
        let title = bare_title(MediaFacet::Series);
        let xml = render_tvshow_nfo(&title, &NfoContext::default());
        assert_eq!(
            xml,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<tvshow>
  <title>Paper Lantern</title>
  <showtitle>Paper Lantern</showtitle>
</tvshow>
"#
        );
        assert_no_empty_elements(&xml);
    }

    #[test]
    fn render_episode_minimal_emits_no_empty_elements() {
        let title = bare_title(MediaFacet::Series);
        let episode = Episode {
            id: "e1".into(),
            title_id: "t1".into(),
            collection_id: None,
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("4".into()),
            season_number: Some("2".into()),
            episode_label: None,
            title: None,
            air_date: None,
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        let xml = render_episode_nfo(&title, &episode, &NfoContext::default());
        assert_eq!(
            xml,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<episodedetails>
  <showtitle>Paper Lantern</showtitle>
  <season>2</season>
  <episode>4</episode>
</episodedetails>
"#
        );
        assert_no_empty_elements(&xml);
    }

    #[test]
    fn render_series_movie_minimal_emits_no_empty_elements() {
        let movie = scryer_domain::MovieEntity {
            id: "m1".into(),
            title: "The Long Night".into(),
            sort_title: None,
            slug: None,
            year: None,
            overview: None,
            poster_url: None,
            background_url: None,
            language: None,
            runtime_minutes: None,
            content_status: None,
            studio: None,
            digital_release_date: None,
            imdb_id: None,
            tvdb_id: None,
            tmdb_id: None,
            mal_id: None,
            anidb_id: None,
            ratings: None,
            credits: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let xml =
            render_series_movie_episode_nfo(&movie, "S00E01", None, None, &NfoContext::default());
        assert_eq!(
            xml,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<episodedetails>
  <title>The Long Night</title>
  <season>0</season>
  <episode>1</episode>
</episodedetails>
"#
        );
        assert_no_empty_elements(&xml);
    }

    #[test]
    fn stream_details_are_omitted_when_the_file_was_never_analyzed() {
        let title = populated_movie_title();
        let unscanned = crate::TitleMediaFile {
            id: "mf-unscanned".into(),
            ..Default::default()
        };
        let xml = render_movie_nfo(
            &title,
            &NfoContext {
                media_file: Some(&unscanned),
                ..Default::default()
            },
        );
        assert!(!xml.contains("<fileinfo>"), "{xml}");
        assert!(!xml.contains("<streamdetails>"), "{xml}");
        assert_no_empty_elements(&xml);
    }

    // -----------------------------------------------------------------------
    // Multi-episode files
    // -----------------------------------------------------------------------

    #[test]
    fn multi_episode_file_renders_one_root_per_episode() {
        let title = populated_series_title();
        let mut first = make_episode();
        first.episode_number = Some("27".into());
        first.title = Some("The Tide Gate".into());
        first.tvdb_id = Some("500027".into());
        let mut second = make_episode();
        second.id = "e2".into();
        second.episode_number = Some("28".into());
        second.title = Some("Slack Water".into());
        second.tvdb_id = Some("500028".into());

        let xml = render_episodes_nfo(&title, &[first, second], &NfoContext::default());

        assert_eq!(xml.matches("<episodedetails>").count(), 2, "{xml}");
        assert_eq!(xml.matches("</episodedetails>").count(), 2, "{xml}");
        // One declaration for the file, not one per root.
        assert_eq!(xml.matches("<?xml").count(), 1, "{xml}");
        assert!(xml.contains("<episode>27</episode>"), "{xml}");
        assert!(xml.contains("<episode>28</episode>"), "{xml}");
        assert!(xml.contains("<title>The Tide Gate</title>"), "{xml}");
        assert!(xml.contains("<title>Slack Water</title>"), "{xml}");
        assert!(
            xml.contains(r#"<uniqueid type="tvdb" default="true">500027</uniqueid>"#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"<uniqueid type="tvdb" default="true">500028</uniqueid>"#),
            "{xml}"
        );
        // Jellyfin's EpisodeNfoParser splits the file on this exact token, so
        // each block has to be independently parseable.
        for block in xml.split_inclusive("</episodedetails>") {
            if block.trim().is_empty() {
                continue;
            }
            assert_eq!(detect_nfo_root_kind(block), NfoRootKind::Episode, "{block}");
        }
    }

    #[test]
    fn episode_uniqueid_never_falls_back_to_the_series_id() {
        let title = populated_series_title();
        let series_tvdb_id = title_external_id_value(&title, "tvdb")
            .expect("fixture series carries a tvdb id")
            .to_string();
        let mut episode = make_episode();
        episode.tvdb_id = None;

        let xml = render_episode_nfo(&title, &episode, &NfoContext::default());

        assert!(
            !xml.contains("<uniqueid"),
            "an episode with no id of its own must carry none: {xml}"
        );
        assert!(
            !xml.contains(&series_tvdb_id),
            "the series id must never be written as an episode id: {xml}"
        );
    }

    // -----------------------------------------------------------------------
    // streamdetails mapping
    // -----------------------------------------------------------------------

    #[test]
    fn rating_scales_match_what_each_reader_expects() {
        // Jellyfin only reads a 0-100 rating out of a name it routes into
        // CriticRating; everything else it reads as a 0-10 community rating,
        // whatever `max=` claims.
        for (source, name, percent) in [
            ("rottentomatoes", "tomatometerallcritics", true),
            ("tomatoes", "tomatometerallcritics", true),
            ("audience", "tomatometerallaudience", false),
            ("popcornmeter", "tomatometerallaudience", false),
            ("metacritic", "metacritic", false),
            ("mcuser", "metacritic", false),
            ("tmdb", "themoviedb", false),
            ("imdb", "imdb", false),
            ("trakt", "trakt", false),
        ] {
            assert_eq!(kodi_rating_name(source), name, "source={source}");
            assert_eq!(
                rating_name_is_percent_scale(name),
                percent,
                "source={source}"
            );
        }
    }

    #[test]
    fn video_codecs_map_to_kodi_names() {
        use crate::release_parser::VideoCodec;
        for (codec, expected) in [
            (VideoCodec::H264, "h264"),
            (VideoCodec::H265, "hevc"),
            (VideoCodec::Av1, "av1"),
            (VideoCodec::Vp9, "vp9"),
            (VideoCodec::Vc1, "vc1"),
            (VideoCodec::Mpeg2, "mpeg2video"),
            (VideoCodec::Mpeg4, "mpeg4"),
            (VideoCodec::Xvid, "xvid"),
            (VideoCodec::Divx, "divx"),
            (VideoCodec::Vvc, "vvc"),
        ] {
            assert_eq!(kodi_video_codec(codec), expected);
        }
    }

    #[test]
    fn audio_codecs_follow_sonarrs_profile_refinement() {
        for (codec, profile, expected) in [
            (
                Some("truehd"),
                Some("Dolby TrueHD + Dolby Atmos"),
                Some("truehd_atmos"),
            ),
            (Some("truehd"), None, Some("truehd")),
            (
                Some("eac3"),
                Some("Dolby Digital Plus + Dolby Atmos"),
                Some("eac3_ddp_atmos"),
            ),
            (Some("eac3"), Some("unknown profile"), Some("eac3")),
            (Some("dts"), Some("DTS-HD MA"), Some("dtshd_ma")),
            (Some("dts"), Some("DTS-HD MA + DTS:X"), Some("dtshd_ma_x")),
            (Some("dts"), Some("DTS-HD HRA"), Some("dtshd_hra")),
            (Some("aac"), Some("LC"), Some("aac_lc")),
            (Some("aac"), Some("HE-AACv2"), Some("he_aac_v2")),
            (Some("AC3"), None, Some("ac3")),
            (Some("  "), None, None),
            (None, Some("LC"), None),
        ] {
            assert_eq!(
                kodi_audio_codec(codec, profile).as_deref(),
                expected,
                "codec={codec:?} profile={profile:?}"
            );
        }
    }

    #[test]
    fn hdr_formats_map_to_kodis_vocabulary() {
        for (stored, expected) in [
            ("Dolby Vision", Some("dolbyvision")),
            ("HDR10", Some("hdr10")),
            ("HDR10+", Some("hdr10")),
            ("PQ10", Some("hdr10")),
            ("HLG", Some("hlg")),
            ("hlg", Some("hlg")),
            ("SDR", None),
            ("", None),
        ] {
            assert_eq!(kodi_hdr_type(stored), expected, "stored={stored}");
        }
    }

    #[test]
    fn aspect_ratio_is_width_over_height_to_two_decimals() {
        assert_eq!(
            stream_aspect_ratio(Some(1920), Some(1080)).as_deref(),
            Some("1.78")
        );
        assert_eq!(
            stream_aspect_ratio(Some(1920), Some(800)).as_deref(),
            Some("2.40")
        );
        assert_eq!(
            stream_aspect_ratio(Some(720), Some(576)).as_deref(),
            Some("1.25")
        );
        assert_eq!(stream_aspect_ratio(Some(1920), Some(0)), None);
        assert_eq!(stream_aspect_ratio(None, Some(1080)), None);
    }

    #[test]
    fn bitrates_are_written_in_bits_per_second() {
        let title = populated_movie_title();
        let media_file = crate::TitleMediaFile {
            video_bitrate_kbps: Some(9_375),
            video_width: Some(1920),
            video_height: Some(1080),
            audio_streams: vec![crate::AudioStreamDetail {
                codec: Some("ac3".into()),
                profile: None,
                channels: Some(6),
                language: Some("eng".into()),
                name: None,
                bitrate_kbps: Some(640),
            }],
            ..Default::default()
        };
        let xml = render_movie_nfo(
            &title,
            &NfoContext {
                media_file: Some(&media_file),
                ..Default::default()
            },
        );
        assert!(xml.contains("<bitrate>9375000</bitrate>"), "{xml}");
        assert!(xml.contains("<bitrate>640000</bitrate>"), "{xml}");
    }

    #[test]
    fn undefined_stream_languages_are_omitted_rather_than_written_as_und() {
        assert_eq!(stream_language(Some("eng")), Some("eng"));
        assert_eq!(stream_language(Some(" jpn ")), Some("jpn"));
        assert_eq!(stream_language(Some("und")), None);
        assert_eq!(stream_language(Some("UND")), None);
        assert_eq!(stream_language(Some("")), None);
        assert_eq!(stream_language(None), None);
    }

    #[test]
    fn reserved_scryer_tags_never_reach_the_sidecar() {
        let mut title = populated_movie_title();
        title.tags = vec![
            "scryer:filler-policy:skip".into(),
            "weeknight".into(),
            "  ".into(),
        ];
        let xml = render_movie_nfo(&title, &NfoContext::default());
        assert!(xml.contains("<tag>weeknight</tag>"), "{xml}");
        assert!(!xml.contains("scryer:"), "{xml}");
        assert_eq!(xml.matches("<tag>").count(), 1, "{xml}");
    }

    #[test]
    fn artwork_is_written_only_for_absolute_urls() {
        let mut title = populated_movie_title();
        title.poster_source_url = None;
        title.background_source_url = None;
        title.poster_url = Some("/cache/posters/paper-lantern.jpg".into());
        title.background_url = Some("images/fanart.jpg".into());
        let xml = render_movie_nfo(&title, &NfoContext::default());
        assert!(!xml.contains("<thumb"), "{xml}");
        assert!(!xml.contains("<fanart>"), "{xml}");
    }

    #[test]
    fn original_title_requires_an_alias_in_the_titles_own_language() {
        let mut title = populated_movie_title();
        // A regional alias in another language is not the original title.
        title.language = Some("en".into());
        let xml = render_movie_nfo(&title, &NfoContext::default());
        assert!(!xml.contains("<originaltitle>"), "{xml}");

        // Nor is an alias that merely repeats the display name.
        title.language = Some("ja".into());
        title.tagged_aliases = vec![scryer_domain::TaggedAlias {
            name: "Paper Lantern".into(),
            language: "ja".into(),
        }];
        let xml = render_movie_nfo(&title, &NfoContext::default());
        assert!(!xml.contains("<originaltitle>"), "{xml}");
    }

    #[test]
    fn episode_guide_is_omitted_without_a_tvdb_id() {
        let mut title = populated_series_title();
        title.external_ids.retain(|id| id.source != "tvdb");
        let xml = render_tvshow_nfo(&title, &NfoContext::default());
        assert!(!xml.contains("<episodeguide"), "{xml}");
    }

    // -----------------------------------------------------------------------
    // Round-trip: the read side accepts what the write side produces
    // -----------------------------------------------------------------------

    #[test]
    fn rendered_movie_nfo_round_trips_through_the_parser() {
        let title = populated_movie_title();
        let (ratings, credits, media_file) =
            (sample_ratings(), sample_credits(), sample_media_file());
        let xml = render_movie_nfo(&title, &populated_context(&ratings, &credits, &media_file));

        assert_eq!(detect_nfo_root_kind(&xml), NfoRootKind::Movie);
        let meta = parse_nfo(&xml);
        assert_eq!(meta.title.as_deref(), Some("Paper Lantern"));
        assert_eq!(meta.year, Some(1999));
        assert_eq!(meta.tmdb_id.as_deref(), Some("603"));
        assert_eq!(meta.imdb_id.as_deref(), Some("tt0133093"));
        assert_eq!(meta.tvdb_id.as_deref(), Some("12345"));
    }

    #[test]
    fn rendered_tvshow_nfo_round_trips_through_the_parser() {
        let title = populated_series_title();
        let (ratings, credits) = (sample_ratings(), sample_credits());
        let xml = render_tvshow_nfo(
            &title,
            &NfoContext {
                ratings: &ratings,
                credits: &credits,
                ..Default::default()
            },
        );

        assert_eq!(detect_nfo_root_kind(&xml), NfoRootKind::TvShow);
        let meta = parse_nfo(&xml);
        assert_eq!(meta.title.as_deref(), Some("Harbor Pals"));
        assert_eq!(meta.year, Some(1999));
        assert_eq!(meta.tvdb_id.as_deref(), Some("12345"));
        assert_eq!(meta.tmdb_id.as_deref(), Some("603"));
        assert_eq!(meta.imdb_id.as_deref(), Some("tt0133093"));
    }

    #[test]
    fn rendered_episode_nfo_carries_no_title_identity_into_the_parser() {
        let title = populated_series_title();
        let episode = make_episode();
        let xml = render_episode_nfo(&title, &episode, &NfoContext::default());

        assert_eq!(detect_nfo_root_kind(&xml), NfoRootKind::Episode);
        // Episode documents must never be read as title identity: the parser
        // drops their ids, and the renderer never writes the series' own.
        let meta = parse_nfo(&xml);
        assert!(!meta.has_external_ids(), "{meta:?}");
    }

    // -----------------------------------------------------------------------
    // Write behaviour
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn write_nfo_if_absent_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Harbor Pals - S01E01.nfo");

        let outcome = write_nfo_if_absent(&path, "<episodedetails/>\n").await;

        assert_eq!(outcome, NfoWriteOutcome::Written);
        assert_eq!(stdfs::read_to_string(&path).unwrap(), "<episodedetails/>\n");
    }

    #[tokio::test]
    async fn write_nfo_if_absent_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("Season 01")
            .join("Harbor Pals - S01E01.nfo");

        let outcome = write_nfo_if_absent(&path, "<episodedetails/>\n").await;

        assert_eq!(outcome, NfoWriteOutcome::Written);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn write_nfo_if_absent_never_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tvshow.nfo");
        let existing = "<tvshow>\n  <title>Written by someone else</title>\n</tvshow>\n";
        stdfs::write(&path, existing).unwrap();

        let outcome = write_nfo_if_absent(&path, "<tvshow><title>Ours</title></tvshow>\n").await;

        assert_eq!(outcome, NfoWriteOutcome::SkippedExisting);
        assert_eq!(stdfs::read_to_string(&path).unwrap(), existing);
    }

    #[tokio::test]
    async fn write_nfo_if_absent_skips_an_existing_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tvshow.nfo");
        stdfs::write(&path, "").unwrap();

        let outcome = write_nfo_if_absent(&path, "<tvshow/>\n").await;

        assert_eq!(outcome, NfoWriteOutcome::SkippedExisting);
        assert_eq!(stdfs::read_to_string(&path).unwrap(), "");
    }

    /// The exists check and the write are not atomic together, so the write
    /// itself has to fail closed. `create_new` is what makes a file that
    /// appears in between the two survive.
    #[tokio::test]
    async fn write_nfo_if_absent_loses_the_race_rather_than_clobbering() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("movie.nfo");
        let racer = "<movie><title>Arrived first</title></movie>\n";

        let mut handles = Vec::new();
        for index in 0..8 {
            let path = path.clone();
            handles.push(tokio::spawn(async move {
                if index == 0 {
                    stdfs::write(&path, racer).unwrap();
                    return NfoWriteOutcome::SkippedExisting;
                }
                write_nfo_if_absent(
                    &path,
                    &format!("<movie><title>Racer {index}</title></movie>\n"),
                )
                .await
            }));
        }
        let mut outcomes = Vec::new();
        for handle in handles {
            outcomes.push(handle.await.unwrap());
        }

        assert!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == NfoWriteOutcome::Written)
                .count()
                <= 1,
            "at most one writer may create the file: {outcomes:?}"
        );
        let landed = stdfs::read_to_string(&path).unwrap();
        assert!(
            landed == racer || landed.starts_with("<movie><title>Racer "),
            "a partial or merged file landed: {landed}"
        );
    }

    /// Everything is staged beside the destination, so a successful write must
    /// not leave the staging file behind for a library scan to trip over.
    #[tokio::test]
    async fn write_nfo_if_absent_leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Harbor Pals - S01E01.nfo");

        let outcome = write_nfo_if_absent(&path, "<episodedetails/>\n").await;

        assert_eq!(outcome, NfoWriteOutcome::Written);
        let entries = stdfs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1, "{entries:?}");
    }

    /// The publish step is a link, not a rename, so a file that appeared while
    /// the content was being staged keeps its own bytes.
    #[tokio::test]
    async fn write_nfo_if_absent_publishing_never_clobbers_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tvshow.nfo");
        let existing = "<tvshow>\n  <title>Written by someone else</title>\n</tvshow>\n";

        // Enter at the publish step the way a writer does when the file
        // appeared after its own existence check passed.
        let ours = "<tvshow><title>Ours</title></tvshow>\n";
        let temp_path = nfo_temp_path(&path);
        write_nfo_temp_file(&temp_path, ours).await.unwrap();
        stdfs::write(&path, existing).unwrap();

        let outcome = publish_staged_nfo(&temp_path, &path, ours).await;

        assert_eq!(outcome, NfoWriteOutcome::SkippedExisting);
        assert_eq!(stdfs::read_to_string(&path).unwrap(), existing);
        assert!(!temp_path.exists(), "the staging file outlived the publish");
    }

    /// A destination whose directory is gone fails without leaving anything
    /// half-written beside it.
    #[tokio::test]
    async fn write_nfo_if_absent_fails_cleanly_when_the_directory_is_unwritable() {
        let dir = tempfile::tempdir().unwrap();
        // A path whose parent is an existing *file* can be neither created nor
        // staged into, which is the portable stand-in for a mount that dropped.
        let blocker = dir.path().join("Season 01");
        stdfs::write(&blocker, "not a directory").unwrap();
        let path = blocker.join("Harbor Pals - S01E01.nfo");

        let outcome = write_nfo_if_absent(&path, "<episodedetails/>\n").await;

        assert_eq!(outcome, NfoWriteOutcome::Failed);
        assert!(!path.exists());
        assert_eq!(stdfs::read_to_string(&blocker).unwrap(), "not a directory");
    }

    /// The fallback taken when the filesystem has no hard links is no-clobber
    /// in its own right.
    #[tokio::test]
    async fn write_nfo_in_place_never_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tvshow.nfo");
        let existing = "<tvshow>\n  <title>Written by someone else</title>\n</tvshow>\n";
        stdfs::write(&path, existing).unwrap();

        let outcome = write_nfo_in_place(&path, "<tvshow><title>Ours</title></tvshow>\n").await;

        assert_eq!(outcome, NfoWriteOutcome::SkippedExisting);
        assert_eq!(stdfs::read_to_string(&path).unwrap(), existing);
    }

    #[tokio::test]
    async fn write_nfo_in_place_writes_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tvshow.nfo");

        let outcome = write_nfo_in_place(&path, "<tvshow/>\n").await;

        assert_eq!(outcome, NfoWriteOutcome::Written);
        assert_eq!(stdfs::read_to_string(&path).unwrap(), "<tvshow/>\n");
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
