use std::collections::HashSet;

use scryer_application::{AppError, AppResult};
use scryer_domain::{MediaFacet, TaggedAlias, Title};
use sqlx::{Postgres, QueryBuilder, Row, Sqlite, SqlitePool, Transaction};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

const TERM_KIND_NAME: &str = "name";
const TERM_KIND_ALIAS: &str = "alias";
const TERM_KIND_TAGGED_ALIAS: &str = "tagged_alias";
const TERM_KIND_SORT_TITLE: &str = "sort_title";
const TERM_KIND_SLUG: &str = "slug";
const TERM_KIND_NAME_TOKEN: &str = "name_token";
const TERM_KIND_ALIAS_TOKEN: &str = "alias_token";
const TERM_KIND_TAGGED_ALIAS_TOKEN: &str = "tagged_alias_token";
const TERM_KIND_SORT_TITLE_TOKEN: &str = "sort_title_token";
const TERM_KIND_SLUG_TOKEN: &str = "slug_token";

const TERM_WEIGHT_NAME: i64 = 0;
const TERM_WEIGHT_ALIAS: i64 = 100;
const TERM_WEIGHT_TAGGED_ALIAS: i64 = 200;
const TERM_WEIGHT_SORT_TITLE: i64 = 300;
const TERM_WEIGHT_SLUG: i64 = 400;

const DIRECT_EXACT_BASE_RANK: i64 = 0;
const DIRECT_PREFIX_BASE_RANK: i64 = 1_000;
const DIRECT_CONTAINS_BASE_RANK: i64 = 2_000;
const TYPO_BASE_RANK: i64 = 3_000;
const TYPO_TOP_LIMIT: i64 = 50;
const MAX_NORMALIZED_QUERY_CHARS: usize = 512;
const MAX_TYPO_QUERY_TOKENS: usize = 16;

#[derive(Clone, Debug)]
pub struct TitleSearchPlan {
    normalized_query: String,
    query_tokens: Vec<String>,
    facets: Vec<MediaFacet>,
}

#[derive(Clone, Debug)]
pub struct TitleSearchTerm {
    pub term_kind: &'static str,
    pub raw_term: String,
    pub normalized_term: String,
    pub weight: i64,
}

#[derive(Clone, Debug)]
pub struct TitleSearchProjectionSource {
    pub title_id: String,
    pub facet: MediaFacet,
    pub name: String,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub aliases: Vec<String>,
    pub tagged_aliases: Vec<TaggedAlias>,
}

impl From<&Title> for TitleSearchProjectionSource {
    fn from(title: &Title) -> Self {
        Self {
            title_id: title.id.clone(),
            facet: title.facet.clone(),
            name: title.name.clone(),
            sort_title: title.sort_title.clone(),
            slug: title.slug.clone(),
            aliases: title.aliases.clone(),
            tagged_aliases: title.tagged_aliases.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum DirectLane {
    Exact,
    Prefix,
    Contains,
}

impl DirectLane {
    fn base_rank(self) -> i64 {
        match self {
            Self::Exact => DIRECT_EXACT_BASE_RANK,
            Self::Prefix => DIRECT_PREFIX_BASE_RANK,
            Self::Contains => DIRECT_CONTAINS_BASE_RANK,
        }
    }
}

pub fn normalize_title_search_text(raw: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_space = true;

    for ch in raw.nfd().flat_map(char::to_lowercase) {
        if is_combining_mark(ch) {
            continue;
        }
        if ch.is_alphanumeric() {
            normalized.push(ch);
            last_was_space = false;
            continue;
        }
        if ch == '&' {
            if !last_was_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push_str("and");
            normalized.push(' ');
            last_was_space = true;
            continue;
        }
        if !last_was_space && !normalized.is_empty() {
            normalized.push(' ');
            last_was_space = true;
        }
    }

    collapse_title_initialisms(&normalized.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn collapse_title_initialisms(raw: &str) -> String {
    let tokens = raw.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return String::new();
    }

    let mut collapsed = Vec::with_capacity(tokens.len());
    let mut index = 0usize;
    while index < tokens.len() {
        let is_initial = |token: &str| {
            token.chars().count() == 1
                && token.chars().next().is_some_and(|ch| ch.is_alphanumeric())
        };

        if !is_initial(tokens[index]) {
            collapsed.push(tokens[index].to_string());
            index += 1;
            continue;
        }

        let start = index;
        while index < tokens.len() && is_initial(tokens[index]) {
            index += 1;
        }

        if index - start >= 2 {
            collapsed.push(tokens[start..index].join(""));
        } else {
            collapsed.push(tokens[start].to_string());
        }
    }

    collapsed.join(" ")
}

fn facet_langid(facet: &MediaFacet) -> i64 {
    match facet {
        MediaFacet::Movie => 1,
        MediaFacet::Series => 2,
        MediaFacet::Anime => 3,
    }
}

fn truncate_chars(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value;
    }
    value.chars().take(max_chars).collect()
}

fn max_typo_distance(query_char_count: usize) -> i64 {
    match query_char_count {
        0..=5 => 100,
        6..=10 => 150,
        _ => 200,
    }
}

fn max_typo_length_delta(query_char_count: usize) -> i64 {
    match query_char_count {
        0..=5 => 1,
        6..=10 => 2,
        _ => 3,
    }
}

fn typo_scope(query_char_count: usize) -> i64 {
    match query_char_count {
        0..=8 => 3,
        _ => 2,
    }
}

fn spellfix_rank_for_weight(weight: i64) -> i64 {
    match weight {
        TERM_WEIGHT_NAME => 10_000,
        TERM_WEIGHT_ALIAS => 5_000,
        TERM_WEIGHT_TAGGED_ALIAS => 4_000,
        TERM_WEIGHT_SORT_TITLE => 2_000,
        TERM_WEIGHT_SLUG => 1_000,
        _ => 1,
    }
}

fn typo_boundary_chars(query_token: &str) -> Option<(String, String)> {
    let mut chars = query_token.chars();
    let first = chars.next()?;
    let last = query_token.chars().last()?;
    Some((first.to_string(), last.to_string()))
}

pub fn build_title_search_plan(facet: Option<MediaFacet>, query: &str) -> Option<TitleSearchPlan> {
    let normalized_query = truncate_chars(
        normalize_title_search_text(query),
        MAX_NORMALIZED_QUERY_CHARS,
    );
    if normalized_query.is_empty() {
        return None;
    }

    let query_tokens = normalized_query
        .split_whitespace()
        .filter(|token| token.chars().count() >= 4)
        .take(MAX_TYPO_QUERY_TOKENS)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let facets = facet
        .map(|facet| vec![facet])
        .unwrap_or_else(|| vec![MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]);

    Some(TitleSearchPlan {
        normalized_query,
        query_tokens,
        facets,
    })
}

pub fn push_ranked_title_matches_cte(builder: &mut QueryBuilder<Sqlite>, plan: &TitleSearchPlan) {
    builder.push("WITH direct_title_matches(title_id, rank) AS (");
    push_direct_match_select(
        builder,
        plan,
        DirectLane::Exact,
        plan.normalized_query.clone(),
    );
    builder.push(" UNION ALL ");
    push_direct_match_select(
        builder,
        plan,
        DirectLane::Prefix,
        format!("{}%", plan.normalized_query),
    );
    builder.push(" UNION ALL ");
    push_direct_match_select(
        builder,
        plan,
        DirectLane::Contains,
        format!("%{}%", plan.normalized_query),
    );
    builder.push(
        "), typo_candidate_matches(title_id, token_key, candidate_weight, candidate_distance) AS (",
    );
    push_typo_candidate_matches(builder, plan);
    builder.push("), typo_token_matches(title_id, token_key, best_weight, best_distance) AS (");
    push_typo_token_matches(builder, plan);
    builder.push(
        "), typo_title_matches(title_id, rank) AS (
             SELECT title_id,
                    ",
    );
    builder.push_bind(TYPO_BASE_RANK);
    builder.push(" + (");
    builder.push_bind(plan.query_tokens.len() as i64);
    builder.push(
        " - COUNT(DISTINCT token_key)) * 50
                    + SUM(best_distance) * 100
                    + MIN(best_weight) AS rank
             FROM typo_token_matches
             GROUP BY title_id
             HAVING COUNT(DISTINCT token_key) >= ",
    );
    builder.push_bind(required_typo_token_matches(plan.query_tokens.len()));
    builder.push(
        "), ranked_title_matches(title_id, rank) AS (
             SELECT title_id, MIN(rank) AS rank
             FROM (
                 SELECT title_id, rank FROM direct_title_matches
                 UNION ALL
                 SELECT title_id, rank FROM typo_title_matches
             )
             GROUP BY title_id
         ) ",
    );
}

fn required_typo_token_matches(token_count: usize) -> i64 {
    if token_count <= 1 { 1 } else { 2 }
}

fn push_direct_match_select(
    builder: &mut QueryBuilder<Sqlite>,
    plan: &TitleSearchPlan,
    lane: DirectLane,
    pattern: String,
) {
    builder.push("SELECT title_id, MIN(");
    builder.push_bind(lane.base_rank());
    builder.push(
        " + weight) AS rank
         FROM title_search_terms
         WHERE term_kind NOT LIKE '%_token' AND ",
    );
    match lane {
        DirectLane::Exact => {
            builder.push("normalized_term = ");
        }
        DirectLane::Prefix | DirectLane::Contains => {
            builder.push("normalized_term LIKE ");
        }
    }
    builder.push_bind(pattern);
    push_facet_filter(builder, &plan.facets);
    builder.push(" GROUP BY title_id");
}

fn push_typo_token_matches(builder: &mut QueryBuilder<Sqlite>, plan: &TitleSearchPlan) {
    if plan.query_tokens.is_empty() || plan.normalized_query.chars().count() < 4 {
        builder.push("SELECT NULL, NULL, NULL, NULL WHERE 0");
        return;
    }

    builder.push(
        "SELECT title_id,
                token_key,
                MIN(candidate_weight) AS best_weight,
                MIN(candidate_distance) AS best_distance
         FROM typo_candidate_matches
         GROUP BY title_id, token_key",
    );
}

fn push_typo_candidate_matches(builder: &mut QueryBuilder<Sqlite>, plan: &TitleSearchPlan) {
    if plan.query_tokens.is_empty() {
        builder.push("SELECT NULL, NULL, NULL, NULL WHERE 0");
        return;
    }

    let mut first = true;
    for query_token in &plan.query_tokens {
        for facet in &plan.facets {
            if !first {
                builder.push(" UNION ALL ");
            }
            first = false;
            push_spellfix_token_candidate_select(builder, plan, query_token, facet);
            builder.push(" UNION ALL ");
            push_edit_distance_token_candidate_select(builder, plan, query_token, facet);
        }
    }
}

fn push_spellfix_token_candidate_select(
    builder: &mut QueryBuilder<Sqlite>,
    plan: &TitleSearchPlan,
    query_token: &str,
    facet: &MediaFacet,
) {
    builder.push(
        "SELECT terms.title_id AS title_id,
                ",
    );
    builder.push_bind(query_token.to_string());
    builder.push(
        " AS token_key,
                MIN(terms.weight) AS candidate_weight,
                MIN(spellfix.distance) AS candidate_distance
         FROM title_search_spellfix spellfix
         JOIN title_search_terms terms ON terms.term_id = spellfix.rowid
         WHERE spellfix.word MATCH ",
    );
    builder.push_bind(query_token.to_string());
    builder.push(" AND spellfix.top = ");
    builder.push_bind(TYPO_TOP_LIMIT);
    builder.push(" AND spellfix.scope = ");
    builder.push_bind(typo_scope(query_token.chars().count()));
    builder.push(" AND spellfix.distance <= ");
    builder.push_bind(max_typo_distance(query_token.chars().count()));
    builder.push(" AND ABS(length(terms.normalized_term) - ");
    builder.push_bind(query_token.chars().count() as i64);
    builder.push(") <= ");
    builder.push_bind(max_typo_length_delta(query_token.chars().count()));
    if let Some((first_char, last_char)) = typo_boundary_chars(query_token) {
        builder.push(" AND substr(terms.normalized_term, 1, 1) = ");
        builder.push_bind(first_char);
        if query_token.chars().count() <= 5 || plan.query_tokens.len() == 1 {
            builder.push(" AND substr(terms.normalized_term, -1, 1) = ");
            builder.push_bind(last_char);
        }
    }
    builder.push(" AND spellfix.langid = ");
    builder.push_bind(facet_langid(facet));
    builder.push(" AND terms.facet = ");
    builder.push_bind(facet.as_str());
    builder.push(" AND terms.term_kind LIKE '%_token' GROUP BY terms.title_id");
}

fn push_edit_distance_token_candidate_select(
    builder: &mut QueryBuilder<Sqlite>,
    plan: &TitleSearchPlan,
    query_token: &str,
    facet: &MediaFacet,
) {
    builder.push(
        "SELECT terms.title_id AS title_id,
                ",
    );
    builder.push_bind(query_token.to_string());
    builder.push(
        " AS token_key,
                MIN(terms.weight) AS candidate_weight,
                MIN(editdist3(terms.normalized_term, ",
    );
    builder.push_bind(query_token.to_string());
    builder.push(
        ")) AS candidate_distance
         FROM title_search_terms terms
         WHERE terms.facet = ",
    );
    builder.push_bind(facet.as_str());
    builder.push(" AND terms.term_kind LIKE '%_token'");
    builder.push(" AND ABS(length(terms.normalized_term) - ");
    builder.push_bind(query_token.chars().count() as i64);
    builder.push(") <= ");
    builder.push_bind(max_typo_length_delta(query_token.chars().count()));
    if let Some((first_char, last_char)) = typo_boundary_chars(query_token) {
        builder.push(" AND substr(terms.normalized_term, 1, 1) = ");
        builder.push_bind(first_char);
        if query_token.chars().count() <= 5 || plan.query_tokens.len() == 1 {
            builder.push(" AND substr(terms.normalized_term, -1, 1) = ");
            builder.push_bind(last_char);
        }
    }
    builder.push(" AND editdist3(terms.normalized_term, ");
    builder.push_bind(query_token.to_string());
    builder.push(") <= ");
    builder.push_bind(max_typo_distance(query_token.chars().count()));
    builder.push(" GROUP BY terms.title_id");
}

fn push_facet_filter(builder: &mut QueryBuilder<Sqlite>, facets: &[MediaFacet]) {
    if facets.len() == 3 {
        return;
    }

    builder.push(" AND facet IN (");
    let mut separated = builder.separated(", ");
    for facet in facets {
        separated.push_bind(facet.as_str());
    }
    separated.push_unseparated(")");
}

fn push_term_with_tokens(
    terms: &mut Vec<TitleSearchTerm>,
    seen: &mut HashSet<(&'static str, String)>,
    term_kind: &'static str,
    token_term_kind: &'static str,
    weight: i64,
    raw_term: &str,
) {
    let raw_term = raw_term.trim();
    if raw_term.is_empty() {
        return;
    }

    let normalized_term = normalize_title_search_text(raw_term);
    if normalized_term.is_empty() {
        return;
    }

    if seen.insert((term_kind, normalized_term.clone())) {
        terms.push(TitleSearchTerm {
            term_kind,
            raw_term: raw_term.to_string(),
            normalized_term: normalized_term.clone(),
            weight,
        });
    }

    for token in normalized_term
        .split_whitespace()
        .filter(|token| token.chars().count() >= 4)
    {
        let token = token.to_string();
        if !seen.insert((token_term_kind, token.clone())) {
            continue;
        }
        terms.push(TitleSearchTerm {
            term_kind: token_term_kind,
            raw_term: token.clone(),
            normalized_term: token,
            weight,
        });
    }
}

pub fn build_title_search_terms(source: &TitleSearchProjectionSource) -> Vec<TitleSearchTerm> {
    let mut seen = HashSet::<(&'static str, String)>::new();
    let mut terms = Vec::new();

    push_term_with_tokens(
        &mut terms,
        &mut seen,
        TERM_KIND_NAME,
        TERM_KIND_NAME_TOKEN,
        TERM_WEIGHT_NAME,
        &source.name,
    );

    if let Some(sort_title) = source.sort_title.as_deref() {
        push_term_with_tokens(
            &mut terms,
            &mut seen,
            TERM_KIND_SORT_TITLE,
            TERM_KIND_SORT_TITLE_TOKEN,
            TERM_WEIGHT_SORT_TITLE,
            sort_title,
        );
    }

    if let Some(slug) = source.slug.as_deref() {
        push_term_with_tokens(
            &mut terms,
            &mut seen,
            TERM_KIND_SLUG,
            TERM_KIND_SLUG_TOKEN,
            TERM_WEIGHT_SLUG,
            slug,
        );
    }

    for alias in &source.aliases {
        push_term_with_tokens(
            &mut terms,
            &mut seen,
            TERM_KIND_ALIAS,
            TERM_KIND_ALIAS_TOKEN,
            TERM_WEIGHT_ALIAS,
            alias,
        );
    }

    for tagged_alias in &source.tagged_aliases {
        push_term_with_tokens(
            &mut terms,
            &mut seen,
            TERM_KIND_TAGGED_ALIAS,
            TERM_KIND_TAGGED_ALIAS_TOKEN,
            TERM_WEIGHT_TAGGED_ALIAS,
            &tagged_alias.name,
        );
    }

    terms
}

pub async fn delete_title_search_projection_tx(
    tx: &mut Transaction<'_, Sqlite>,
    title_id: &str,
) -> AppResult<()> {
    delete_title_search_projection_on_connection(tx, title_id).await
}

async fn delete_title_search_projection_on_connection(
    connection: &mut sqlx::SqliteConnection,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM title_search_spellfix
         WHERE rowid IN (
             SELECT term_id
             FROM title_search_terms
             WHERE title_id = ?
         )",
    )
    .bind(title_id)
    .execute(&mut *connection)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM title_search_terms WHERE title_id = ?")
        .bind(title_id)
        .execute(&mut *connection)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub async fn replace_title_search_projection_tx(
    tx: &mut Transaction<'_, Sqlite>,
    title: &Title,
) -> AppResult<()> {
    replace_title_search_projection_source_tx(tx, &TitleSearchProjectionSource::from(title)).await
}

pub async fn replace_title_search_projection_pg_tx(
    tx: &mut Transaction<'_, Postgres>,
    title: &Title,
) -> AppResult<()> {
    replace_title_search_projection_pg_source_tx(tx, &TitleSearchProjectionSource::from(title))
        .await
}

pub async fn replace_title_search_projection_pg_source_tx(
    tx: &mut Transaction<'_, Postgres>,
    source: &TitleSearchProjectionSource,
) -> AppResult<()> {
    sqlx::query("DELETE FROM title_search_terms WHERE title_id = $1")
        .bind(&source.title_id)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let facet = source.facet.as_str();
    for term in build_title_search_terms(source) {
        sqlx::query(
            "INSERT INTO title_search_terms
             (title_id, facet, term_kind, raw_term, normalized_term, weight)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (title_id, term_kind, normalized_term) DO UPDATE SET
                raw_term = EXCLUDED.raw_term,
                weight = EXCLUDED.weight",
        )
        .bind(&source.title_id)
        .bind(facet)
        .bind(term.term_kind)
        .bind(&term.raw_term)
        .bind(&term.normalized_term)
        .bind(term.weight)
        .execute(&mut **tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

async fn replace_title_search_projection_source_tx(
    connection: &mut sqlx::SqliteConnection,
    source: &TitleSearchProjectionSource,
) -> AppResult<()> {
    delete_title_search_projection_on_connection(connection, &source.title_id).await?;

    let facet = source.facet.as_str();
    let langid = facet_langid(&source.facet);

    for term in build_title_search_terms(source) {
        let term_id: i64 = sqlx::query_scalar(
            "INSERT INTO title_search_terms
             (title_id, facet, term_kind, raw_term, normalized_term, weight)
             VALUES (?, ?, ?, ?, ?, ?)
             RETURNING term_id",
        )
        .bind(&source.title_id)
        .bind(facet)
        .bind(term.term_kind)
        .bind(&term.raw_term)
        .bind(&term.normalized_term)
        .bind(term.weight)
        .fetch_one(&mut *connection)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

        sqlx::query(
            "INSERT INTO title_search_spellfix(rowid, word, rank, langid)
             VALUES (?, ?, ?, ?)",
        )
        .bind(term_id)
        .bind(&term.normalized_term)
        .bind(spellfix_rank_for_weight(term.weight))
        .bind(langid)
        .execute(&mut *connection)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    Ok(())
}

pub async fn seed_title_search_projection_if_empty(pool: &SqlitePool) -> AppResult<()> {
    let existing_term_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_search_terms")
        .fetch_one(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    if existing_term_count != 0 {
        return Ok(());
    }

    rebuild_title_search_projection(pool).await
}

pub async fn rebuild_title_search_projection(pool: &SqlitePool) -> AppResult<()> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    rebuild_title_search_projection_on_connection(&mut tx).await?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))
}

/// Rebuild on the caller's active transaction, including the catalog read.
/// The caller must commit or roll back this connection; restore uses its
/// existing BEGIN IMMEDIATE transaction so the catalog and index stay atomic.
pub async fn rebuild_title_search_projection_on_connection(
    connection: &mut sqlx::SqliteConnection,
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT id, name, facet, sort_title, slug, aliases, tagged_aliases_json
         FROM titles
         ORDER BY id ASC",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM title_search_terms")
        .execute(&mut *connection)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query("DELETE FROM title_search_spellfix")
        .execute(&mut *connection)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    for row in rows {
        let facet_raw: String = row
            .try_get("facet")
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let aliases_json: String = row.try_get("aliases").unwrap_or_else(|_| "[]".to_string());
        let tagged_aliases_json: String = row
            .try_get("tagged_aliases_json")
            .unwrap_or_else(|_| "[]".to_string());

        let source = TitleSearchProjectionSource {
            title_id: row
                .try_get("id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            facet: MediaFacet::parse(&facet_raw).unwrap_or_default(),
            name: row
                .try_get("name")
                .map_err(|err| AppError::Repository(err.to_string()))?,
            sort_title: row.try_get("sort_title").unwrap_or(None),
            slug: row.try_get("slug").unwrap_or(None),
            aliases: serde_json::from_str(&aliases_json)
                .map_err(|err| AppError::Repository(err.to_string()))?,
            tagged_aliases: serde_json::from_str(&tagged_aliases_json)
                .map_err(|err| AppError::Repository(err.to_string()))?,
        };

        replace_title_search_projection_source_tx(connection, &source).await?;
    }

    Ok(())
}
