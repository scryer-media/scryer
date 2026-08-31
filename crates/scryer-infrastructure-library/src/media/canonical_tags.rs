use std::collections::{BTreeMap, HashSet};

use chrono::Utc;
use scryer_application::{AppResult, TitleExternalRating, TitleRatingSummary};
use scryer_domain::{CanonicalMediaTag, Title};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, SqlTx};

#[derive(Clone, Copy)]
struct MetadataTables {
    owner_column: &'static str,
    tags: &'static str,
    tag_sources: &'static str,
    tag_source_keys: &'static str,
    rating_summaries: &'static str,
    rating_sources: &'static str,
    external_ratings: &'static str,
}

#[derive(Clone, Copy)]
struct MetadataRatingTables {
    owner_column: &'static str,
    rating_summaries: &'static str,
    rating_sources: &'static str,
    external_ratings: &'static str,
}

#[derive(Clone, Copy)]
enum MetadataRatingOwner {
    Title,
    MovieEntity,
    DiscoveryTitle,
}

impl MetadataRatingOwner {
    const fn tables(self) -> MetadataRatingTables {
        match self {
            Self::Title => TITLE_METADATA_TABLES.ratings(),
            Self::MovieEntity => MOVIE_ENTITY_METADATA_RATING_TABLES,
            Self::DiscoveryTitle => DISCOVERY_TITLE_METADATA_TABLES.ratings(),
        }
    }
}

impl MetadataTables {
    const fn ratings(self) -> MetadataRatingTables {
        MetadataRatingTables {
            owner_column: self.owner_column,
            rating_summaries: self.rating_summaries,
            rating_sources: self.rating_sources,
            external_ratings: self.external_ratings,
        }
    }
}

const TITLE_METADATA_TABLES: MetadataTables = MetadataTables {
    owner_column: "title_id",
    tags: "title_metadata_tags",
    tag_sources: "title_metadata_tag_sources",
    tag_source_keys: "title_metadata_tag_source_keys",
    rating_summaries: "title_metadata_rating_summaries",
    rating_sources: "title_metadata_rating_sources",
    external_ratings: "title_metadata_external_ratings",
};

const DISCOVERY_TITLE_METADATA_TABLES: MetadataTables = MetadataTables {
    owner_column: "discovery_title_id",
    tags: "discovery_title_metadata_tags",
    tag_sources: "discovery_title_metadata_tag_sources",
    tag_source_keys: "discovery_title_metadata_tag_source_keys",
    rating_summaries: "discovery_title_metadata_rating_summaries",
    rating_sources: "discovery_title_metadata_rating_sources",
    external_ratings: "discovery_title_metadata_external_ratings",
};

const MOVIE_ENTITY_METADATA_RATING_TABLES: MetadataRatingTables = MetadataRatingTables {
    owner_column: "movie_entity_id",
    rating_summaries: "title_metadata_rating_summaries",
    rating_sources: "title_metadata_rating_sources",
    external_ratings: "title_metadata_external_ratings",
};

pub async fn replace_title_metadata_tags_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    tags: &[CanonicalMediaTag],
) -> AppResult<()> {
    replace_metadata_tags_tx(tx, TITLE_METADATA_TABLES, title_id, tags).await
}

pub async fn replace_discovery_title_metadata_tags_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    tags: &[CanonicalMediaTag],
) -> AppResult<()> {
    replace_metadata_tags_tx(
        tx,
        DISCOVERY_TITLE_METADATA_TABLES,
        discovery_title_id,
        tags,
    )
    .await
}

async fn replace_metadata_tags_tx(
    tx: &mut SqlTx<'_>,
    tables: MetadataTables,
    owner_id: &str,
    tags: &[CanonicalMediaTag],
) -> AppResult<()> {
    tx.execute(
        &format!(
            "DELETE FROM {} WHERE {} = {{}}",
            tables.tags, tables.owner_column
        ),
        &[SqlArg::Text(owner_id.to_string())],
    )
    .await?;

    let mut seen = HashSet::new();
    for (sort_index, tag) in tags.iter().enumerate() {
        let key = tag.key.trim();
        let category = tag.category.trim();
        let name = tag.name.trim();
        if key.is_empty() || category.is_empty() || name.is_empty() || !seen.insert(key.to_string())
        {
            continue;
        }

        tx.execute(
            &format!(
                "INSERT INTO {} (
                    {}, tag_key, category, name, confidence, is_adult, is_spoiler, sort_index
                ) VALUES ({{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}})",
                tables.tags, tables.owner_column
            ),
            &[
                SqlArg::Text(owner_id.to_string()),
                SqlArg::Text(key.to_string()),
                SqlArg::Text(category.to_string()),
                SqlArg::Text(name.to_string()),
                SqlArg::OptF64(tag.confidence.filter(|value| value.is_finite())),
                SqlArg::Bool(tag.is_adult),
                SqlArg::Bool(tag.is_spoiler),
                SqlArg::I32(sort_index as i32),
            ],
        )
        .await?;

        insert_tag_values_tx(
            tx,
            tables,
            tables.tag_sources,
            "source",
            owner_id,
            key,
            &tag.sources,
        )
        .await?;
        insert_tag_values_tx(
            tx,
            tables,
            tables.tag_source_keys,
            "source_tag_key",
            owner_id,
            key,
            &tag.source_tag_keys,
        )
        .await?;
    }

    Ok(())
}

pub async fn attach_metadata_tags_to_titles(
    exec: SqlExec<'_, '_>,
    titles: &mut [Title],
) -> AppResult<()> {
    if titles.is_empty() {
        return Ok(());
    }

    let title_ids = titles
        .iter()
        .map(|title| title.id.clone())
        .collect::<Vec<_>>();
    let tags_by_title = load_title_metadata_tags(exec, &title_ids).await?;
    for title in titles {
        if let Some(tags) = tags_by_title.get(&title.id) {
            title.canonical_tags = tags.clone();
        }
    }
    Ok(())
}

pub async fn load_title_metadata_tags(
    exec: SqlExec<'_, '_>,
    title_ids: &[String],
) -> AppResult<BTreeMap<String, Vec<CanonicalMediaTag>>> {
    load_metadata_tags(exec, TITLE_METADATA_TABLES, title_ids).await
}

pub async fn load_discovery_title_metadata_tags(
    exec: SqlExec<'_, '_>,
    discovery_title_ids: &[String],
) -> AppResult<BTreeMap<String, Vec<CanonicalMediaTag>>> {
    load_metadata_tags(exec, DISCOVERY_TITLE_METADATA_TABLES, discovery_title_ids).await
}

async fn load_metadata_tags(
    exec: SqlExec<'_, '_>,
    tables: MetadataTables,
    owner_ids: &[String],
) -> AppResult<BTreeMap<String, Vec<CanonicalMediaTag>>> {
    if owner_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let placeholders = bind_placeholders(owner_ids.len());
    let sql = format!(
        "SELECT
            t.{owner_column} AS owner_id,
            t.tag_key AS tag_key,
            t.category AS category,
            t.name AS name,
            t.confidence AS confidence,
            t.is_adult AS is_adult,
            t.is_spoiler AS is_spoiler,
            ts.source AS source,
            tsk.source_tag_key AS source_tag_key
         FROM {tags} t
         LEFT JOIN {tag_sources} ts
            ON ts.{owner_column} = t.{owner_column} AND ts.tag_key = t.tag_key
         LEFT JOIN {tag_source_keys} tsk
            ON tsk.{owner_column} = t.{owner_column} AND tsk.tag_key = t.tag_key
         WHERE t.{owner_column} IN ({placeholders})
         ORDER BY t.{owner_column}, t.sort_index, t.category, t.name,
                  ts.sort_index, ts.source, tsk.sort_index, tsk.source_tag_key",
        owner_column = tables.owner_column,
        tags = tables.tags,
        tag_sources = tables.tag_sources,
        tag_source_keys = tables.tag_source_keys,
    );
    let args = owner_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;
    rows_to_tags_by_owner(&rows)
}

pub async fn replace_title_metadata_ratings_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    ratings: &TitleRatingSummary,
) -> AppResult<()> {
    replace_metadata_ratings_tx(tx, MetadataRatingOwner::Title, title_id, ratings).await
}

pub async fn replace_movie_entity_metadata_ratings_tx(
    tx: &mut SqlTx<'_>,
    movie_entity_id: &str,
    ratings: &TitleRatingSummary,
) -> AppResult<()> {
    replace_metadata_ratings_tx(
        tx,
        MetadataRatingOwner::MovieEntity,
        movie_entity_id,
        ratings,
    )
    .await
}

pub async fn replace_discovery_title_metadata_ratings_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    ratings: &TitleRatingSummary,
) -> AppResult<()> {
    replace_metadata_ratings_tx(
        tx,
        MetadataRatingOwner::DiscoveryTitle,
        discovery_title_id,
        ratings,
    )
    .await
}

async fn replace_metadata_ratings_tx(
    tx: &mut SqlTx<'_>,
    owner: MetadataRatingOwner,
    owner_id: &str,
    ratings: &TitleRatingSummary,
) -> AppResult<()> {
    let tables = owner.tables();
    for table in [
        tables.external_ratings,
        tables.rating_sources,
        tables.rating_summaries,
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE {} = {{}}", tables.owner_column),
            &[SqlArg::Text(owner_id.to_string())],
        )
        .await?;
    }

    let now = Utc::now();
    if ratings.rating.is_some() {
        tx.execute(
            &format!(
                "INSERT INTO {} ({}, rating, created_at, updated_at)
                 VALUES ({{}}, {{}}, {{}}, {{}})",
                tables.rating_summaries, tables.owner_column
            ),
            &[
                SqlArg::Text(owner_id.to_string()),
                SqlArg::OptF64(ratings.rating),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    let mut seen_sources = HashSet::new();
    for (sort_index, source) in ratings.rating_sources.iter().enumerate() {
        let source = source.trim();
        if source.is_empty() || !seen_sources.insert(source.to_ascii_lowercase()) {
            continue;
        }
        tx.execute(
            &format!(
                "INSERT INTO {} ({}, source, sort_index, created_at, updated_at)
                 VALUES ({{}}, {{}}, {{}}, {{}}, {{}})",
                tables.rating_sources, tables.owner_column
            ),
            &[
                SqlArg::Text(owner_id.to_string()),
                SqlArg::Text(source.to_string()),
                SqlArg::I32(sort_index as i32),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    let mut seen_external = HashSet::new();
    for (sort_index, rating) in ratings.external_ratings.iter().enumerate() {
        let source = rating.source.trim();
        if source.is_empty() || !seen_external.insert(source.to_ascii_lowercase()) {
            continue;
        }
        tx.execute(
            &format!(
                "INSERT INTO {} (
                    {}, source, sort_index, value, score, normalized, votes, url,
                    created_at, updated_at
                ) VALUES ({{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}})",
                tables.external_ratings, tables.owner_column
            ),
            &[
                SqlArg::Text(owner_id.to_string()),
                SqlArg::Text(source.to_string()),
                SqlArg::I32(sort_index as i32),
                SqlArg::OptF64(rating.value.filter(|value| value.is_finite())),
                SqlArg::OptF64(rating.score.filter(|value| value.is_finite())),
                SqlArg::OptF64(Some(rating.normalized).filter(|value| value.is_finite())),
                SqlArg::OptI32(rating.votes),
                SqlArg::Text(rating.url.trim().to_string()),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    Ok(())
}

pub async fn load_title_metadata_ratings(
    exec: SqlExec<'_, '_>,
    title_ids: &[String],
) -> AppResult<BTreeMap<String, TitleRatingSummary>> {
    load_metadata_ratings(exec, MetadataRatingOwner::Title, title_ids).await
}

pub async fn load_movie_entity_metadata_ratings(
    exec: SqlExec<'_, '_>,
    movie_entity_ids: &[String],
) -> AppResult<BTreeMap<String, TitleRatingSummary>> {
    load_metadata_ratings(exec, MetadataRatingOwner::MovieEntity, movie_entity_ids).await
}

pub async fn load_discovery_title_metadata_ratings(
    exec: SqlExec<'_, '_>,
    discovery_title_ids: &[String],
) -> AppResult<BTreeMap<String, TitleRatingSummary>> {
    load_metadata_ratings(
        exec,
        MetadataRatingOwner::DiscoveryTitle,
        discovery_title_ids,
    )
    .await
}

async fn load_metadata_ratings(
    exec: SqlExec<'_, '_>,
    owner: MetadataRatingOwner,
    owner_ids: &[String],
) -> AppResult<BTreeMap<String, TitleRatingSummary>> {
    if owner_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let tables = owner.tables();
    let placeholders = bind_placeholders(owner_ids.len());
    let sql = format!(
        "WITH metadata_ratings(
            owner_id,
            row_kind,
            rating,
            rating_source,
            external_source,
            external_value,
            external_score,
            external_normalized,
            external_votes,
            external_url,
            sort_index,
            source_name
         ) AS (
            SELECT
                summary.{owner_column},
                0,
                summary.rating,
                CAST(NULL AS TEXT),
                CAST(NULL AS TEXT),
                CAST(NULL AS DOUBLE PRECISION),
                CAST(NULL AS DOUBLE PRECISION),
                CAST(NULL AS DOUBLE PRECISION),
                CAST(NULL AS INTEGER),
                CAST(NULL AS TEXT),
                CAST(NULL AS INTEGER),
                CAST(NULL AS TEXT)
              FROM {rating_summaries} summary
             WHERE summary.{owner_column} IN ({placeholders})
            UNION ALL
            SELECT
                source.{owner_column},
                1,
                CAST(NULL AS DOUBLE PRECISION),
                source.source,
                CAST(NULL AS TEXT),
                CAST(NULL AS DOUBLE PRECISION),
                CAST(NULL AS DOUBLE PRECISION),
                CAST(NULL AS DOUBLE PRECISION),
                CAST(NULL AS INTEGER),
                CAST(NULL AS TEXT),
                source.sort_index,
                source.source
              FROM {rating_sources} source
             WHERE source.{owner_column} IN ({placeholders})
            UNION ALL
            SELECT
                external.{owner_column},
                2,
                CAST(NULL AS DOUBLE PRECISION),
                CAST(NULL AS TEXT),
                external.source,
                external.value,
                external.score,
                external.normalized,
                external.votes,
                external.url,
                external.sort_index,
                external.source
              FROM {external_ratings} external
             WHERE external.{owner_column} IN ({placeholders})
         )
         SELECT
            owner_id,
            rating,
            rating_source,
            external_source,
            external_value,
            external_score,
            external_normalized,
            external_votes,
            external_url
           FROM metadata_ratings
          ORDER BY owner_id, row_kind, sort_index, source_name",
        owner_column = tables.owner_column,
        rating_summaries = tables.rating_summaries,
        rating_sources = tables.rating_sources,
        external_ratings = tables.external_ratings,
    );
    let args = (0..3)
        .flat_map(|_| owner_ids.iter().cloned().map(SqlArg::Text))
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;
    rows_to_ratings_by_owner(&rows)
}

async fn insert_tag_values_tx(
    tx: &mut SqlTx<'_>,
    tables: MetadataTables,
    table: &str,
    column: &str,
    owner_id: &str,
    tag_key: &str,
    values: &[String],
) -> AppResult<()> {
    let mut seen = HashSet::new();
    for (sort_index, value) in values.iter().enumerate() {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        tx.execute(
            &format!(
                "INSERT INTO {table} ({}, tag_key, {column}, sort_index)
                 VALUES ({{}}, {{}}, {{}}, {{}})",
                tables.owner_column
            ),
            &[
                SqlArg::Text(owner_id.to_string()),
                SqlArg::Text(tag_key.to_string()),
                SqlArg::Text(value.to_string()),
                SqlArg::I32(sort_index as i32),
            ],
        )
        .await?;
    }
    Ok(())
}

fn rows_to_tags_by_owner(
    rows: &[crate::queries::sql_runtime::SqlRow],
) -> AppResult<BTreeMap<String, Vec<CanonicalMediaTag>>> {
    #[derive(Default)]
    struct OrderedOwnerTags {
        tag_order: Vec<String>,
        tags: BTreeMap<String, CanonicalMediaTag>,
    }

    let mut tags = BTreeMap::<String, OrderedOwnerTags>::new();
    for row in rows {
        let owner_id = row.text("owner_id")?;
        let tag_key = row.text("tag_key")?;
        let owner_tags = tags.entry(owner_id).or_default();
        if !owner_tags.tags.contains_key(&tag_key) {
            owner_tags.tag_order.push(tag_key.clone());
            owner_tags.tags.insert(
                tag_key.clone(),
                CanonicalMediaTag {
                    key: tag_key.clone(),
                    category: row.text("category")?,
                    name: row.text("name")?,
                    confidence: row.opt_f64("confidence")?,
                    sources: Vec::new(),
                    source_tag_keys: Vec::new(),
                    is_adult: row.bool("is_adult")?,
                    is_spoiler: row.bool("is_spoiler")?,
                },
            );
        }
        let tag = owner_tags
            .tags
            .get_mut(&tag_key)
            .expect("metadata tag was inserted before lookup");
        push_unique_opt(&mut tag.sources, row.opt_text("source")?);
        push_unique_opt(&mut tag.source_tag_keys, row.opt_text("source_tag_key")?);
    }
    Ok(tags
        .into_iter()
        .map(|(owner_id, tags)| {
            let values = tags
                .tag_order
                .into_iter()
                .filter_map(|tag_key| tags.tags.get(&tag_key).cloned())
                .collect();
            (owner_id, values)
        })
        .collect())
}

fn rows_to_ratings_by_owner(
    rows: &[crate::queries::sql_runtime::SqlRow],
) -> AppResult<BTreeMap<String, TitleRatingSummary>> {
    let mut ratings_by_owner = BTreeMap::<String, TitleRatingSummary>::new();
    for row in rows {
        let owner_id = row.text("owner_id")?;
        let ratings = ratings_by_owner.entry(owner_id).or_default();
        if ratings.rating.is_none() {
            ratings.rating = row.opt_f64("rating")?;
        }
        push_unique_opt(&mut ratings.rating_sources, row.opt_text("rating_source")?);
        let Some(source) = row.opt_text("external_source")? else {
            continue;
        };
        if source.trim().is_empty()
            || ratings
                .external_ratings
                .iter()
                .any(|rating| rating.source == source)
        {
            continue;
        }
        ratings.external_ratings.push(TitleExternalRating {
            source,
            value: row.opt_f64("external_value")?,
            score: row.opt_f64("external_score")?,
            normalized: row.opt_f64("external_normalized")?.unwrap_or_default(),
            votes: row
                .opt_i64("external_votes")?
                .and_then(|value| i32::try_from(value).ok()),
            url: row.opt_text("external_url")?.unwrap_or_default(),
        });
    }
    Ok(ratings_by_owner)
}

fn push_unique_opt(values: &mut Vec<String>, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let value = value.trim();
    if !value.is_empty() && !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn bind_placeholders(count: usize) -> String {
    (0..count).map(|_| "{}").collect::<Vec<_>>().join(", ")
}
