use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};
use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, TitleImageBlob, TitleImageKind, TitleImageRepository,
    TitleImageSourceResult, TitleImageSyncTask, TitleImageVariantRecord, TitleImageVariantSpec,
};
use scryer_domain::{
    DomainEvent, DomainEventActorKind, DomainEventStream, Id, MediaFacet, NewDomainEvent,
};
use scryer_infrastructure_sql::domain_event_payload::{
    decode_domain_event_payload, derive_domain_event_projections, encode_domain_event_payload,
};

use super::{normalize_title_image_source_url, title_image_blob_digest};

const DOMAIN_EVENT_COLUMNS: &str = "sequence, event_id, occurred_at, actor_kind, actor_user_id, actor_display_name, title_id, facet, correlation_id, causation_id, schema_version, stream_kind, stream_id, event_type, payload_json";

#[derive(Clone)]
pub struct TitleImageStore {
    datastore: StoreDatastore,
}

impl TitleImageStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl TitleImageRepository for TitleImageStore {
    async fn list_title_image_refresh_work(
        &self,
        limit: usize,
        skipped: &[TitleImageSyncTask],
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        list_prioritized_refresh_work(&self.datastore, limit, skipped).await
    }

    async fn clear_title_image_cache(&self) -> AppResult<()> {
        SqlRuntime::run_in_transaction(&self.datastore, "clear_title_image_cache", move |tx| {
            Box::pin(async move { clear_title_image_cache_tx(tx).await })
        })
        .await
    }

    async fn upsert_title_image_source_result(
        &self,
        title_id: &str,
        result: TitleImageSourceResult,
        event: Option<NewDomainEvent>,
    ) -> AppResult<Option<DomainEvent>> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "upsert_title_image_source_result",
            move |tx| {
                let title_id = title_id.clone();
                let result = result.clone();
                let event = event.clone();
                Box::pin(async move {
                    upsert_title_image_source_result_tx(tx, &title_id, &result, event.as_ref())
                        .await
                })
            },
        )
        .await
    }

    async fn get_title_image_blob(
        &self,
        title_id: &str,
        kind: TitleImageKind,
        variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        fetch_title_image_blob(self.datastore.read_exec(), title_id, kind, variant_key).await
    }
}

const IMAGE_REFRESH_PRIORITIES: &[(TitleImageKind, &str, u32)] = &[
    (TitleImageKind::Poster, "w250", 250),
    (TitleImageKind::Poster, "w70", 70),
    (TitleImageKind::Fanart, "w1280", 1280),
];

async fn list_prioritized_refresh_work(
    datastore: &StoreDatastore,
    limit: usize,
    skipped: &[TitleImageSyncTask],
) -> AppResult<Vec<TitleImageSyncTask>> {
    for &(kind, variant_key, _) in IMAGE_REFRESH_PRIORITIES {
        let (sql, args) = build_variant_refresh_sql(kind, variant_key, limit, skipped);
        let candidates = fetch_refresh_candidates(datastore.read_exec(), &sql, &args, kind).await?;
        let mut tasks = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let variants = missing_variant_specs(
                datastore.read_exec(),
                candidate.kind,
                &candidate.title_id,
                &candidate.source_url,
            )
            .await?;
            if !variants.is_empty() {
                tasks.push(TitleImageSyncTask {
                    title_id: candidate.title_id,
                    kind: candidate.kind,
                    source_url: candidate.source_url,
                    variants,
                });
            }
        }
        if !tasks.is_empty() {
            return Ok(tasks);
        }
    }
    Ok(Vec::new())
}

fn build_variant_refresh_sql(
    kind: TitleImageKind,
    variant_key: &str,
    limit: usize,
    skipped: &[TitleImageSyncTask],
) -> (String, Vec<SqlArg>) {
    let source_col = match kind {
        TitleImageKind::Poster => "poster_url",
        TitleImageKind::Fanart => "background_url",
    };
    let skipped_for_kind = skipped
        .iter()
        .filter(|task| task.kind == kind)
        .collect::<Vec<_>>();
    let skipped_clause = skipped_for_kind
        .iter()
        .map(|_| format!("AND NOT (t.id = {{}} AND t.{source_col} = {{}})"))
        .collect::<Vec<_>>()
        .join("\n           ");

    let sql = format!(
        "SELECT t.id AS title_id, t.{source_col} AS source_url, ti.source_url AS cached_source_url
         FROM titles t
         LEFT JOIN title_images ti
           ON ti.title_id = t.id
          AND ti.kind = {{}}
         LEFT JOIN title_image_variants pv
           ON pv.title_image_id = ti.id
          AND pv.variant_key = {{}}
         WHERE NULLIF(TRIM(t.{source_col}), '') IS NOT NULL
           AND TRIM(t.{source_col}) NOT LIKE {{}}
           AND (
                ti.id IS NULL
                OR ti.source_url <> t.{source_col}
                OR pv.id IS NULL
           )
           {skipped_clause}
         ORDER BY t.created_at ASC
         LIMIT {{}}"
    );

    let mut args = vec![
        SqlArg::Text(kind.as_str().to_string()),
        SqlArg::Text(variant_key.to_string()),
        SqlArg::Text(local_title_image_route_pattern().to_string()),
    ];
    for task in skipped_for_kind {
        args.push(SqlArg::Text(task.title_id.clone()));
        args.push(SqlArg::Text(task.source_url.clone()));
    }
    args.push(SqlArg::I64(limit as i64));
    (sql, args)
}

fn local_title_image_route_pattern() -> &'static str {
    "/images/titles/%"
}

async fn fetch_refresh_tasks(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    kind: TitleImageKind,
) -> AppResult<Vec<RefreshCandidate>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| {
            Ok(RefreshCandidate {
                title_id: row.text("title_id")?,
                kind,
                source_url: row.text("source_url")?,
            })
        })
        .collect()
}

async fn fetch_refresh_candidates(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    kind: TitleImageKind,
) -> AppResult<Vec<RefreshCandidate>> {
    fetch_refresh_tasks(exec, sql, args, kind).await
}

#[derive(Clone, Debug)]
struct RefreshCandidate {
    title_id: String,
    kind: TitleImageKind,
    source_url: String,
}

fn variant_specs_for_kind(kind: TitleImageKind) -> Vec<TitleImageVariantSpec> {
    IMAGE_REFRESH_PRIORITIES
        .iter()
        .filter(|(candidate_kind, _, _)| *candidate_kind == kind)
        .map(|(_, variant_key, width)| TitleImageVariantSpec {
            variant_key: (*variant_key).to_string(),
            width: *width,
        })
        .collect()
}

async fn missing_variant_specs(
    exec: SqlExec<'_, '_>,
    kind: TitleImageKind,
    title_id: &str,
    source_url: &str,
) -> AppResult<Vec<TitleImageVariantSpec>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT ti.source_url, tiv.variant_key
           FROM title_images ti
           LEFT JOIN title_image_variants tiv ON tiv.title_image_id = ti.id
          WHERE ti.title_id = {} AND ti.kind = {}",
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(kind.as_str().to_string()),
        ],
    )
    .await?;
    let stored_source = rows.first().map(|row| row.text("source_url")).transpose()?;
    let normalized_source = normalize_title_image_source_url(source_url)?;
    let normalized_stored_source = stored_source
        .as_deref()
        .map(normalize_title_image_source_url)
        .transpose()?;
    let source_changed = normalized_stored_source
        .as_deref()
        .is_none_or(|stored_source| stored_source != normalized_source);
    let existing_keys = rows
        .iter()
        .filter_map(|row| row.opt_text("variant_key").transpose())
        .collect::<AppResult<std::collections::HashSet<_>>>()?;

    Ok(variant_specs_for_kind(kind)
        .into_iter()
        .filter(|spec| source_changed || !existing_keys.contains(spec.variant_key.as_str()))
        .collect())
}

async fn clear_title_image_cache_tx(tx: &mut SqlTx<'_>) -> AppResult<()> {
    for kind in [TitleImageKind::Poster, TitleImageKind::Fanart] {
        repair_local_title_image_source_tx(tx, kind).await?;
        clear_unrecoverable_local_title_image_source_tx(tx, kind).await?;
    }

    SqlRuntime::execute(SqlExec::Tx(tx), "DELETE FROM title_image_variants", &[]).await?;
    SqlRuntime::execute(SqlExec::Tx(tx), "DELETE FROM title_image_blobs", &[]).await?;
    SqlRuntime::execute(SqlExec::Tx(tx), "DELETE FROM title_images", &[]).await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE titles
            SET poster_local_path = NULL,
                background_local_path = NULL
          WHERE poster_local_path IS NOT NULL
             OR background_local_path IS NOT NULL",
        &[],
    )
    .await?;

    Ok(())
}

fn title_image_source_columns(kind: TitleImageKind) -> (&'static str, &'static str) {
    match kind {
        TitleImageKind::Poster => ("poster_url", "poster_local_path"),
        TitleImageKind::Fanart => ("background_url", "background_local_path"),
    }
}

async fn repair_local_title_image_source_tx(
    tx: &mut SqlTx<'_>,
    kind: TitleImageKind,
) -> AppResult<()> {
    let (source_col, _) = title_image_source_columns(kind);
    let sql = format!(
        "UPDATE titles
            SET {source_col} = (
                SELECT ti.source_url
                  FROM title_images ti
                 WHERE ti.title_id = titles.id
                   AND ti.kind = {{}}
                   AND NULLIF(TRIM(ti.source_url), '') IS NOT NULL
                   AND TRIM(ti.source_url) NOT LIKE {{}}
                 LIMIT 1
            )
          WHERE TRIM(COALESCE({source_col}, '')) LIKE {{}}
            AND EXISTS (
                SELECT 1
                  FROM title_images ti
                 WHERE ti.title_id = titles.id
                   AND ti.kind = {{}}
                   AND NULLIF(TRIM(ti.source_url), '') IS NOT NULL
                   AND TRIM(ti.source_url) NOT LIKE {{}}
            )"
    );
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &sql,
        &[
            SqlArg::Text(kind.as_str().to_string()),
            SqlArg::Text(local_title_image_route_pattern().to_string()),
            SqlArg::Text(local_title_image_route_pattern().to_string()),
            SqlArg::Text(kind.as_str().to_string()),
            SqlArg::Text(local_title_image_route_pattern().to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn clear_unrecoverable_local_title_image_source_tx(
    tx: &mut SqlTx<'_>,
    kind: TitleImageKind,
) -> AppResult<()> {
    let (source_col, _) = title_image_source_columns(kind);
    let sql = format!(
        "UPDATE titles
            SET {source_col} = NULL,
                metadata_fetched_at = NULL,
                metadata_hydration_next_attempt_at = {{}},
                metadata_hydration_attempt_count = 0
          WHERE TRIM(COALESCE({source_col}, '')) LIKE {{}}"
    );
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &sql,
        &[
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Text(local_title_image_route_pattern().to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn upsert_title_image_source_result_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    result: &TitleImageSourceResult,
    event: Option<&NewDomainEvent>,
) -> AppResult<Option<DomainEvent>> {
    let now = Utc::now();
    let (remote_source_column, _) = title_image_source_columns(result.kind);
    let current_source_sql =
        format!("SELECT {remote_source_column} AS source_url FROM titles WHERE id = {{}}");
    let current_source = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &current_source_sql,
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?
    .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?
    .opt_text("source_url")?;
    let Some(current_source) = current_source else {
        return Ok(None);
    };
    let normalized_current_source = normalize_title_image_source_url(&current_source)?;
    let normalized_requested_source =
        normalize_title_image_source_url(&result.requested_source_url)?;
    let normalized_result_source = normalize_title_image_source_url(&result.source_url)?;
    if normalized_requested_source != normalized_result_source {
        return Err(AppError::Repository(
            "processed title image source does not match the requested source".to_string(),
        ));
    }
    if normalized_current_source != normalized_requested_source {
        return Ok(None);
    }

    let canonicalize_source_sql = format!(
        "UPDATE titles SET {remote_source_column} = {{}} WHERE id = {{}} AND {remote_source_column} = {{}} RETURNING id"
    );
    let canonicalized = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &canonicalize_source_sql,
        &[
            SqlArg::Text(normalized_result_source),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(current_source),
        ],
    )
    .await?;
    if canonicalized.is_none() {
        return Ok(None);
    }

    let existing = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id, source_url FROM title_images WHERE title_id = {} AND kind = {}",
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(result.kind.as_str().to_string()),
        ],
    )
    .await?
    .map(|row| Ok::<_, AppError>((row.text("id")?, row.text("source_url")?)))
    .transpose()?;
    let source_changed = existing
        .as_ref()
        .is_some_and(|(_, source_url)| source_url != &result.source_url);
    let image_id = match tx {
        SqlTx::Sqlite(_) => upsert_title_image_sqlite_tx(tx, title_id, result, now).await?,
        SqlTx::Postgres(_) => upsert_title_image_postgres_tx(tx, title_id, result, now).await?,
    };

    if source_changed {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "DELETE FROM title_image_variants WHERE title_image_id = {}",
            &[SqlArg::Text(image_id.clone())],
        )
        .await?;
    }

    for variant in &result.variants {
        upsert_title_image_blob_tx(tx, variant, now).await?;
        upsert_title_image_variant_tx(tx, &image_id, variant, now).await?;
    }

    if let Some(local_path) = preferred_local_variant_path(title_id, result.kind, &result.variants)
    {
        update_title_local_image_path_tx(tx, title_id, result.kind, local_path).await?;
    }

    if let Some(event) = event {
        append_domain_event_tx(tx, event).await.map(Some)
    } else {
        Ok(None)
    }
}

fn preferred_local_variant_path(
    title_id: &str,
    kind: TitleImageKind,
    variants: &[TitleImageVariantRecord],
) -> Option<String> {
    let preferred_key = match kind {
        TitleImageKind::Poster => "w250",
        TitleImageKind::Fanart => "w1280",
    };
    variants
        .iter()
        .find(|variant| variant.variant_key == preferred_key)
        .map(|variant| {
            super::synthesize_local_title_image_url(
                "",
                title_id,
                kind,
                &variant.variant_key,
                &variant.digest,
            )
        })
}

async fn upsert_title_image_blob_tx(
    tx: &mut SqlTx<'_>,
    variant: &TitleImageVariantRecord,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let computed_digest = title_image_blob_digest(&variant.bytes);
    if variant.digest != computed_digest {
        return Err(AppError::Repository(format!(
            "title image variant {} supplied digest {} but bytes hash to {}",
            variant.variant_key, variant.digest, computed_digest
        )));
    }

    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO title_image_blobs (
            digest, format, width, height, bytes, created_at, updated_at
         ) VALUES ({}, {}, {}, {}, {}, {}, {})
         ON CONFLICT (digest) DO NOTHING",
        &[
            SqlArg::Text(variant.digest.clone()),
            SqlArg::Text(variant.format.clone()),
            SqlArg::I32(variant.width),
            SqlArg::I32(variant.height),
            SqlArg::OptBytes(Some(variant.bytes.clone())),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;

    let stored = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT format, width, height, bytes FROM title_image_blobs WHERE digest = {}",
        &[SqlArg::Text(variant.digest.clone())],
    )
    .await?
    .ok_or_else(|| {
        AppError::Repository(format!(
            "title image blob {} was not persisted",
            variant.digest
        ))
    })?;
    let stored_bytes = stored
        .opt_bytes("bytes")?
        .ok_or_else(|| AppError::Repository("title image blob missing bytes".into()))?;
    if stored.text("format")? != variant.format
        || stored.i32("width")? != variant.width
        || stored.i32("height")? != variant.height
        || stored_bytes != variant.bytes
    {
        return Err(AppError::Repository(format!(
            "title image blob {} conflicts with existing content",
            variant.digest
        )));
    }
    Ok(())
}

async fn upsert_title_image_variant_tx(
    tx: &mut SqlTx<'_>,
    image_id: &str,
    variant: &TitleImageVariantRecord,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let variant_id = Id::new().0;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO title_image_variants (
            id, title_image_id, variant_key, blob_digest, created_at, updated_at
         ) VALUES ({}, {}, {}, {}, {}, {})
         ON CONFLICT (title_image_id, variant_key) DO UPDATE SET
            blob_digest = excluded.blob_digest,
            updated_at = excluded.updated_at",
        &[
            SqlArg::Text(variant_id),
            SqlArg::Text(image_id.to_string()),
            SqlArg::Text(variant.variant_key.clone()),
            SqlArg::Text(variant.digest.clone()),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    Ok(())
}

async fn update_title_local_image_path_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    kind: TitleImageKind,
    local_path: String,
) -> AppResult<()> {
    let local_path_column = match kind {
        TitleImageKind::Poster => "poster_local_path",
        TitleImageKind::Fanart => "background_local_path",
    };
    let update_title_sql = format!("UPDATE titles SET {local_path_column} = {{}} WHERE id = {{}}");
    let rows = SqlRuntime::execute(
        SqlExec::Tx(tx),
        &update_title_sql,
        &[SqlArg::Text(local_path), SqlArg::Text(title_id.to_string())],
    )
    .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("title {title_id}")));
    }
    Ok(())
}

async fn upsert_title_image_sqlite_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    result: &TitleImageSourceResult,
    now: chrono::DateTime<Utc>,
) -> AppResult<String> {
    let existing = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id FROM title_images WHERE title_id = {} AND kind = {}",
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(result.kind.as_str().to_string()),
        ],
    )
    .await?
    .map(|row| row.text("id"))
    .transpose()?;

    if let Some(image_id) = existing {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "UPDATE title_images SET
                source_url = {},
                source_etag = {},
                source_last_modified = {},
                source_format = {},
                source_width = {},
                source_height = {},
                updated_at = {}
             WHERE id = {}",
            &title_image_update_args(&image_id, result, now),
        )
        .await?;
        Ok(image_id)
    } else {
        let image_id = Id::new().0;
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO title_images (
                id, title_id, provider, provider_image_id, kind, source_url, source_etag,
                source_last_modified, source_format, source_width, source_height,
                created_at, updated_at
            ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            &title_image_insert_args(&image_id, title_id, result, now),
        )
        .await?;
        Ok(image_id)
    }
}

async fn upsert_title_image_postgres_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    result: &TitleImageSourceResult,
    now: chrono::DateTime<Utc>,
) -> AppResult<String> {
    let image_id = Id::new().0;
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "INSERT INTO title_images (
            id, title_id, provider, provider_image_id, kind, source_url, source_etag,
            source_last_modified, source_format, source_width, source_height,
            created_at, updated_at
         ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT (title_id, kind) DO UPDATE SET
            source_url = excluded.source_url,
            source_etag = excluded.source_etag,
            source_last_modified = excluded.source_last_modified,
            source_format = excluded.source_format,
            source_width = excluded.source_width,
            source_height = excluded.source_height,
            updated_at = excluded.updated_at
         RETURNING id",
        &title_image_insert_args(&image_id, title_id, result, now),
    )
    .await?
    .ok_or_else(|| AppError::Repository("failed to upsert title image".into()))?;
    row.text("id")
}

fn title_image_insert_args(
    image_id: &str,
    title_id: &str,
    result: &TitleImageSourceResult,
    now: chrono::DateTime<Utc>,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(image_id.to_string()),
        SqlArg::Text(title_id.to_string()),
        SqlArg::Text("tvdb".to_string()),
        SqlArg::OptText(None),
        SqlArg::Text(result.kind.as_str().to_string()),
        SqlArg::Text(result.source_url.clone()),
        SqlArg::OptText(result.source_etag.clone()),
        SqlArg::OptText(result.source_last_modified.clone()),
        SqlArg::Text(result.source_format.clone()),
        SqlArg::I32(result.source_width),
        SqlArg::I32(result.source_height),
        SqlArg::Timestamp(now),
        SqlArg::Timestamp(now),
    ]
}

fn title_image_update_args(
    image_id: &str,
    result: &TitleImageSourceResult,
    now: chrono::DateTime<Utc>,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(result.source_url.clone()),
        SqlArg::OptText(result.source_etag.clone()),
        SqlArg::OptText(result.source_last_modified.clone()),
        SqlArg::Text(result.source_format.clone()),
        SqlArg::I32(result.source_width),
        SqlArg::I32(result.source_height),
        SqlArg::Timestamp(now),
        SqlArg::Text(image_id.to_string()),
    ]
}

async fn fetch_title_image_blob(
    exec: SqlExec<'_, '_>,
    title_id: &str,
    kind: TitleImageKind,
    variant_key: &str,
) -> AppResult<Option<TitleImageBlob>> {
    fetch_optional_blob(
        exec,
        "SELECT tib.format, tib.digest, tib.bytes
         FROM title_image_variants tiv
         INNER JOIN title_images ti ON ti.id = tiv.title_image_id
         INNER JOIN title_image_blobs tib ON tib.digest = tiv.blob_digest
         WHERE ti.title_id = {} AND ti.kind = {} AND tiv.variant_key = {}",
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(kind.as_str().to_string()),
            SqlArg::Text(variant_key.to_string()),
        ],
    )
    .await
}

async fn fetch_optional_blob(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<TitleImageBlob>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| {
            Ok(TitleImageBlob {
                content_type: crate::title_images::content_type_for_format(row.text("format")?),
                etag: row.text("digest")?,
                bytes: row
                    .opt_bytes("bytes")?
                    .ok_or_else(|| AppError::Repository("title image blob missing bytes".into()))?,
            })
        })
        .transpose()
}

async fn append_domain_event_tx(
    tx: &mut SqlTx<'_>,
    event: &NewDomainEvent,
) -> AppResult<DomainEvent> {
    let payload = serde_json::to_value(&event.payload).map_err(repo_err)?;
    let event_type = event.payload.event_type().as_str();
    let encoded_payload = encode_domain_event_payload(&payload).map_err(|error| {
        AppError::Repository(format!(
            "failed to encode domain event {} ({event_type}): {error}",
            event.event_id
        ))
    })?;
    let projections = derive_domain_event_projections(event_type, &payload);
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO domain_events (
            event_id, occurred_at, actor_kind, actor_user_id, actor_display_name,
            title_id, facet, correlation_id, causation_id, schema_version,
            stream_kind, stream_id, event_type, payload_json, import_status,
            media_file_delete_reason, download_id
         ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(event.event_id.clone()),
            SqlArg::Timestamp(event.occurred_at),
            SqlArg::Text(event.actor_kind.as_str().to_string()),
            SqlArg::OptText(event.actor_user_id.clone()),
            SqlArg::Text(event.actor_display_name.clone()),
            SqlArg::OptText(event.title_id.clone()),
            SqlArg::OptText(event.facet.as_ref().map(|facet| facet.as_str().to_string())),
            SqlArg::OptText(event.correlation_id.clone()),
            SqlArg::OptText(event.causation_id.clone()),
            SqlArg::I32(event.schema_version),
            SqlArg::Text(event.stream.kind().to_string()),
            SqlArg::OptText(event.stream.identifier().map(str::to_string)),
            SqlArg::Text(event_type.to_string()),
            SqlArg::OptBytes(Some(encoded_payload)),
            SqlArg::OptText(projections.import_status),
            SqlArg::OptText(projections.media_file_delete_reason),
            SqlArg::OptText(projections.download_id),
        ],
    )
    .await?;
    fetch_domain_event_by_event_id(SqlExec::Tx(tx), &event.event_id)
        .await?
        .ok_or_else(|| AppError::Repository("failed to reload inserted domain event".into()))
}

async fn fetch_domain_event_by_event_id(
    exec: SqlExec<'_, '_>,
    event_id: &str,
) -> AppResult<Option<DomainEvent>> {
    SqlRuntime::fetch_optional(
        exec,
        &format!("SELECT {DOMAIN_EVENT_COLUMNS} FROM domain_events WHERE event_id = {{}}"),
        &[SqlArg::Text(event_id.to_string())],
    )
    .await?
    .map(|row| domain_event_from_row(&row))
    .transpose()
}

fn domain_event_from_row(row: &SqlRow) -> AppResult<DomainEvent> {
    let event_id = row.text("event_id")?;
    let event_type = row.text("event_type")?;
    let stream_kind = row.text("stream_kind")?;
    let encoded_payload = row.opt_bytes("payload_json")?.ok_or_else(|| {
        AppError::Repository(format!(
            "domain event {event_id} ({event_type}) payload is null"
        ))
    })?;
    let decoded_payload = decode_domain_event_payload(&encoded_payload).map_err(|error| {
        AppError::Repository(format!(
            "failed to decode domain event {event_id} ({event_type}): {error}"
        ))
    })?;
    let payload = serde_json::from_value(decoded_payload).map_err(|error| {
        AppError::Repository(format!(
            "failed to deserialize domain event {event_id} ({event_type}): {error}"
        ))
    })?;
    Ok(DomainEvent {
        sequence: row.i64("sequence")?,
        event_id,
        occurred_at: row.timestamp("occurred_at")?,
        actor_kind: DomainEventActorKind::parse(row.text("actor_kind")?.as_str())
            .unwrap_or(DomainEventActorKind::System),
        actor_user_id: row.opt_text("actor_user_id")?,
        actor_display_name: row.text("actor_display_name")?,
        title_id: row.opt_text("title_id")?,
        facet: row
            .opt_text("facet")?
            .as_deref()
            .and_then(MediaFacet::parse),
        correlation_id: row.opt_text("correlation_id")?,
        causation_id: row.opt_text("causation_id")?,
        schema_version: row.i32("schema_version")?,
        stream: stream_from_parts(&stream_kind, row.opt_text("stream_id")?)?,
        payload,
    })
}

fn stream_from_parts(kind: &str, identifier: Option<String>) -> AppResult<DomainEventStream> {
    match kind {
        "global" => Ok(DomainEventStream::Global),
        "title" => identifier
            .map(|title_id| DomainEventStream::Title { title_id })
            .ok_or_else(|| AppError::Repository("domain event missing title stream id".into())),
        "library_scan" => identifier
            .map(|session_id| DomainEventStream::LibraryScan { session_id })
            .ok_or_else(|| {
                AppError::Repository("domain event missing library scan stream id".into())
            }),
        "job_run" => identifier
            .map(|run_id| DomainEventStream::JobRun { run_id })
            .ok_or_else(|| AppError::Repository("domain event missing job run stream id".into())),
        "download_queue_item" => identifier
            .map(|item_id| DomainEventStream::DownloadQueueItem { item_id })
            .ok_or_else(|| {
                AppError::Repository("domain event missing download queue item stream id".into())
            }),
        other => Err(AppError::Repository(format!(
            "unknown domain event stream kind: {other}"
        ))),
    }
}
