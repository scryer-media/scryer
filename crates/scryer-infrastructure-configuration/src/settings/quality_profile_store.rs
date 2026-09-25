use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, AudioCodec, QualityProfile, QualityProfileCriteria,
    QualityProfileRepository, ReleaseSource, ScoringConfig, VideoCodec,
};
use serde_json::{Value as JsonValue, json};
use sqlx::{Row, types::Json};

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};
use crate::settings::read_cache::{CacheLookup, GenerationCache};

const QUALITY_PROFILE_COLUMNS: &str = "id, name, scope, scope_id, archival_quality,
    allow_unknown_quality, atmos_preferred, dolby_vision_allowed, detected_hdr_allowed,
    prefer_remux, allow_bd_disk, allow_upgrades, prefer_dual_audio,
    required_audio_languages, scoring_config";

/// One entry per `(scope, scope_id)` that has been listed. Profiles are
/// system- and title-scoped in practice, so this stays small; the bound only
/// guards against an unexpected key space.
const QUALITY_PROFILE_CACHE_MAX_ENTRIES: usize = 1_024;

/// Profile ids per child-table query, well under the bind limits of both
/// engines.
const CHILD_ROWS_ID_CHUNK: usize = 500;

/// The child tables of `quality_profiles`, in the order a profile decodes
/// them, with the column and ordering each list is stored under.
const TIERS: ChildTable = ChildTable {
    table: "quality_profile_quality_tiers",
    column: "quality_tier",
    order_by: "sort_order ASC",
};
const SOURCE_ALLOWLIST: ChildTable = ChildTable {
    table: "quality_profile_source_allowlist",
    column: "source",
    order_by: "source ASC",
};
const SOURCE_BLOCKLIST: ChildTable = ChildTable {
    table: "quality_profile_source_blocklist",
    column: "source",
    order_by: "source ASC",
};
const VIDEO_CODEC_ALLOWLIST: ChildTable = ChildTable {
    table: "quality_profile_video_codec_allowlist",
    column: "codec",
    order_by: "codec ASC",
};
const VIDEO_CODEC_BLOCKLIST: ChildTable = ChildTable {
    table: "quality_profile_video_codec_blocklist",
    column: "codec",
    order_by: "codec ASC",
};
const AUDIO_CODEC_ALLOWLIST: ChildTable = ChildTable {
    table: "quality_profile_audio_codec_allowlist",
    column: "codec",
    order_by: "codec ASC",
};
const AUDIO_CODEC_BLOCKLIST: ChildTable = ChildTable {
    table: "quality_profile_audio_codec_blocklist",
    column: "codec",
    order_by: "codec ASC",
};
const CHILD_TABLES: [ChildTable; 7] = [
    TIERS,
    SOURCE_ALLOWLIST,
    SOURCE_BLOCKLIST,
    VIDEO_CODEC_ALLOWLIST,
    VIDEO_CODEC_BLOCKLIST,
    AUDIO_CODEC_ALLOWLIST,
    AUDIO_CODEC_BLOCKLIST,
];

#[derive(Clone, Copy)]
struct ChildTable {
    table: &'static str,
    column: &'static str,
    order_by: &'static str,
}

/// Each child table's values for one profile, in stored order, indexed like
/// [`CHILD_TABLES`].
type ChildValues = [Vec<String>; 7];

type ProfileCacheKey = (String, Option<String>);

#[derive(Clone)]
pub struct QualityProfileStore {
    datastore: StoreDatastore,
    /// `list_quality_profiles` results until a write through this store
    /// invalidates them. See [`crate::settings::read_cache`] for the fill
    /// fence.
    cache: Arc<GenerationCache<ProfileCacheKey, Arc<Vec<QualityProfile>>>>,
}

impl QualityProfileStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self {
            datastore,
            cache: Arc::new(GenerationCache::new(QUALITY_PROFILE_CACHE_MAX_ENTRIES)),
        }
    }

    /// Drop every cached listing. Writes through this store invalidate on
    /// their own; this is for code that changes the profile tables without
    /// going through the store.
    pub fn invalidate_cache(&self) {
        self.cache.invalidate_all();
    }
}

/// Invalidates on drop, so a write invalidates after it settles, and also
/// when its future is dropped mid-flight, where the commit may already have
/// landed.
struct InvalidateOnDrop<'a>(&'a GenerationCache<ProfileCacheKey, Arc<Vec<QualityProfile>>>);

impl Drop for InvalidateOnDrop<'_> {
    fn drop(&mut self) {
        self.0.invalidate_all();
    }
}

#[async_trait]
impl QualityProfileRepository for QualityProfileStore {
    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        let scope = scope.trim().to_string();
        if scope.is_empty() {
            return Err(AppError::Validation(
                "scope is required to list quality profiles".into(),
            ));
        }

        let key = (scope, normalize_scope_id(scope_id));
        let ticket = match self.cache.lookup(&(), &key) {
            CacheLookup::Hit(profiles) => return Ok(profiles.as_ref().clone()),
            CacheLookup::Miss(ticket) => ticket,
        };
        let profiles =
            Arc::new(load_quality_profiles(&self.datastore, &key.0, key.1.clone()).await?);
        #[cfg(test)]
        self.cache.before_fill().await;
        self.cache.fill(ticket, (), key, profiles.clone());
        Ok(profiles.as_ref().clone())
    }

    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        let scope = scope.trim().to_string();
        if scope.is_empty() {
            return Err(AppError::Validation(
                "scope is required to replace quality profiles".into(),
            ));
        }

        let normalized_scope_id = normalize_scope_id(scope_id);
        // Everything, not just this scope: the upsert is keyed on profile id
        // and can move a profile out of another scope.
        let _invalidate = InvalidateOnDrop(&self.cache);
        SqlRuntime::run_in_transaction(&self.datastore, "replace_quality_profiles", move |tx| {
            let scope = scope.clone();
            let scope_id = normalized_scope_id.clone();
            let profiles = profiles.clone();
            Box::pin(async move {
                if let Some(scope_id) = scope_id.as_ref() {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM quality_profiles WHERE scope = {} AND scope_id = {}",
                        &[SqlArg::Text(scope.clone()), SqlArg::Text(scope_id.clone())],
                    )
                    .await?;
                } else {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM quality_profiles WHERE scope = {} AND scope_id IS NULL",
                        &[SqlArg::Text(scope.clone())],
                    )
                    .await?;
                }

                for profile in profiles {
                    upsert_quality_profile_tx(tx, &scope, scope_id.as_ref(), profile).await?;
                }

                Ok(())
            })
        })
        .await
    }
}

async fn load_quality_profiles(
    datastore: &StoreDatastore,
    scope: &str,
    scope_id: Option<String>,
) -> AppResult<Vec<QualityProfile>> {
    let rows = fetch_profile_rows(datastore, scope, scope_id).await?;
    let ids = rows
        .iter()
        .map(|row| row.text("id"))
        .collect::<AppResult<Vec<_>>>()?;
    let mut children = load_child_values(datastore, &ids, CHILD_ROWS_ID_CHUNK).await?;
    let mut out = Vec::with_capacity(rows.len());
    for (row, id) in rows.iter().zip(ids) {
        let values = children.remove(&id).unwrap_or_default();
        out.push(decode_quality_profile(row, id, values)?);
    }
    Ok(out)
}

async fn fetch_profile_rows(
    datastore: &StoreDatastore,
    scope: &str,
    scope_id: Option<String>,
) -> AppResult<Vec<SqlRow>> {
    let (sql, args) = if let Some(scope_id) = scope_id {
        (
            format!(
                "SELECT {QUALITY_PROFILE_COLUMNS}
                   FROM quality_profiles
                  WHERE scope = {{}} AND scope_id = {{}}
                  ORDER BY name"
            ),
            vec![SqlArg::Text(scope.to_string()), SqlArg::Text(scope_id)],
        )
    } else {
        (
            format!(
                "SELECT {QUALITY_PROFILE_COLUMNS}
                   FROM quality_profiles
                  WHERE scope = {{}} AND scope_id IS NULL
                  ORDER BY name"
            ),
            vec![SqlArg::Text(scope.to_string())],
        )
    };
    SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await
}

/// Every child-table value of `profile_ids`, one query per table (per chunk
/// of ids) instead of one per table per profile. Each profile's values keep
/// the order the per-profile query returned them in.
async fn load_child_values(
    datastore: &StoreDatastore,
    profile_ids: &[String],
    chunk_size: usize,
) -> AppResult<HashMap<String, ChildValues>> {
    let mut children: HashMap<String, ChildValues> = HashMap::with_capacity(profile_ids.len());
    if profile_ids.is_empty() {
        return Ok(children);
    }
    for chunk in profile_ids.chunks(chunk_size.max(1)) {
        let placeholders = std::iter::repeat_n("{}", chunk.len())
            .collect::<Vec<_>>()
            .join(", ");
        let args = chunk.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
        for (index, child) in CHILD_TABLES.iter().enumerate() {
            let sql = format!(
                "SELECT profile_id, {column} AS value
                   FROM {table}
                  WHERE profile_id IN ({placeholders})
                  ORDER BY {order_by}",
                column = child.column,
                table = child.table,
                order_by = child.order_by,
            );
            let rows = SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await?;
            for row in rows.iter() {
                children.entry(row.text("profile_id")?).or_default()[index]
                    .push(row.text("value")?);
            }
        }
    }
    Ok(children)
}

fn decode_quality_profile(
    row: &SqlRow,
    id: String,
    children: ChildValues,
) -> AppResult<QualityProfile> {
    let [
        quality_tiers,
        source_allowlist,
        source_blocklist,
        video_codec_allowlist,
        video_codec_blocklist,
        audio_codec_allowlist,
        audio_codec_blocklist,
    ] = children;
    let archival_quality = row.opt_text("archival_quality")?.and_then(|value| {
        let value = value.trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    });
    let required_audio_languages: Vec<String> = serde_json::from_value(row_json_or_default(
        row,
        "required_audio_languages",
        json!([]),
    )?)
    .unwrap_or_default();
    let scoring_config: ScoringConfig =
        serde_json::from_value(row_json_or_default(row, "scoring_config", json!({}))?)
            .unwrap_or_default();

    Ok(QualityProfile {
        id,
        name: row.text("name")?,
        criteria: QualityProfileCriteria {
            quality_tiers,
            archival_quality,
            allow_unknown_quality: row.bool("allow_unknown_quality")?,
            source_allowlist: parse_release_sources(source_allowlist)?,
            source_blocklist: parse_release_sources(source_blocklist)?,
            video_codec_allowlist: parse_video_codecs(video_codec_allowlist)?,
            video_codec_blocklist: parse_video_codecs(video_codec_blocklist)?,
            audio_codec_allowlist: parse_audio_codecs(audio_codec_allowlist)?,
            audio_codec_blocklist: parse_audio_codecs(audio_codec_blocklist)?,
            atmos_preferred: row.bool("atmos_preferred")?,
            dolby_vision_allowed: row.bool("dolby_vision_allowed")?,
            detected_hdr_allowed: row.bool("detected_hdr_allowed")?,
            prefer_remux: row.bool("prefer_remux")?,
            allow_bd_disk: row.bool("allow_bd_disk")?,
            allow_upgrades: row.bool("allow_upgrades")?,
            prefer_dual_audio: row.bool("prefer_dual_audio").unwrap_or(false),
            required_audio_languages,
            scoring_persona: scoring_config.scoring_persona,
            scoring_overrides: scoring_config.scoring_overrides,
            cutoff_tier: scoring_config.cutoff_tier,
            min_score_to_grab: scoring_config.min_score_to_grab,
            cutoff_score: scoring_config.cutoff_score,
            facet_persona_overrides: scoring_config.facet_persona_overrides,
        },
    })
}

fn parse_release_sources(values: Vec<String>) -> AppResult<Vec<ReleaseSource>> {
    values
        .into_iter()
        .map(|value| {
            ReleaseSource::parse(value.as_str())
                .ok_or_else(|| repo_err(format!("invalid stored release source {value:?}")))
        })
        .collect()
}

fn parse_video_codecs(values: Vec<String>) -> AppResult<Vec<VideoCodec>> {
    values
        .into_iter()
        .map(|value| {
            VideoCodec::parse(value.as_str())
                .ok_or_else(|| repo_err(format!("invalid stored video codec {value:?}")))
        })
        .collect()
}

fn parse_audio_codecs(values: Vec<String>) -> AppResult<Vec<AudioCodec>> {
    values
        .into_iter()
        .map(|value| {
            AudioCodec::parse(value.as_str())
                .ok_or_else(|| repo_err(format!("invalid stored audio codec {value:?}")))
        })
        .collect()
}

async fn upsert_quality_profile_tx(
    tx: &mut SqlTx<'_>,
    scope: &str,
    scope_id: Option<&String>,
    profile: QualityProfile,
) -> AppResult<()> {
    let id = profile.id.trim().to_string();
    if id.is_empty() {
        return Ok(());
    }

    let name = profile.name.trim().to_string();
    let criteria = profile.criteria;
    let archival_quality = criteria
        .archival_quality
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let quality_tiers = normalize_profile_string_values(criteria.quality_tiers);
    let source_allowlist = normalize_profile_display_values(criteria.source_allowlist);
    let source_blocklist = normalize_profile_display_values(criteria.source_blocklist);
    let video_codec_allowlist = normalize_profile_display_values(criteria.video_codec_allowlist);
    let video_codec_blocklist = normalize_profile_display_values(criteria.video_codec_blocklist);
    let audio_codec_allowlist = normalize_profile_display_values(criteria.audio_codec_allowlist);
    let audio_codec_blocklist = normalize_profile_display_values(criteria.audio_codec_blocklist);
    let required_audio_languages = serde_json::to_value(&criteria.required_audio_languages)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let scoring_config = serde_json::to_value(ScoringConfig {
        scoring_persona: criteria.scoring_persona,
        scoring_overrides: criteria.scoring_overrides,
        cutoff_tier: criteria.cutoff_tier,
        min_score_to_grab: criteria.min_score_to_grab,
        cutoff_score: criteria.cutoff_score,
        facet_persona_overrides: criteria.facet_persona_overrides,
    })
    .map_err(|error| AppError::Repository(error.to_string()))?;

    clear_quality_profile_value_rows(tx, &id).await?;

    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO quality_profiles
            (id, name, scope, scope_id, archival_quality, allow_unknown_quality,
             atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux,
             allow_bd_disk, allow_upgrades, prefer_dual_audio, required_audio_languages,
             scoring_config, created_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            scope = excluded.scope,
            scope_id = excluded.scope_id,
            archival_quality = excluded.archival_quality,
            allow_unknown_quality = excluded.allow_unknown_quality,
            atmos_preferred = excluded.atmos_preferred,
            dolby_vision_allowed = excluded.dolby_vision_allowed,
            detected_hdr_allowed = excluded.detected_hdr_allowed,
            prefer_remux = excluded.prefer_remux,
            allow_bd_disk = excluded.allow_bd_disk,
            allow_upgrades = excluded.allow_upgrades,
            prefer_dual_audio = excluded.prefer_dual_audio,
            required_audio_languages = excluded.required_audio_languages,
            scoring_config = excluded.scoring_config",
        &[
            SqlArg::Text(id.clone()),
            SqlArg::Text(name),
            SqlArg::Text(scope.to_string()),
            SqlArg::OptText(scope_id.cloned()),
            SqlArg::OptText(archival_quality),
            SqlArg::Bool(criteria.allow_unknown_quality),
            SqlArg::Bool(criteria.atmos_preferred),
            SqlArg::Bool(criteria.dolby_vision_allowed),
            SqlArg::Bool(criteria.detected_hdr_allowed),
            SqlArg::Bool(criteria.prefer_remux),
            SqlArg::Bool(criteria.allow_bd_disk),
            SqlArg::Bool(criteria.allow_upgrades),
            SqlArg::Bool(criteria.prefer_dual_audio),
            SqlArg::Json(required_audio_languages),
            SqlArg::Json(scoring_config),
            SqlArg::Timestamp(Utc::now()),
        ],
    )
    .await?;

    insert_quality_profile_quality_tiers(tx, &id, &quality_tiers).await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_source_allowlist",
        "source",
        &id,
        &source_allowlist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_source_blocklist",
        "source",
        &id,
        &source_blocklist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_video_codec_allowlist",
        "codec",
        &id,
        &video_codec_allowlist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_video_codec_blocklist",
        "codec",
        &id,
        &video_codec_blocklist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_audio_codec_allowlist",
        "codec",
        &id,
        &audio_codec_allowlist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_audio_codec_blocklist",
        "codec",
        &id,
        &audio_codec_blocklist,
    )
    .await?;

    Ok(())
}

async fn clear_quality_profile_value_rows(tx: &mut SqlTx<'_>, profile_id: &str) -> AppResult<()> {
    for table in [
        "quality_profile_quality_tiers",
        "quality_profile_source_allowlist",
        "quality_profile_source_blocklist",
        "quality_profile_video_codec_allowlist",
        "quality_profile_video_codec_blocklist",
        "quality_profile_audio_codec_allowlist",
        "quality_profile_audio_codec_blocklist",
    ] {
        let sql = format!("DELETE FROM {table} WHERE profile_id = {{}}");
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            &sql,
            &[SqlArg::Text(profile_id.to_string())],
        )
        .await?;
    }
    Ok(())
}

async fn insert_quality_profile_quality_tiers(
    tx: &mut SqlTx<'_>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for (index, value) in values.iter().enumerate() {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO quality_profile_quality_tiers(profile_id, quality_tier, sort_order)
             VALUES ({}, {}, {})",
            &[
                SqlArg::Text(profile_id.to_string()),
                SqlArg::Text(value.clone()),
                SqlArg::I64(index as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_quality_profile_values(
    tx: &mut SqlTx<'_>,
    table: &str,
    column: &str,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    let sql = format!("INSERT INTO {table}(profile_id, {column}) VALUES ({{}}, {{}})");
    for value in values {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            &sql,
            &[
                SqlArg::Text(profile_id.to_string()),
                SqlArg::Text(value.clone()),
            ],
        )
        .await?;
    }
    Ok(())
}

/// The per-profile read the batched load replaced, kept to check it against.
#[cfg(test)]
async fn list_quality_profile_values(
    datastore: &StoreDatastore,
    table: &str,
    column: &str,
    order_by: &str,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let sql = format!(
        "SELECT {column} AS value
           FROM {table}
          WHERE profile_id = {{}}
          ORDER BY {order_by}"
    );
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        &sql,
        &[SqlArg::Text(profile_id.to_string())],
    )
    .await?;

    rows.iter().map(|row| row.text("value")).collect()
}

fn row_json_or_default(row: &SqlRow, column: &str, default: JsonValue) -> AppResult<JsonValue> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
            let Some(raw) = raw else {
                return Ok(default);
            };
            serde_json::from_str(&raw).or(Ok(default))
        }
        SqlRow::Postgres(row) => {
            let raw: Option<Json<JsonValue>> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.map(|value| value.0).unwrap_or(default))
        }
    }
}

fn normalize_profile_string_values(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }
    normalized
}

fn normalize_profile_display_values<T: ToString>(values: Vec<T>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let key = value.to_string();
        if seen.insert(key.clone()) {
            normalized.push(key);
        }
    }
    normalized
}

fn normalize_scope_id(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

    use super::*;

    const SCOPE: &str = "system";
    const TITLE_SCOPE: &str = "title";

    const PROFILE_DDL: &[&str] = &[
        "CREATE TABLE quality_profiles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            scope TEXT NOT NULL,
            scope_id TEXT,
            archival_quality TEXT,
            allow_unknown_quality INTEGER NOT NULL DEFAULT 0,
            atmos_preferred INTEGER NOT NULL DEFAULT 0,
            dolby_vision_allowed INTEGER NOT NULL DEFAULT 0,
            detected_hdr_allowed INTEGER NOT NULL DEFAULT 1,
            prefer_remux INTEGER NOT NULL DEFAULT 0,
            allow_bd_disk INTEGER NOT NULL DEFAULT 0,
            allow_upgrades INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            prefer_dual_audio INTEGER NOT NULL DEFAULT 0,
            required_audio_languages TEXT NOT NULL DEFAULT '[]',
            scoring_config TEXT NOT NULL DEFAULT '{}'
        )",
        "CREATE TABLE quality_profile_quality_tiers (
            profile_id TEXT NOT NULL,
            quality_tier TEXT NOT NULL,
            sort_order INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            PRIMARY KEY (profile_id, quality_tier),
            FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
        )",
    ];

    fn child_ddl(table: &str, column: &str) -> String {
        format!(
            "CREATE TABLE {table} (
                profile_id TEXT NOT NULL,
                {column} TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (profile_id, {column}),
                FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
            )"
        )
    }

    async fn fixture() -> (QualityProfileStore, SqlitePool) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        for ddl in PROFILE_DDL {
            sqlx::query(*ddl).execute(&pool).await.expect("profile ddl");
        }
        for child in &CHILD_TABLES[1..] {
            // Fixture DDL built from the static table list above.
            sqlx::query(sqlx::AssertSqlSafe(child_ddl(child.table, child.column)))
                .execute(&pool)
                .await
                .expect("child ddl");
        }
        let store = QualityProfileStore::new(StoreDatastore::sqlite(
            pool.clone(),
            Arc::new(tokio::sync::Mutex::new(())),
        ));
        (store, pool)
    }

    async fn insert_profile(pool: &SqlitePool, id: &str, name: &str, scope_id: Option<&str>) {
        let scope = if scope_id.is_some() {
            TITLE_SCOPE
        } else {
            SCOPE
        };
        sqlx::query(
            "INSERT INTO quality_profiles (id, name, scope, scope_id, created_at)
             VALUES (?1, ?2, ?3, ?4, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(name)
        .bind(scope)
        .bind(scope_id)
        .execute(pool)
        .await
        .expect("insert profile");
    }

    async fn insert_tier(pool: &SqlitePool, id: &str, tier: &str, sort_order: i64) {
        sqlx::query(
            "INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order)
             VALUES (?1, ?2, ?3)",
        )
        .bind(id)
        .bind(tier)
        .bind(sort_order)
        .execute(pool)
        .await
        .expect("insert tier");
    }

    async fn insert_child(pool: &SqlitePool, child: ChildTable, id: &str, value: &str) {
        // Table and column come from the static child-table list.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {} (profile_id, {}) VALUES (?1, ?2)",
            child.table, child.column
        )))
        .bind(id)
        .bind(value)
        .execute(pool)
        .await
        .expect("insert child row");
    }

    /// Three system profiles with lists stored out of order, one title-scoped
    /// profile, and one profile with no child rows at all.
    async fn seed_profiles(pool: &SqlitePool) {
        insert_profile(pool, "qp-b", "Fixture Beta", None).await;
        insert_tier(pool, "qp-b", "2160P", 2).await;
        insert_tier(pool, "qp-b", "1080P", 0).await;
        insert_tier(pool, "qp-b", "720P", 1).await;
        for source in ["WEB-DL", "BluRay", "HDTV"] {
            insert_child(pool, SOURCE_ALLOWLIST, "qp-b", source).await;
        }
        insert_child(pool, SOURCE_BLOCKLIST, "qp-b", "CAM").await;
        for codec in ["x265", "AV1", "H264"] {
            insert_child(pool, VIDEO_CODEC_ALLOWLIST, "qp-b", codec).await;
        }
        insert_child(pool, VIDEO_CODEC_BLOCKLIST, "qp-b", "XVID").await;
        for codec in ["TRUEHD", "AAC", "FLAC"] {
            insert_child(pool, AUDIO_CODEC_ALLOWLIST, "qp-b", codec).await;
        }
        insert_child(pool, AUDIO_CODEC_BLOCKLIST, "qp-b", "OPUS").await;

        insert_profile(pool, "qp-a", "Fixture Alpha", None).await;
        insert_tier(pool, "qp-a", "480P", 1).await;
        insert_tier(pool, "qp-a", "720P", 0).await;
        for source in ["HDTV", "DVD"] {
            insert_child(pool, SOURCE_BLOCKLIST, "qp-a", source).await;
        }
        for codec in ["DTS", "AC3"] {
            insert_child(pool, AUDIO_CODEC_BLOCKLIST, "qp-a", codec).await;
        }

        insert_profile(pool, "qp-c", "Fixture Gamma", None).await;

        insert_profile(pool, "qp-t", "Fixture Title", Some("title-fixture")).await;
        insert_tier(pool, "qp-t", "1080P", 0).await;
        insert_child(pool, VIDEO_CODEC_ALLOWLIST, "qp-t", "H265").await;
    }

    /// The per-profile read the batched load replaced: seven queries per
    /// profile, decoded the same way.
    async fn load_per_profile(
        datastore: &StoreDatastore,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        let rows = fetch_profile_rows(datastore, scope, scope_id).await?;
        let mut out = Vec::new();
        for row in rows.iter() {
            let id = row.text("id")?;
            let mut children = ChildValues::default();
            for (index, child) in CHILD_TABLES.iter().enumerate() {
                children[index] = list_quality_profile_values(
                    datastore,
                    child.table,
                    child.column,
                    child.order_by,
                    &id,
                )
                .await?;
            }
            out.push(decode_quality_profile(row, id, children)?);
        }
        Ok(out)
    }

    fn names(profiles: &[QualityProfile]) -> Vec<&str> {
        profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect()
    }

    #[tokio::test]
    async fn batched_load_matches_the_per_profile_load_in_content_and_order() {
        let (store, pool) = fixture().await;
        seed_profiles(&pool).await;

        for (scope, scope_id) in [
            (SCOPE, None),
            (TITLE_SCOPE, Some("title-fixture".to_string())),
        ] {
            let per_profile = load_per_profile(&store.datastore, scope, scope_id.clone())
                .await
                .expect("per-profile load");
            let batched = load_quality_profiles(&store.datastore, scope, scope_id.clone())
                .await
                .expect("batched load");
            assert_eq!(batched, per_profile);

            // Chunk boundaries fall between profiles without reordering.
            let rows = fetch_profile_rows(&store.datastore, scope, scope_id)
                .await
                .expect("profile rows");
            let ids = rows
                .iter()
                .map(|row| row.text("id").expect("id"))
                .collect::<Vec<_>>();
            let one_per_chunk = load_child_values(&store.datastore, &ids, 1)
                .await
                .expect("chunked child load");
            let all_at_once = load_child_values(&store.datastore, &ids, CHILD_ROWS_ID_CHUNK)
                .await
                .expect("child load");
            assert_eq!(one_per_chunk, all_at_once);
        }

        let profiles = store
            .list_quality_profiles(SCOPE, None)
            .await
            .expect("list profiles");
        assert_eq!(
            names(&profiles),
            ["Fixture Alpha", "Fixture Beta", "Fixture Gamma"]
        );
        let beta = &profiles[1].criteria;
        assert_eq!(beta.quality_tiers, ["1080P", "720P", "2160P"]);
        assert_eq!(
            beta.source_allowlist,
            [
                ReleaseSource::BluRay,
                ReleaseSource::Hdtv,
                ReleaseSource::WebDl
            ]
        );
        assert_eq!(
            beta.video_codec_allowlist,
            [VideoCodec::Av1, VideoCodec::H264, VideoCodec::H265]
        );
        assert_eq!(
            beta.audio_codec_allowlist,
            [AudioCodec::Aac, AudioCodec::Flac, AudioCodec::TrueHd]
        );
        let alpha = &profiles[0].criteria;
        assert_eq!(alpha.quality_tiers, ["720P", "480P"]);
        assert_eq!(
            alpha.source_blocklist,
            [ReleaseSource::Dvd, ReleaseSource::Hdtv]
        );
        let gamma = &profiles[2].criteria;
        assert!(gamma.quality_tiers.is_empty() && gamma.source_allowlist.is_empty());
    }

    #[tokio::test]
    async fn an_invalid_stored_value_keeps_its_error_and_is_not_cached() {
        let (store, pool) = fixture().await;
        seed_profiles(&pool).await;
        insert_child(&pool, VIDEO_CODEC_BLOCKLIST, "qp-a", "not-a-codec").await;

        let error = store
            .list_quality_profiles(SCOPE, None)
            .await
            .expect_err("invalid codec should fail the load");
        assert!(
            error
                .to_string()
                .contains("invalid stored video codec \"not-a-codec\""),
            "{error}"
        );

        sqlx::query("DELETE FROM quality_profile_video_codec_blocklist WHERE profile_id = 'qp-a'")
            .execute(&pool)
            .await
            .expect("repair row");
        store
            .list_quality_profiles(SCOPE, None)
            .await
            .expect("a failed load leaves nothing cached");
    }

    #[tokio::test]
    async fn repeated_listings_are_served_from_memory_until_invalidated() {
        let (store, pool) = fixture().await;
        seed_profiles(&pool).await;
        let first = store.list_quality_profiles(SCOPE, None).await.unwrap();

        // An external write: nothing through the store.
        sqlx::query("UPDATE quality_profiles SET name = 'Fixture Renamed' WHERE id = 'qp-a'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            store.list_quality_profiles(SCOPE, None).await.unwrap(),
            first
        );
        // Scope ids are normalized into the same key.
        assert_eq!(
            store
                .list_quality_profiles(" system ", Some("  ".to_string()))
                .await
                .unwrap(),
            first
        );

        store.invalidate_cache();
        let reread = store.list_quality_profiles(SCOPE, None).await.unwrap();
        assert!(names(&reread).contains(&"Fixture Renamed"));
    }

    #[tokio::test]
    async fn replacing_profiles_invalidates_every_scope() {
        let (store, pool) = fixture().await;
        seed_profiles(&pool).await;
        let system = store.list_quality_profiles(SCOPE, None).await.unwrap();
        let title_scope_id = Some("title-fixture".to_string());
        store
            .list_quality_profiles(TITLE_SCOPE, title_scope_id.clone())
            .await
            .unwrap();

        let mut beta = system[1].clone();
        beta.name = "Fixture Beta Edited".to_string();
        beta.criteria.quality_tiers = vec!["720P".to_string(), "1080P".to_string()];
        store
            .replace_quality_profiles(SCOPE, None, vec![system[0].clone(), beta])
            .await
            .unwrap();
        let replaced = store.list_quality_profiles(SCOPE, None).await.unwrap();
        assert_eq!(names(&replaced), ["Fixture Alpha", "Fixture Beta Edited"]);
        assert_eq!(replaced[1].criteria.quality_tiers, ["720P", "1080P"]);

        // The upsert is keyed on id, so writing qp-a into the title scope
        // moves it out of the system scope, whose listing must not keep it.
        store
            .replace_quality_profiles(
                TITLE_SCOPE,
                title_scope_id.clone(),
                vec![replaced[0].clone()],
            )
            .await
            .unwrap();
        assert_eq!(
            names(&store.list_quality_profiles(SCOPE, None).await.unwrap()),
            ["Fixture Beta Edited"]
        );
        assert_eq!(
            names(
                &store
                    .list_quality_profiles(TITLE_SCOPE, title_scope_id)
                    .await
                    .unwrap()
            ),
            ["Fixture Alpha"]
        );
    }

    #[tokio::test]
    async fn a_listing_that_raced_a_replace_does_not_cache_what_it_read() {
        let (store, pool) = fixture().await;
        seed_profiles(&pool).await;
        let before = store.list_quality_profiles(SCOPE, None).await.unwrap();
        store.invalidate_cache();

        let pause = store.cache.arm_fill_pause();
        let reader = tokio::spawn({
            let store = store.clone();
            async move { store.list_quality_profiles(SCOPE, None).await }
        });
        // The reader has read the old profiles and is parked before its fill.
        pause.reached.await.expect("reader reached its fill");
        store
            .replace_quality_profiles(SCOPE, None, vec![before[0].clone()])
            .await
            .unwrap();
        pause.release.send(()).expect("reader is waiting");
        assert_eq!(reader.await.unwrap().unwrap(), before);

        assert_eq!(store.cache.len(), 0);
        assert_eq!(
            names(&store.list_quality_profiles(SCOPE, None).await.unwrap()),
            ["Fixture Alpha"]
        );
    }
}
