use scryer_application::{AppError, AppResult, PrimaryCollectionSummary, TitleMetadataUpdate};
use scryer_domain::{
    CalendarEpisode, Collection, CollectionType, Episode, ExternalId, InterstitialMovieMetadata,
    MediaFacet, Title,
};
use serde_json;
use sqlx::{Row, SqlitePool};
use std::collections::HashSet;

use super::common::parse_utc_datetime;
use crate::title_images::{apply_local_image_urls, apply_local_poster_urls};
use scryer_application::TitleImageKind;

const TITLE_COLUMNS: &str = "id, name, facet, monitored, tags, external_ids, created_by, created_at, \
    year, overview, poster_url, banner_url, background_url, sort_title, slug, imdb_id, runtime_minutes, genres, \
    content_status, language, first_aired, network, studio, country, aliases, \
    metadata_language, metadata_fetched_at, min_availability, digital_release_date, folder_path, tagged_aliases_json";

fn parse_facet(raw: &str) -> MediaFacet {
    MediaFacet::parse(raw).unwrap_or_default()
}

pub(crate) async fn list_titles_query(
    pool: &SqlitePool,
    facet: Option<MediaFacet>,
    query: Option<String>,
) -> AppResult<Vec<Title>> {
    let mut sql = format!("SELECT {} FROM titles", TITLE_COLUMNS);

    let mut where_clauses = Vec::new();
    if facet.is_some() {
        where_clauses.push("facet = ?");
    }
    if query.is_some() {
        where_clauses.push("LOWER(name) LIKE ?");
    }

    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY LOWER(name) ASC, id ASC");

    let mut statement = sqlx::query(&sql);

    if let Some(selected_facet) = facet {
        statement = statement.bind(selected_facet.as_str());
    }
    if let Some(search) = query {
        statement = statement.bind(format!("%{}%", search.to_lowercase()));
    }

    let rows = statement
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_title(&row)?);
    }
    apply_local_poster_urls(pool, &mut out).await?;
    apply_local_image_urls(pool, TitleImageKind::Banner, "master", &mut out).await?;
    apply_local_image_urls(pool, TitleImageKind::Fanart, "master", &mut out).await?;
    Ok(out)
}

pub(crate) async fn list_unhydrated_titles_query(
    pool: &SqlitePool,
    limit: usize,
    language: &str,
) -> AppResult<Vec<Title>> {
    let sql = format!(
        "SELECT {} FROM titles WHERE metadata_fetched_at IS NULL OR metadata_language IS NULL OR metadata_language != ? ORDER BY created_at ASC LIMIT ?",
        TITLE_COLUMNS
    );
    let rows = sqlx::query(&sql)
        .bind(language)
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_title(&row)?);
    }
    Ok(out)
}

pub(crate) async fn clear_metadata_language_for_all_query(pool: &SqlitePool) -> AppResult<u64> {
    let result = sqlx::query(
        "UPDATE titles SET metadata_language = NULL WHERE metadata_language IS NOT NULL",
    )
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(result.rows_affected())
}

pub(crate) async fn get_title_by_id_query(pool: &SqlitePool, id: &str) -> AppResult<Option<Title>> {
    let sql = format!("SELECT {} FROM titles WHERE id = ?", TITLE_COLUMNS);
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => {
            let mut titles = vec![row_to_title(&row)?];
            apply_local_poster_urls(pool, &mut titles).await?;
            apply_local_image_urls(pool, TitleImageKind::Banner, "master", &mut titles).await?;
            apply_local_image_urls(pool, TitleImageKind::Fanart, "master", &mut titles).await?;
            Ok(titles.into_iter().next())
        }
        None => Ok(None),
    }
}

pub(crate) async fn get_title_by_external_id_query(
    pool: &SqlitePool,
    source: &str,
    value: &str,
) -> AppResult<Option<Title>> {
    let sql = format!(
        "SELECT {} FROM titles
         WHERE EXISTS (
             SELECT 1
             FROM json_each(titles.external_ids) AS external_id
             WHERE LOWER(json_extract(external_id.value, '$.source')) = LOWER(?)
               AND json_extract(external_id.value, '$.value') = ?
         )
         ORDER BY id ASC
         LIMIT 1",
        TITLE_COLUMNS
    );

    let row = sqlx::query(&sql)
        .bind(source)
        .bind(value)
        .fetch_optional(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => {
            let mut titles = vec![row_to_title(&row)?];
            apply_local_poster_urls(pool, &mut titles).await?;
            apply_local_image_urls(pool, TitleImageKind::Banner, "master", &mut titles).await?;
            apply_local_image_urls(pool, TitleImageKind::Fanart, "master", &mut titles).await?;
            Ok(titles.into_iter().next())
        }
        None => Ok(None),
    }
}

fn row_to_title(row: &sqlx::sqlite::SqliteRow) -> AppResult<Title> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let name: String = row
        .try_get("name")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let facet: String = row
        .try_get("facet")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let monitored: i64 = row
        .try_get("monitored")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let tags_json: String = row
        .try_get("tags")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let external_ids_json: String = row
        .try_get("external_ids")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_by: Option<String> = row
        .try_get("created_by")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let tags: Vec<String> =
        serde_json::from_str(&tags_json).map_err(|err| AppError::Repository(err.to_string()))?;
    let external_ids: Vec<ExternalId> = serde_json::from_str(&external_ids_json)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at = parse_utc_datetime(&created_at_raw)?;

    // metadata fields
    let year: Option<i32> = row.try_get("year").unwrap_or(None);
    let overview: Option<String> = row.try_get("overview").unwrap_or(None);
    let poster_url: Option<String> = row.try_get("poster_url").unwrap_or(None);
    let banner_url: Option<String> = row.try_get("banner_url").unwrap_or(None);
    let background_url: Option<String> = row.try_get("background_url").unwrap_or(None);
    let sort_title: Option<String> = row.try_get("sort_title").unwrap_or(None);
    let slug: Option<String> = row.try_get("slug").unwrap_or(None);
    let imdb_id: Option<String> = row.try_get("imdb_id").unwrap_or(None);
    let runtime_minutes: Option<i32> = row.try_get("runtime_minutes").unwrap_or(None);
    let genres_json: String = row.try_get("genres").unwrap_or_else(|_| "[]".to_string());
    let content_status: Option<String> = row.try_get("content_status").unwrap_or(None);
    let language: Option<String> = row.try_get("language").unwrap_or(None);
    let first_aired: Option<String> = row.try_get("first_aired").unwrap_or(None);
    let network: Option<String> = row.try_get("network").unwrap_or(None);
    let studio: Option<String> = row.try_get("studio").unwrap_or(None);
    let country: Option<String> = row.try_get("country").unwrap_or(None);
    let aliases_json: String = row.try_get("aliases").unwrap_or_else(|_| "[]".to_string());
    let metadata_language: Option<String> = row.try_get("metadata_language").unwrap_or(None);
    let metadata_fetched_at_raw: Option<String> =
        row.try_get("metadata_fetched_at").unwrap_or(None);
    let min_availability: Option<String> = row.try_get("min_availability").unwrap_or(None);
    let digital_release_date: Option<String> = row.try_get("digital_release_date").unwrap_or(None);
    let folder_path: Option<String> = row.try_get("folder_path").unwrap_or(None);

    let genres: Vec<String> =
        serde_json::from_str(&genres_json).map_err(|err| AppError::Repository(err.to_string()))?;
    let aliases: Vec<String> =
        serde_json::from_str(&aliases_json).map_err(|err| AppError::Repository(err.to_string()))?;
    let metadata_fetched_at = match metadata_fetched_at_raw {
        Some(raw) => Some(parse_utc_datetime(&raw)?),
        None => None,
    };

    Ok(Title {
        id,
        name,
        facet: parse_facet(&facet),
        monitored: monitored != 0,
        tags,
        external_ids,
        created_by,
        created_at,
        year,
        overview,
        poster_url,
        poster_source_url: None,
        banner_url,
        banner_source_url: None,
        background_url,
        background_source_url: None,
        sort_title,
        slug,
        imdb_id,
        runtime_minutes,
        genres,
        content_status,
        language,
        first_aired,
        network,
        studio,
        country,
        aliases,
        tagged_aliases: {
            let raw: Option<String> = row.try_get("tagged_aliases_json").unwrap_or(None);
            raw.as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default()
        },
        metadata_language,
        metadata_fetched_at,
        min_availability,
        digital_release_date,
        folder_path,
    })
}

pub(crate) async fn list_collections_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<Collection>> {
    let rows = sqlx::query(
        "SELECT id, title_id, collection_type, collection_index, label, ordered_path,
                narrative_order, first_episode_number, last_episode_number,
                interstitial_tvdb_id, interstitial_name, interstitial_slug, interstitial_year,
                interstitial_content_status, interstitial_overview, interstitial_poster_url,
                interstitial_language, interstitial_runtime_minutes, interstitial_sort_title,
                interstitial_imdb_id, interstitial_genres_json, interstitial_studio,
                interstitial_digital_release_date, interstitial_association_confidence,
                interstitial_continuity_status, interstitial_movie_form, interstitial_confidence,
                interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id,
                interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode,
                special_movies_json, monitored, created_at
         FROM collections WHERE title_id = ? ORDER BY collection_index ASC, id ASC",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_collection(&row)?);
    }
    Ok(out)
}

pub(crate) async fn list_primary_collection_summaries_query(
    pool: &SqlitePool,
    title_ids: &[String],
) -> AppResult<Vec<PrimaryCollectionSummary>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: String = title_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT title_id, collection_type, collection_index, label, ordered_path FROM collections \
         WHERE title_id IN ({placeholders}) AND (collection_index = '0' OR collection_type = 'movie')"
    );

    let mut query = sqlx::query(&sql);
    for id in title_ids {
        query = query.bind(id);
    }

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut candidates = rows
        .into_iter()
        .map(|row| {
            let raw_type: String = row.get("collection_type");
            SummaryCandidate {
                title_id: row.get("title_id"),
                collection_type: CollectionType::parse(&raw_type).unwrap_or_default(),
                collection_index: row.get("collection_index"),
                label: row.get("label"),
                ordered_path: row.get("ordered_path"),
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(summary_candidate_sort_key);

    let mut seen = HashSet::new();
    let mut summaries = Vec::new();
    for candidate in candidates {
        if seen.contains(candidate.title_id.as_str()) {
            continue;
        }
        if !summary_candidate_should_include(&candidate) {
            continue;
        }
        seen.insert(candidate.title_id.clone());
        summaries.push(PrimaryCollectionSummary {
            title_id: candidate.title_id,
            label: candidate.label,
            ordered_path: candidate.ordered_path,
        });
    }

    Ok(summaries)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SummaryCandidate {
    title_id: String,
    collection_type: CollectionType,
    collection_index: String,
    label: Option<String>,
    ordered_path: Option<String>,
}

fn summary_candidate_should_include(candidate: &SummaryCandidate) -> bool {
    if candidate.collection_type == CollectionType::Movie {
        return true;
    }
    candidate.collection_index.trim() == "0"
}

fn summary_candidate_sort_key(candidate: &SummaryCandidate) -> (String, bool, bool, u32, String) {
    (
        candidate.title_id.clone(),
        candidate.collection_type != CollectionType::Movie,
        candidate
            .ordered_path
            .as_deref()
            .is_none_or(|path| path.trim().is_empty()),
        candidate
            .collection_index
            .parse::<u32>()
            .unwrap_or(u32::MAX),
        candidate.collection_index.clone(),
    )
}

pub(crate) async fn get_collection_by_id_query(
    pool: &SqlitePool,
    collection_id: &str,
) -> AppResult<Option<Collection>> {
    let row = sqlx::query(
        "SELECT id, title_id, collection_type, collection_index, label, ordered_path,
                narrative_order, first_episode_number, last_episode_number,
                interstitial_tvdb_id, interstitial_name, interstitial_slug, interstitial_year,
                interstitial_content_status, interstitial_overview, interstitial_poster_url,
                interstitial_language, interstitial_runtime_minutes, interstitial_sort_title,
                interstitial_imdb_id, interstitial_genres_json, interstitial_studio,
                interstitial_digital_release_date, interstitial_association_confidence,
                interstitial_continuity_status, interstitial_movie_form, interstitial_confidence,
                interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id,
                interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode,
                special_movies_json, monitored, created_at
         FROM collections WHERE id = ?",
    )
    .bind(collection_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_collection(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn create_collection_query(
    pool: &SqlitePool,
    collection: &Collection,
) -> AppResult<Collection> {
    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, ordered_path, narrative_order,
          first_episode_number, last_episode_number, interstitial_tvdb_id, interstitial_name,
          interstitial_slug, interstitial_year, interstitial_content_status,
          interstitial_overview, interstitial_poster_url, interstitial_language,
          interstitial_runtime_minutes, interstitial_sort_title, interstitial_imdb_id,
          interstitial_genres_json, interstitial_studio, interstitial_digital_release_date,
          interstitial_association_confidence, interstitial_continuity_status,
          interstitial_movie_form, interstitial_confidence, interstitial_signal_summary,
          interstitial_placement, interstitial_movie_tmdb_id, interstitial_movie_mal_id,
          interstitial_movie_anidb_id, interstitial_season_episode, special_movies_json, monitored, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&collection.id)
    .bind(&collection.title_id)
    .bind(collection.collection_type.as_str())
    .bind(&collection.collection_index)
    .bind(&collection.label)
    .bind(&collection.ordered_path)
    .bind(&collection.narrative_order)
    .bind(&collection.first_episode_number)
    .bind(&collection.last_episode_number)
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.tvdb_id.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.name.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.slug.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.year),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.content_status.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.overview.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.poster_url.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.language.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.runtime_minutes),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.sort_title.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.imdb_id.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| serde_json::to_string(&movie.genres).unwrap_or_else(|_| "[]".to_string())),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.studio.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.digital_release_date.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.association_confidence.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.continuity_status.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.movie_form.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.confidence.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.signal_summary.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.placement.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.movie_tmdb_id.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.movie_mal_id.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.movie_anidb_id.clone()),
    )
    .bind(&collection.interstitial_season_episode)
    .bind(serde_json::to_string(&collection.specials_movies).unwrap_or_else(|_| "[]".to_string()))
    .bind(if collection.monitored { 1_i64 } else { 0_i64 })
    .bind(collection.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(collection.clone())
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn update_collection_query(
    pool: &SqlitePool,
    collection_id: &str,
    collection_type: Option<CollectionType>,
    collection_index: Option<String>,
    label: Option<String>,
    ordered_path: Option<String>,
    first_episode_number: Option<String>,
    last_episode_number: Option<String>,
    monitored: Option<bool>,
) -> AppResult<Collection> {
    let mut assignments = Vec::new();
    if collection_type.is_some() {
        assignments.push("collection_type = ?");
    }
    if collection_index.is_some() {
        assignments.push("collection_index = ?");
    }
    if label.is_some() {
        assignments.push("label = ?");
    }
    if ordered_path.is_some() {
        assignments.push("ordered_path = ?");
    }
    if first_episode_number.is_some() {
        assignments.push("first_episode_number = ?");
    }
    if last_episode_number.is_some() {
        assignments.push("last_episode_number = ?");
    }
    if monitored.is_some() {
        assignments.push("monitored = ?");
    }

    if assignments.is_empty() {
        return Err(AppError::Validation(
            "at least one collection field must be provided".into(),
        ));
    }

    let mut sql = String::from("UPDATE collections SET ");
    sql.push_str(&assignments.join(", "));
    sql.push_str(" WHERE id = ?");

    let mut statement = sqlx::query(&sql);
    if let Some(collection_type) = collection_type {
        statement = statement.bind(collection_type.as_str().to_string());
    }
    if let Some(collection_index) = collection_index {
        statement = statement.bind(collection_index);
    }
    if let Some(label) = label {
        statement = statement.bind(label);
    }
    if let Some(ordered_path) = ordered_path {
        statement = statement.bind(ordered_path);
    }
    if let Some(first_episode_number) = first_episode_number {
        statement = statement.bind(first_episode_number);
    }
    if let Some(last_episode_number) = last_episode_number {
        statement = statement.bind(last_episode_number);
    }
    if let Some(monitored) = monitored {
        statement = statement.bind(if monitored { 1_i64 } else { 0_i64 });
    }
    statement = statement.bind(collection_id);

    let result = statement
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("collection {}", collection_id)));
    }

    get_collection_by_id_query(pool, collection_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))
}

pub(crate) async fn list_episodes_for_collection_query(
    pool: &SqlitePool,
    collection_id: &str,
) -> AppResult<Vec<Episode>> {
    let rows = sqlx::query(
        "SELECT id, title_id, collection_id, episode_type, episode_number, season_number,
                episode_label, title, air_date, duration_seconds, has_multi_audio,
                has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at
         FROM episodes WHERE collection_id = ? ORDER BY episode_number ASC, id ASC",
    )
    .bind(collection_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_episode(&row)?);
    }
    Ok(out)
}

pub(crate) async fn list_episodes_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<Episode>> {
    let rows = sqlx::query(
        "SELECT id, title_id, collection_id, episode_type, episode_number, season_number,
                episode_label, title, air_date, duration_seconds, has_multi_audio,
                has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at
         FROM episodes WHERE title_id = ? ORDER BY season_number ASC, episode_number ASC, id ASC",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_episode(&row)?);
    }
    Ok(out)
}

pub(crate) async fn get_episode_by_id_query(
    pool: &SqlitePool,
    episode_id: &str,
) -> AppResult<Option<Episode>> {
    let row = sqlx::query(
        "SELECT id, title_id, collection_id, episode_type, episode_number, season_number,
                episode_label, title, air_date, duration_seconds, has_multi_audio,
                has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at
         FROM episodes WHERE id = ?",
    )
    .bind(episode_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_episode(&row)?)),
        None => Ok(None),
    }
}

#[expect(clippy::too_many_arguments)]
pub(crate) async fn update_episode_query(
    pool: &SqlitePool,
    episode_id: &str,
    episode_type: Option<scryer_domain::EpisodeType>,
    episode_number: Option<String>,
    season_number: Option<String>,
    episode_label: Option<String>,
    title: Option<String>,
    air_date: Option<String>,
    duration_seconds: Option<i64>,
    has_multi_audio: Option<bool>,
    has_subtitle: Option<bool>,
    monitored: Option<bool>,
    collection_id: Option<String>,
    overview: Option<String>,
    tvdb_id: Option<String>,
) -> AppResult<Episode> {
    let mut assignments = Vec::new();
    if episode_type.is_some() {
        assignments.push("episode_type = ?");
    }
    if episode_number.is_some() {
        assignments.push("episode_number = ?");
    }
    if season_number.is_some() {
        assignments.push("season_number = ?");
    }
    if episode_label.is_some() {
        assignments.push("episode_label = ?");
    }
    if title.is_some() {
        assignments.push("title = ?");
    }
    if air_date.is_some() {
        assignments.push("air_date = ?");
    }
    if duration_seconds.is_some() {
        assignments.push("duration_seconds = ?");
    }
    if has_multi_audio.is_some() {
        assignments.push("has_multi_audio = ?");
    }
    if has_subtitle.is_some() {
        assignments.push("has_subtitle = ?");
    }
    if monitored.is_some() {
        assignments.push("monitored = ?");
    }
    if collection_id.is_some() {
        assignments.push("collection_id = ?");
    }
    if overview.is_some() {
        assignments.push("overview = ?");
    }
    if tvdb_id.is_some() {
        assignments.push("tvdb_id = ?");
    }

    if assignments.is_empty() {
        return Err(AppError::Validation(
            "at least one episode field must be provided".into(),
        ));
    }

    let mut sql = String::from("UPDATE episodes SET ");
    sql.push_str(&assignments.join(", "));
    sql.push_str(" WHERE id = ?");

    let mut statement = sqlx::query(&sql);
    if let Some(episode_type) = episode_type {
        statement = statement.bind(episode_type.as_str());
    }
    if let Some(episode_number) = episode_number {
        statement = statement.bind(episode_number);
    }
    if let Some(season_number) = season_number {
        statement = statement.bind(season_number);
    }
    if let Some(episode_label) = episode_label {
        statement = statement.bind(episode_label);
    }
    if let Some(title) = title {
        statement = statement.bind(title);
    }
    if let Some(air_date) = air_date {
        statement = statement.bind(air_date);
    }
    if let Some(duration_seconds) = duration_seconds {
        statement = statement.bind(duration_seconds);
    }
    if let Some(has_multi_audio) = has_multi_audio {
        statement = statement.bind(if has_multi_audio { 1_i64 } else { 0_i64 });
    }
    if let Some(has_subtitle) = has_subtitle {
        statement = statement.bind(if has_subtitle { 1_i64 } else { 0_i64 });
    }
    if let Some(monitored) = monitored {
        statement = statement.bind(if monitored { 1_i64 } else { 0_i64 });
    }
    if let Some(collection_id) = collection_id {
        statement = statement.bind(collection_id);
    }
    if let Some(overview) = overview {
        statement = statement.bind(overview);
    }
    if let Some(tvdb_id) = tvdb_id {
        statement = statement.bind(tvdb_id);
    }
    statement = statement.bind(episode_id);

    let result = statement
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("episode {}", episode_id)));
    }

    get_episode_by_id_query(pool, episode_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))
}

pub(crate) async fn create_episode_query(
    pool: &SqlitePool,
    episode: &Episode,
) -> AppResult<Episode> {
    sqlx::query(
        "INSERT INTO episodes
         (id, title_id, collection_id, episode_type, episode_number, season_number,
          episode_label, title, air_date, duration_seconds, has_multi_audio,
          has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&episode.id)
    .bind(&episode.title_id)
    .bind(&episode.collection_id)
    .bind(episode.episode_type.as_str())
    .bind(&episode.episode_number)
    .bind(&episode.season_number)
    .bind(&episode.episode_label)
    .bind(&episode.title)
    .bind(&episode.air_date)
    .bind(episode.duration_seconds)
    .bind(if episode.has_multi_audio {
        1_i64
    } else {
        0_i64
    })
    .bind(if episode.has_subtitle { 1_i64 } else { 0_i64 })
    .bind(if episode.is_filler { 1_i64 } else { 0_i64 })
    .bind(if episode.is_recap { 1_i64 } else { 0_i64 })
    .bind(&episode.absolute_number)
    .bind(&episode.overview)
    .bind(&episode.tvdb_id)
    .bind(if episode.monitored { 1_i64 } else { 0_i64 })
    .bind(episode.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(episode.clone())
}

pub(crate) async fn delete_collection_query(
    pool: &SqlitePool,
    collection_id: &str,
) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM collections WHERE id = ?")
        .bind(collection_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("collection {}", collection_id)));
    }

    Ok(())
}

pub(crate) async fn delete_collections_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM collections WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn delete_episode_query(pool: &SqlitePool, episode_id: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM episodes WHERE id = ?")
        .bind(episode_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("episode {}", episode_id)));
    }

    Ok(())
}

pub(crate) async fn delete_episodes_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM episodes WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn find_episode_by_title_and_numbers_query(
    pool: &SqlitePool,
    title_id: &str,
    season_number: &str,
    episode_number: &str,
) -> AppResult<Option<Episode>> {
    let row = sqlx::query(
        "SELECT e.id, e.title_id, e.collection_id, e.episode_type, e.episode_number,
                e.season_number, e.episode_label, e.title, e.air_date, e.duration_seconds,
                e.has_multi_audio, e.has_subtitle, e.is_filler, e.is_recap, e.absolute_number,
                e.overview, e.tvdb_id, e.monitored, e.created_at
         FROM episodes e
         INNER JOIN collections c ON c.id = e.collection_id
         WHERE e.title_id = ?
           AND c.collection_index = ?
           AND e.episode_number = ?
         LIMIT 1",
    )
    .bind(title_id)
    .bind(season_number)
    .bind(episode_number)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_episode(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn find_episode_by_title_and_absolute_number_query(
    pool: &SqlitePool,
    title_id: &str,
    absolute_number: &str,
) -> AppResult<Option<Episode>> {
    let row = sqlx::query(
        "SELECT e.id, e.title_id, e.collection_id, e.episode_type, e.episode_number,
                e.season_number, e.episode_label, e.title, e.air_date, e.duration_seconds,
                e.has_multi_audio, e.has_subtitle, e.is_filler, e.is_recap, e.absolute_number,
                e.overview, e.tvdb_id, e.monitored, e.created_at
         FROM episodes e
         WHERE e.title_id = ?
           AND e.absolute_number = ?
         LIMIT 1",
    )
    .bind(title_id)
    .bind(absolute_number)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_episode(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn list_episodes_in_date_range_query(
    pool: &SqlitePool,
    start_date: &str,
    end_date: &str,
) -> AppResult<Vec<CalendarEpisode>> {
    let rows = sqlx::query(
        "SELECT e.id, e.title_id, t.name AS title_name, t.facet AS title_facet,
                e.season_number, e.episode_number, e.title AS episode_title,
                e.air_date, e.monitored
         FROM episodes e
         JOIN titles t ON e.title_id = t.id
         WHERE e.air_date IS NOT NULL AND e.air_date != ''
           AND e.air_date >= ? AND e.air_date <= ?
         ORDER BY e.air_date ASC",
    )
    .bind(start_date)
    .bind(end_date)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(CalendarEpisode {
            id: row.get("id"),
            title_id: row.get("title_id"),
            title_name: row.get("title_name"),
            title_facet: row.get("title_facet"),
            season_number: row.get("season_number"),
            episode_number: row.get("episode_number"),
            episode_title: row.get("episode_title"),
            air_date: row.get("air_date"),
            monitored: row.get::<i64, _>("monitored") != 0,
        });
    }
    Ok(out)
}

pub(crate) async fn update_interstitial_season_episode_query(
    pool: &SqlitePool,
    collection_id: &str,
    season_episode: Option<&str>,
) -> AppResult<()> {
    sqlx::query("UPDATE collections SET interstitial_season_episode = ? WHERE id = ?")
        .bind(season_episode)
        .bind(collection_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn set_collection_episodes_monitored_query(
    pool: &SqlitePool,
    collection_id: &str,
    monitored: bool,
) -> AppResult<()> {
    sqlx::query("UPDATE episodes SET monitored = ? WHERE collection_id = ?")
        .bind(if monitored { 1_i64 } else { 0_i64 })
        .bind(collection_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

fn row_to_collection(row: &sqlx::sqlite::SqliteRow) -> AppResult<Collection> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title_id: String = row
        .try_get("title_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let collection_type_raw: String = row
        .try_get::<String, _>("collection_type")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let collection_type = CollectionType::parse(&collection_type_raw).unwrap_or_default();
    let collection_index: String = row
        .try_get("collection_index")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let label: Option<String> = row
        .try_get("label")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let ordered_path: Option<String> = row
        .try_get("ordered_path")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let narrative_order: Option<String> = row.try_get("narrative_order").unwrap_or(None);
    let first_episode_number = optional_text_from_column(row, "first_episode_number")?;
    let last_episode_number = optional_text_from_column(row, "last_episode_number")?;
    let interstitial_movie = row_to_interstitial_movie(row)?;
    let specials_movies = row_to_specials_movies(row)?;
    let interstitial_season_episode: Option<String> =
        row.try_get("interstitial_season_episode").unwrap_or(None);
    let monitored: i64 = row
        .try_get("monitored")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(Collection {
        id,
        title_id,
        collection_type,
        collection_index,
        label,
        ordered_path,
        narrative_order,
        first_episode_number,
        last_episode_number,
        interstitial_movie,
        specials_movies,
        interstitial_season_episode,
        monitored: monitored != 0,
        created_at: parse_utc_datetime(&created_at_raw)?,
    })
}

fn row_to_interstitial_movie(
    row: &sqlx::sqlite::SqliteRow,
) -> AppResult<Option<InterstitialMovieMetadata>> {
    let Some(tvdb_id) = row
        .try_get::<Option<String>, _>("interstitial_tvdb_id")
        .unwrap_or(None)
    else {
        return Ok(None);
    };

    let genres_json = row
        .try_get::<Option<String>, _>("interstitial_genres_json")
        .unwrap_or(None);
    let genres = genres_json
        .as_deref()
        .map(serde_json::from_str::<Vec<String>>)
        .transpose()
        .map_err(|err| AppError::Repository(err.to_string()))?
        .unwrap_or_default();

    Ok(Some(InterstitialMovieMetadata {
        tvdb_id,
        name: row.try_get("interstitial_name").unwrap_or_default(),
        slug: row.try_get("interstitial_slug").unwrap_or_default(),
        year: row.try_get("interstitial_year").unwrap_or(None),
        content_status: row
            .try_get("interstitial_content_status")
            .unwrap_or_default(),
        overview: row.try_get("interstitial_overview").unwrap_or_default(),
        poster_url: row.try_get("interstitial_poster_url").unwrap_or_default(),
        language: row.try_get("interstitial_language").unwrap_or_default(),
        runtime_minutes: row
            .try_get("interstitial_runtime_minutes")
            .unwrap_or_default(),
        sort_title: row.try_get("interstitial_sort_title").unwrap_or_default(),
        imdb_id: row.try_get("interstitial_imdb_id").unwrap_or_default(),
        genres,
        studio: row.try_get("interstitial_studio").unwrap_or_default(),
        digital_release_date: row
            .try_get("interstitial_digital_release_date")
            .unwrap_or(None),
        association_confidence: row
            .try_get("interstitial_association_confidence")
            .unwrap_or(None),
        continuity_status: row
            .try_get("interstitial_continuity_status")
            .unwrap_or(None),
        movie_form: row.try_get("interstitial_movie_form").unwrap_or(None),
        confidence: row.try_get("interstitial_confidence").unwrap_or(None),
        signal_summary: row.try_get("interstitial_signal_summary").unwrap_or(None),
        placement: row.try_get("interstitial_placement").unwrap_or(None),
        movie_tmdb_id: row.try_get("interstitial_movie_tmdb_id").unwrap_or(None),
        movie_mal_id: row.try_get("interstitial_movie_mal_id").unwrap_or(None),
        movie_anidb_id: row.try_get("interstitial_movie_anidb_id").unwrap_or(None),
    }))
}

fn row_to_specials_movies(
    row: &sqlx::sqlite::SqliteRow,
) -> AppResult<Vec<InterstitialMovieMetadata>> {
    let raw = row
        .try_get::<Option<String>, _>("special_movies_json")
        .unwrap_or(None)
        .unwrap_or_else(|| "[]".to_string());
    serde_json::from_str(&raw).map_err(|err| AppError::Repository(err.to_string()))
}

fn optional_text_from_column(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> AppResult<Option<String>> {
    match row.try_get::<Option<String>, _>(column) {
        Ok(value) => Ok(value),
        Err(string_err) => match row.try_get::<Option<i64>, _>(column) {
            Ok(value) => Ok(value.map(|value| value.to_string())),
            Err(integer_err) => Err(AppError::Repository(format!(
                "failed decode {column} as optional text: {string_err}; {integer_err}"
            ))),
        },
    }
}

fn row_to_episode(row: &sqlx::sqlite::SqliteRow) -> AppResult<Episode> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title_id: String = row
        .try_get("title_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let collection_id: Option<String> = row
        .try_get("collection_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let episode_type_raw: String = row
        .try_get::<String, _>("episode_type")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let episode_type = scryer_domain::EpisodeType::parse(&episode_type_raw).unwrap_or_default();
    let episode_number: Option<String> = row
        .try_get("episode_number")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let season_number: Option<String> = row
        .try_get("season_number")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let episode_label: Option<String> = row
        .try_get("episode_label")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title: Option<String> = row
        .try_get("title")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let air_date: Option<String> = row
        .try_get("air_date")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let duration_seconds: Option<i64> = row
        .try_get("duration_seconds")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let has_multi_audio: i64 = row
        .try_get("has_multi_audio")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let has_subtitle: i64 = row
        .try_get("has_subtitle")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let is_filler: i64 = row.try_get("is_filler").unwrap_or(0);
    let is_recap: i64 = row.try_get("is_recap").unwrap_or(0);
    let absolute_number: Option<String> = row.try_get("absolute_number").unwrap_or(None);
    let overview: Option<String> = row.try_get("overview").unwrap_or(None);
    let tvdb_id: Option<String> = row.try_get("tvdb_id").unwrap_or(None);
    let monitored: i64 = row
        .try_get("monitored")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(Episode {
        id,
        title_id,
        collection_id,
        episode_type,
        episode_number,
        season_number,
        episode_label,
        title,
        air_date,
        duration_seconds,
        has_multi_audio: has_multi_audio != 0,
        has_subtitle: has_subtitle != 0,
        is_filler: is_filler != 0,
        is_recap: is_recap != 0,
        absolute_number,
        overview,
        tvdb_id,
        monitored: monitored != 0,
        created_at: parse_utc_datetime(&created_at_raw)?,
    })
}

pub(crate) async fn create_title_query(pool: &SqlitePool, title: &Title) -> AppResult<Title> {
    let tags_json =
        serde_json::to_string(&title.tags).map_err(|err| AppError::Repository(err.to_string()))?;
    let ext_json = serde_json::to_string(&title.external_ids)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let genres_json = serde_json::to_string(&title.genres)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let aliases_json = serde_json::to_string(&title.aliases)
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query(
        "INSERT INTO titles (
            id, name, facet, monitored, tags, external_ids, created_by, created_at,
            year, overview, poster_url, sort_title, slug, runtime_minutes,
            genres, content_status, language, min_availability, aliases, folder_path, tagged_aliases_json
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&title.id)
    .bind(&title.name)
    .bind(title.facet.as_str())
    .bind(if title.monitored { 1_i64 } else { 0_i64 })
    .bind(&tags_json)
    .bind(&ext_json)
    .bind(&title.created_by)
    .bind(title.created_at.to_rfc3339())
    .bind(title.year)
    .bind(&title.overview)
    .bind(&title.poster_url)
    .bind(&title.sort_title)
    .bind(&title.slug)
    .bind(title.runtime_minutes)
    .bind(&genres_json)
    .bind(&title.content_status)
    .bind(&title.language)
    .bind(&title.min_availability)
    .bind(&aliases_json)
    .bind(&title.folder_path)
    .bind(serde_json::to_string(&title.tagged_aliases).unwrap_or_else(|_| "[]".to_string()))
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(title.clone())
}

pub(crate) async fn update_title_monitored_query(
    pool: &SqlitePool,
    id: &str,
    monitored: bool,
) -> AppResult<Title> {
    let result = sqlx::query("UPDATE titles SET monitored = ? WHERE id = ?")
        .bind(if monitored { 1_i64 } else { 0_i64 })
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    get_title_by_id_query(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {}", id)))
}

pub(crate) async fn set_title_folder_path_query(
    pool: &SqlitePool,
    id: &str,
    folder_path: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE titles SET folder_path = ? WHERE id = ?")
        .bind(folder_path)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn update_title_metadata_query(
    pool: &SqlitePool,
    id: &str,
    name: Option<String>,
    facet: Option<MediaFacet>,
    tags_json: Option<String>,
) -> AppResult<Title> {
    let mut assignments = Vec::new();
    if name.is_some() {
        assignments.push("name = ?");
    }
    if facet.is_some() {
        assignments.push("facet = ?");
    }
    if tags_json.is_some() {
        assignments.push("tags = ?");
    }

    if assignments.is_empty() {
        return Err(AppError::Validation(
            "at least one title field must be provided".into(),
        ));
    }

    let mut sql = String::from("UPDATE titles SET ");
    sql.push_str(&assignments.join(", "));
    sql.push_str(" WHERE id = ?");

    let mut statement = sqlx::query(&sql);
    if let Some(name) = name {
        let normalized = name.trim();
        if normalized.is_empty() {
            return Err(AppError::Validation("title name cannot be empty".into()));
        }
        statement = statement.bind(normalized.to_string());
    }
    if let Some(facet) = facet {
        statement = statement.bind(facet.as_str());
    }
    if let Some(tags_json) = tags_json {
        statement = statement.bind(tags_json);
    }
    statement = statement.bind(id);

    let result = statement
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    get_title_by_id_query(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {}", id)))
}

pub(crate) async fn replace_title_match_state_query(
    pool: &SqlitePool,
    id: &str,
    external_ids: Vec<ExternalId>,
    tags: Vec<String>,
) -> AppResult<Title> {
    let external_ids_json = serde_json::to_string(&external_ids)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let tags_json =
        serde_json::to_string(&tags).map_err(|err| AppError::Repository(err.to_string()))?;

    let result = sqlx::query(
        "UPDATE titles SET
            external_ids = ?,
            tags = ?,
            year = NULL,
            overview = NULL,
            poster_url = NULL,
            banner_url = NULL,
            background_url = NULL,
            sort_title = NULL,
            slug = NULL,
            imdb_id = NULL,
            runtime_minutes = NULL,
            genres = '[]',
            content_status = NULL,
            language = NULL,
            first_aired = NULL,
            network = NULL,
            studio = NULL,
            country = NULL,
            aliases = '[]',
            tagged_aliases_json = '[]',
            metadata_language = NULL,
            metadata_fetched_at = NULL,
            digital_release_date = NULL
         WHERE id = ?",
    )
    .bind(&external_ids_json)
    .bind(&tags_json)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    get_title_by_id_query(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {}", id)))
}

pub(crate) async fn update_title_hydrated_metadata_query(
    pool: &SqlitePool,
    id: &str,
    metadata: TitleMetadataUpdate,
) -> AppResult<Title> {
    let genres_json = serde_json::to_string(&metadata.genres)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let aliases_json = serde_json::to_string(&metadata.aliases)
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let result = sqlx::query(
        "UPDATE titles SET
            year = COALESCE(?, year),
            overview = COALESCE(NULLIF(?, ''), overview),
            poster_url = COALESCE(NULLIF(?, ''), poster_url),
            banner_url = COALESCE(NULLIF(?, ''), banner_url),
            background_url = COALESCE(NULLIF(?, ''), background_url),
            sort_title = COALESCE(NULLIF(?, ''), sort_title),
            slug = COALESCE(NULLIF(?, ''), slug),
            imdb_id = COALESCE(NULLIF(?, ''), imdb_id),
            runtime_minutes = COALESCE(?, runtime_minutes),
            genres = CASE WHEN NULLIF(?, '[]') IS NOT NULL THEN ? ELSE genres END,
            content_status = COALESCE(NULLIF(?, ''), content_status),
            language = COALESCE(NULLIF(?, ''), language),
            first_aired = COALESCE(NULLIF(?, ''), first_aired),
            network = COALESCE(NULLIF(?, ''), network),
            studio = COALESCE(NULLIF(?, ''), studio),
            country = COALESCE(NULLIF(?, ''), country),
            aliases = CASE WHEN NULLIF(?, '[]') IS NOT NULL THEN ? ELSE aliases END,
            tagged_aliases_json = CASE WHEN NULLIF(?, '[]') IS NOT NULL THEN ? ELSE tagged_aliases_json END,
            metadata_language = COALESCE(NULLIF(?, ''), metadata_language),
            metadata_fetched_at = COALESCE(NULLIF(?, ''), metadata_fetched_at),
            digital_release_date = COALESCE(NULLIF(?, ''), digital_release_date)
         WHERE id = ?",
    )
    .bind(metadata.year)
    .bind(&metadata.overview)
    .bind(&metadata.poster_url)
    .bind(&metadata.banner_url)
    .bind(&metadata.background_url)
    .bind(&metadata.sort_title)
    .bind(&metadata.slug)
    .bind(&metadata.imdb_id)
    .bind(metadata.runtime_minutes)
    .bind(&genres_json)
    .bind(&genres_json)
    .bind(&metadata.content_status)
    .bind(&metadata.language)
    .bind(&metadata.first_aired)
    .bind(&metadata.network)
    .bind(&metadata.studio)
    .bind(&metadata.country)
    .bind(&aliases_json)
    .bind(&aliases_json)
    .bind(serde_json::to_string(&metadata.tagged_aliases).unwrap_or_else(|_| "[]".to_string()))
    .bind(serde_json::to_string(&metadata.tagged_aliases).unwrap_or_else(|_| "[]".to_string()))
    .bind(&metadata.metadata_language)
    .bind(&metadata.metadata_fetched_at)
    .bind(&metadata.digital_release_date)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    // Merge extra external IDs (e.g. anime mappings) into the title's external_ids JSON
    if !metadata.extra_external_ids.is_empty() {
        let existing_json: String =
            sqlx::query_scalar("SELECT external_ids FROM titles WHERE id = ?")
                .bind(id)
                .fetch_one(pool)
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;

        let mut existing: Vec<ExternalId> =
            serde_json::from_str(&existing_json).unwrap_or_default();

        for eid in &metadata.extra_external_ids {
            // Replace any existing entry with the same source so that
            // re-hydration converges to a single ID per source (e.g. one
            // "mal" entry instead of one per anime season).
            existing.retain(|e| e.source != eid.source);
            existing.push(eid.clone());
        }

        let merged_json = serde_json::to_string(&existing)
            .map_err(|err| AppError::Repository(err.to_string()))?;

        sqlx::query("UPDATE titles SET external_ids = ? WHERE id = ?")
            .bind(&merged_json)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    // Merge extra tags (e.g. anime metadata) into the title's tags JSON
    if !metadata.extra_tags.is_empty() {
        let existing_json: String = sqlx::query_scalar("SELECT tags FROM titles WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        let mut existing: Vec<String> = serde_json::from_str(&existing_json).unwrap_or_default();

        for tag in &metadata.extra_tags {
            if let Some(colon_pos) = tag.rfind(':') {
                let prefix = &tag[..=colon_pos];
                existing.retain(|t| !t.starts_with(prefix));
            }
            existing.push(tag.clone());
        }

        let merged_json = serde_json::to_string(&existing)
            .map_err(|err| AppError::Repository(err.to_string()))?;

        sqlx::query("UPDATE titles SET tags = ? WHERE id = ?")
            .bind(&merged_json)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    get_title_by_id_query(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {}", id)))
}

pub(crate) async fn delete_title_query(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM titles WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SummaryCandidate, summary_candidate_sort_key};
    use scryer_domain::CollectionType;

    #[test]
    fn movie_collection_wins_over_index_zero_fallback() {
        let mut candidates = [
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Season,
                collection_index: "0".to_string(),
                label: Some("Specials".to_string()),
                ordered_path: None,
            },
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Movie,
                collection_index: "1".to_string(),
                label: Some("1080P".to_string()),
                ordered_path: Some("/media/movies/Movie/Movie.1080P.mkv".to_string()),
            },
        ];
        candidates.sort_by_key(summary_candidate_sort_key);

        assert_eq!(candidates[0].collection_type, CollectionType::Movie);
        assert_eq!(candidates[0].label.as_deref(), Some("1080P"));
    }

    #[test]
    fn movie_collection_with_path_wins_over_pathless_movie_collection() {
        let mut candidates = [
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Movie,
                collection_index: "2".to_string(),
                label: Some("2160P".to_string()),
                ordered_path: None,
            },
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Movie,
                collection_index: "1".to_string(),
                label: Some("1080P".to_string()),
                ordered_path: Some("/media/movies/Movie/Movie.1080P.mkv".to_string()),
            },
        ];
        candidates.sort_by_key(summary_candidate_sort_key);

        assert_eq!(candidates[0].collection_index, "1");
        assert_eq!(candidates[0].label.as_deref(), Some("1080P"));
    }
}
