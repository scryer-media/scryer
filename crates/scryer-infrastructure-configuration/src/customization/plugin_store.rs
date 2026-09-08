use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, PluginInstallationRepository,
    persisted_records::external_plugin_installation_shape_is_supported,
};
use scryer_domain::{
    Id, PersistedPluginWasmPayload, PluginCatalogSource, PluginCatalogStatusRecord,
    PluginInstallation, PluginSourceKind, PluginSupportTier, PluginWasmEncoding,
};
use sqlx::Row;

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};
use crate::storage::sql::json::opt_json_text;

#[derive(Clone)]
pub struct PluginStore {
    datastore: StoreDatastore,
}

#[derive(Clone, Copy)]
struct BuiltinPluginSeed<'a> {
    plugin_id: &'a str,
    name: &'a str,
    description: &'a str,
    version: &'a str,
    sdk_version: &'a str,
    sdk_constraint: &'a str,
    plugin_type: &'a str,
    provider_type: &'a str,
}

impl PluginStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    pub async fn delete_incompatible_external_plugin_installations(
        &self,
        preserve_restored_recovery_targets: bool,
    ) -> AppResult<Vec<String>> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_incompatible_external_plugin_installations",
            |tx| {
                Box::pin(async move {
                    let sql = format!(
                        "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes
                           FROM plugin_installations
                          WHERE is_builtin = {{}} AND source_kind IN ('downloaded', 'manual')"
                    );
                    let rows = SqlRuntime::fetch_all(SqlExec::Tx(tx), &sql, &[SqlArg::Bool(false)])
                        .await?;
                    let removed_plugin_ids = rows
                        .iter()
                        .filter(|row| {
                            row_is_incompatible_external_installation(
                                row,
                                preserve_restored_recovery_targets,
                            )
                        })
                        .map(|row| row.text("plugin_id"))
                        .collect::<AppResult<Vec<_>>>()?;

                    for plugin_id in &removed_plugin_ids {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "DELETE FROM plugin_installations WHERE plugin_id = {}",
                            &[SqlArg::Text(plugin_id.clone())],
                        )
                        .await?;
                    }

                    Ok(removed_plugin_ids)
                })
            },
        )
        .await
    }

    async fn insert_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let installation = installation.clone();
        let wasm_bytes = wasm_bytes.map(<[u8]>::to_vec);
        SqlRuntime::run_in_transaction(&self.datastore, "create_plugin_installation", move |tx| {
            let installation = installation.clone();
            let wasm_bytes = wasm_bytes.clone();
            Box::pin(async move {
                if existing_installation_is_incompatible_external_shape(tx, &installation.plugin_id)
                    .await?
                {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM plugin_installations WHERE plugin_id = {}",
                        &[SqlArg::Text(installation.plugin_id.clone())],
                    )
                    .await?;
                }

                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    PLUGIN_INSTALLATION_INSERT_SQL,
                    &plugin_insert_args(&installation, wasm_bytes),
                )
                .await?;

                read_plugin_installation_tx(tx, &installation.plugin_id, false)
                    .await?
                    .ok_or_else(|| {
                        AppError::Repository(
                            "failed to read back created plugin installation".to_string(),
                        )
                    })
            })
        })
        .await
    }

    async fn persist_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let installation = installation.clone();
        let wasm_bytes = wasm_bytes.map(<[u8]>::to_vec);
        SqlRuntime::run_in_transaction(&self.datastore, "update_plugin_installation", move |tx| {
            let installation = installation.clone();
            let wasm_bytes = wasm_bytes.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    PLUGIN_INSTALLATION_UPDATE_SQL,
                    &plugin_update_args(&installation, wasm_bytes),
                )
                .await?;

                read_plugin_installation_tx(tx, &installation.plugin_id, false)
                    .await?
                    .ok_or_else(|| {
                        AppError::Repository(
                            "failed to read back updated plugin installation".to_string(),
                        )
                    })
            })
        })
        .await
    }

    async fn seed_builtin_plugin(&self, seed: BuiltinPluginSeed<'_>) -> AppResult<()> {
        let now = Utc::now();
        let plugin_id = seed.plugin_id.to_string();
        let insert_args = vec![
            SqlArg::Text(Id::new().0),
            SqlArg::Text(plugin_id.clone()),
            SqlArg::Text(seed.name.to_string()),
            SqlArg::Text(seed.description.to_string()),
            SqlArg::Text(seed.version.to_string()),
            SqlArg::Text(seed.sdk_version.to_string()),
            SqlArg::Text(seed.sdk_constraint.to_string()),
            SqlArg::Text(seed.plugin_type.to_string()),
            SqlArg::Text(seed.provider_type.to_string()),
            SqlArg::Bool(true),
            SqlArg::Bool(true),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ];
        let update_args = vec![
            SqlArg::Text(seed.name.to_string()),
            SqlArg::Text(seed.description.to_string()),
            SqlArg::Text(seed.version.to_string()),
            SqlArg::Text(seed.sdk_version.to_string()),
            SqlArg::Text(seed.sdk_constraint.to_string()),
            SqlArg::Text(seed.plugin_type.to_string()),
            SqlArg::Text(seed.provider_type.to_string()),
            SqlArg::Timestamp(now),
            SqlArg::Text(plugin_id.clone()),
            SqlArg::Bool(true),
        ];

        SqlRuntime::run_in_transaction(&self.datastore, "seed_builtin_plugin", move |tx| {
            let plugin_id = plugin_id.clone();
            let insert_args = insert_args.clone();
            let update_args = update_args.clone();
            Box::pin(async move {
                let existing = read_plugin_installation_tx(tx, &plugin_id, false).await?;
                match existing {
                    None => {
                        SqlRuntime::execute(SqlExec::Tx(tx), PLUGIN_SEED_INSERT_SQL, &insert_args)
                            .await?;
                    }
                    Some(existing) if existing.is_builtin => {
                        SqlRuntime::execute(SqlExec::Tx(tx), PLUGIN_SEED_UPDATE_SQL, &update_args)
                            .await?;
                    }
                    Some(_) => {}
                }
                Ok(())
            })
        })
        .await
    }
}

#[async_trait]
impl PluginInstallationRepository for PluginStore {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        let sql = format!(
            "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes IS NOT NULL AS wasm_bytes_present
               FROM plugin_installations
              ORDER BY is_builtin DESC, name ASC, plugin_id ASC"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &[])
            .await?
            .iter()
            .map(row_to_plugin_installation)
            .collect()
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        let sql = format!(
            "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes
               FROM plugin_installations
              WHERE plugin_id = {{}}"
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(plugin_id.to_string())],
        )
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        // Startup compatibility repair and the operator UI must see legacy
        // rows, including incomplete payload metadata. Runtime loading applies
        // compatibility checks separately; hiding rows prevents their repair.
        row_to_plugin_installation(&row).map(Some)
    }

    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        self.insert_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        self.persist_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_plugin_installation",
            "DELETE FROM plugin_installations WHERE plugin_id = {}",
            vec![SqlArg::Text(plugin_id.to_string())],
        )
        .await
    }

    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>> {
        let sql = format!(
            "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes
               FROM plugin_installations
              WHERE is_enabled = {{}}
              ORDER BY is_builtin DESC, name ASC, plugin_id ASC"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &[SqlArg::Bool(true)])
            .await?
            .iter()
            .map(|row| {
                let installation = row_to_plugin_installation(row)?;
                let payload =
                    row.opt_bytes("wasm_bytes")?
                        .map(|bytes| PersistedPluginWasmPayload {
                            encoding: installation.wasm_encoding,
                            bytes,
                        });
                Ok((installation, payload))
            })
            .collect()
    }

    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PersistedPluginWasmPayload>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT wasm_bytes, wasm_encoding FROM plugin_installations WHERE plugin_id = {}",
            &[SqlArg::Text(plugin_id.to_string())],
        )
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let Some(bytes) = row.opt_bytes("wasm_bytes")? else {
            return Ok(None);
        };
        Ok(Some(PersistedPluginWasmPayload {
            encoding: parse_wasm_encoding(&row.text("wasm_encoding")?),
            bytes,
        }))
    }

    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        sdk_version: &str,
        sdk_constraint: &str,
        plugin_type: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        let seed = BuiltinPluginSeed {
            plugin_id,
            name,
            description,
            version,
            sdk_version,
            sdk_constraint,
            plugin_type,
            provider_type,
        };
        self.seed_builtin_plugin(seed).await
    }

    async fn upsert_plugin_catalog_source(&self, source: &PluginCatalogSource) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "upsert_plugin_catalog_source",
            "INSERT INTO plugin_catalog_sources
                (source_key, source_kind, source_url, github_repo, support_tier, catalog_json,
                 last_success_at, last_error, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})
             ON CONFLICT (source_key) DO UPDATE SET
                source_kind = excluded.source_kind,
                source_url = excluded.source_url,
                github_repo = excluded.github_repo,
                support_tier = excluded.support_tier,
                catalog_json = excluded.catalog_json,
                last_success_at = excluded.last_success_at,
                last_error = excluded.last_error,
                updated_at = excluded.updated_at",
            vec![
                SqlArg::Text(source.source_key.clone()),
                SqlArg::Text(source.source_kind.clone()),
                SqlArg::Text(source.source_url.clone()),
                SqlArg::OptText(source.github_repo.clone()),
                SqlArg::Text(support_tier_label(source.support_tier).to_string()),
                SqlArg::OptText(source.catalog_json.clone()),
                SqlArg::OptTimestamp(source.last_success_at),
                SqlArg::OptText(source.last_error.clone()),
                SqlArg::Timestamp(source.updated_at),
            ],
        )
        .await
    }

    async fn delete_plugin_catalog_source(&self, source_key: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_plugin_catalog_source",
            "DELETE FROM plugin_catalog_sources WHERE source_key = {}",
            vec![SqlArg::Text(source_key.to_string())],
        )
        .await
    }

    async fn list_plugin_catalog_sources(&self) -> AppResult<Vec<PluginCatalogSource>> {
        let sql = format!(
            "SELECT {PLUGIN_CATALOG_SOURCE_COLUMNS}
               FROM plugin_catalog_sources
              ORDER BY source_kind ASC, source_key ASC"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &[])
            .await?
            .iter()
            .map(row_to_plugin_catalog_source)
            .collect()
    }

    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<PluginCatalogSource>> {
        let sql = format!(
            "SELECT {PLUGIN_CATALOG_SOURCE_COLUMNS}
               FROM plugin_catalog_sources
              WHERE source_key = {{}}"
        );
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(source_key.to_string())],
        )
        .await?
        .as_ref()
        .map(row_to_plugin_catalog_source)
        .transpose()
    }

    async fn upsert_plugin_catalog_status(
        &self,
        status: &PluginCatalogStatusRecord,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "upsert_plugin_catalog_status",
            "INSERT INTO plugin_catalog_status (status_key, status_json, checked_at)
             VALUES ({}, {}, {})
             ON CONFLICT (status_key) DO UPDATE SET
                status_json = excluded.status_json,
                checked_at = excluded.checked_at",
            vec![
                SqlArg::Text(status.status_key.clone()),
                SqlArg::Text(status.status_json.clone()),
                SqlArg::Timestamp(status.checked_at),
            ],
        )
        .await
    }

    async fn get_plugin_catalog_status(
        &self,
        status_key: &str,
    ) -> AppResult<Option<PluginCatalogStatusRecord>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT status_key, status_json, checked_at
               FROM plugin_catalog_status
              WHERE status_key = {}",
            &[SqlArg::Text(status_key.to_string())],
        )
        .await?
        .as_ref()
        .map(row_to_plugin_catalog_status)
        .transpose()
    }
}

const PLUGIN_INSTALLATION_COLUMNS: &str = "id, plugin_id, name, description, version, sdk_version,
    sdk_constraint, scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
    source_kind, wasm_encoding, wasm_digest_algo, source_url, support_tier, publisher,
    docs_url, source_repo, manifest_url, wasm_digest, artifact_digest, descriptor_json,
    installed_at, updated_at";

const PLUGIN_CATALOG_SOURCE_COLUMNS: &str = "source_key, source_kind, source_url, github_repo,
    support_tier, catalog_json, last_success_at, last_error, updated_at";

const PLUGIN_INSTALLATION_INSERT_SQL: &str = "INSERT INTO plugin_installations
    (id, plugin_id, name, description, version, sdk_version, sdk_constraint,
     scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
     source_kind, wasm_bytes, wasm_encoding, wasm_digest_algo, source_url, support_tier, publisher,
     docs_url, source_repo, manifest_url, wasm_digest, artifact_digest, descriptor_json,
     installed_at, updated_at)
 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

const PLUGIN_INSTALLATION_UPDATE_SQL: &str = "UPDATE plugin_installations
   SET name = {}, description = {}, version = {}, sdk_version = {}, sdk_constraint = {},
       scryer_constraint = {}, plugin_type = {}, provider_type = {}, is_enabled = {},
       is_builtin = {}, source_kind = {},
       wasm_bytes = CASE WHEN {} = 'bundled' THEN NULL ELSE COALESCE({}, wasm_bytes) END,
       wasm_encoding = CASE WHEN {} = 'bundled' THEN 'identity' ELSE COALESCE({}, wasm_encoding) END,
       wasm_digest_algo = CASE WHEN {} = 'bundled' THEN NULL ELSE COALESCE({}, wasm_digest_algo) END,
       source_url = CASE WHEN {} = 'bundled' THEN NULL ELSE COALESCE({}, source_url) END,
       support_tier = {},
       publisher = CASE WHEN {} = 'bundled' THEN NULL ELSE {} END,
       docs_url = CASE WHEN {} = 'bundled' THEN NULL ELSE {} END,
       source_repo = CASE WHEN {} = 'bundled' THEN NULL ELSE {} END,
       manifest_url = CASE WHEN {} = 'bundled' THEN NULL ELSE COALESCE({}, manifest_url) END,
       wasm_digest = CASE WHEN {} = 'bundled' THEN NULL ELSE COALESCE({}, wasm_digest) END,
       artifact_digest = CASE WHEN {} = 'bundled' THEN NULL ELSE COALESCE({}, artifact_digest) END,
       descriptor_json = CASE WHEN {} = 'bundled' THEN NULL ELSE COALESCE({}, descriptor_json) END,
       updated_at = {}
 WHERE plugin_id = {}";

const PLUGIN_SEED_INSERT_SQL: &str = "INSERT INTO plugin_installations
    (id, plugin_id, name, description, version, sdk_version, sdk_constraint,
     scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
     source_kind, installed_at, updated_at)
 VALUES ({}, {}, {}, {}, {}, {}, {}, NULL, {}, {}, {}, {}, 'bundled', {}, {})";

const PLUGIN_SEED_UPDATE_SQL: &str = "UPDATE plugin_installations
   SET name = CASE WHEN source_kind = 'downloaded' THEN name ELSE {} END,
       description = CASE WHEN source_kind = 'downloaded' THEN description ELSE {} END,
       version = CASE WHEN source_kind = 'downloaded' THEN version ELSE {} END,
       sdk_version = CASE WHEN source_kind = 'downloaded' THEN sdk_version ELSE {} END,
       sdk_constraint = CASE WHEN source_kind = 'downloaded' THEN sdk_constraint ELSE {} END,
       scryer_constraint = CASE WHEN source_kind = 'downloaded' THEN scryer_constraint ELSE NULL END,
       plugin_type = {},
       provider_type = {},
       source_kind = CASE WHEN source_kind = 'downloaded' THEN source_kind ELSE 'bundled' END,
       updated_at = {}
 WHERE plugin_id = {} AND is_builtin = {}";

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: &'static str,
    args: Vec<SqlArg>,
) -> AppResult<()> {
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let args = args.clone();
        Box::pin(async move {
            SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await?;
            Ok(())
        })
    })
    .await
}

async fn existing_installation_is_incompatible_external_shape(
    tx: &mut SqlTx<'_>,
    plugin_id: &str,
) -> AppResult<bool> {
    let sql = format!(
        "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes
           FROM plugin_installations
          WHERE plugin_id = {{}}"
    );
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &sql,
        &[SqlArg::Text(plugin_id.to_string())],
    )
    .await?;

    Ok(row
        .as_ref()
        .is_some_and(|row| row_is_incompatible_external_installation(row, false)))
}

async fn read_plugin_installation_tx(
    tx: &mut SqlTx<'_>,
    plugin_id: &str,
    filter_incompatible: bool,
) -> AppResult<Option<PluginInstallation>> {
    let sql = format!(
        "SELECT {PLUGIN_INSTALLATION_COLUMNS}, wasm_bytes
           FROM plugin_installations
          WHERE plugin_id = {{}}"
    );
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &sql,
        &[SqlArg::Text(plugin_id.to_string())],
    )
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    if filter_incompatible && row_is_incompatible_external_installation(&row, true) {
        return Ok(None);
    }
    row_to_plugin_installation(&row).map(Some)
}

fn plugin_insert_args(
    installation: &PluginInstallation,
    wasm_bytes: Option<Vec<u8>>,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(installation.id.clone()),
        SqlArg::Text(installation.plugin_id.clone()),
        SqlArg::Text(installation.name.clone()),
        SqlArg::Text(installation.description.clone()),
        SqlArg::Text(installation.version.clone()),
        SqlArg::Text(installation.sdk_version.clone()),
        SqlArg::Text(installation.sdk_constraint.clone()),
        SqlArg::OptText(installation.scryer_constraint.clone()),
        SqlArg::Text(installation.plugin_type.clone()),
        SqlArg::Text(installation.provider_type.clone()),
        SqlArg::Bool(installation.is_enabled),
        SqlArg::Bool(installation.is_builtin),
        SqlArg::Text(source_kind_label(installation.source_kind).to_string()),
        SqlArg::OptBytes(wasm_bytes),
        SqlArg::Text(wasm_encoding_label(installation.wasm_encoding).to_string()),
        SqlArg::OptText(installation.wasm_digest_algo.clone()),
        SqlArg::OptText(installation.source_url.clone()),
        SqlArg::Text(support_tier_label(installation.support_tier).to_string()),
        SqlArg::OptText(installation.publisher.clone()),
        SqlArg::OptText(installation.docs_url.clone()),
        SqlArg::OptText(installation.source_repo.clone()),
        SqlArg::OptText(installation.manifest_url.clone()),
        SqlArg::OptText(installation.wasm_digest.clone()),
        SqlArg::OptText(installation.artifact_digest.clone()),
        SqlArg::OptText(installation.descriptor_json.clone()),
        SqlArg::Timestamp(installation.installed_at),
        SqlArg::Timestamp(installation.updated_at),
    ]
}

fn plugin_update_args(
    installation: &PluginInstallation,
    wasm_bytes: Option<Vec<u8>>,
) -> Vec<SqlArg> {
    let source_kind = source_kind_label(installation.source_kind).to_string();
    vec![
        SqlArg::Text(installation.name.clone()),
        SqlArg::Text(installation.description.clone()),
        SqlArg::Text(installation.version.clone()),
        SqlArg::Text(installation.sdk_version.clone()),
        SqlArg::Text(installation.sdk_constraint.clone()),
        SqlArg::OptText(installation.scryer_constraint.clone()),
        SqlArg::Text(installation.plugin_type.clone()),
        SqlArg::Text(installation.provider_type.clone()),
        SqlArg::Bool(installation.is_enabled),
        SqlArg::Bool(installation.is_builtin),
        SqlArg::Text(source_kind.clone()),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptBytes(wasm_bytes),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptText(Some(
            wasm_encoding_label(installation.wasm_encoding).to_string(),
        )),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptText(installation.wasm_digest_algo.clone()),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptText(installation.source_url.clone()),
        SqlArg::Text(support_tier_label(installation.support_tier).to_string()),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptText(installation.publisher.clone()),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptText(installation.docs_url.clone()),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptText(installation.source_repo.clone()),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptText(installation.manifest_url.clone()),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptText(installation.wasm_digest.clone()),
        SqlArg::Text(source_kind.clone()),
        SqlArg::OptText(installation.artifact_digest.clone()),
        SqlArg::Text(source_kind),
        SqlArg::OptText(installation.descriptor_json.clone()),
        SqlArg::Timestamp(installation.updated_at),
        SqlArg::Text(installation.plugin_id.clone()),
    ]
}

fn row_to_plugin_installation(row: &SqlRow) -> AppResult<PluginInstallation> {
    Ok(PluginInstallation {
        id: row.text("id")?,
        plugin_id: row.text("plugin_id")?,
        name: row.text("name")?,
        description: row.text("description")?,
        version: row.text("version")?,
        sdk_version: row.text("sdk_version")?,
        sdk_constraint: row.text("sdk_constraint")?,
        scryer_constraint: row.opt_text("scryer_constraint")?,
        plugin_type: row.text("plugin_type")?,
        provider_type: row.text("provider_type")?,
        is_enabled: row.bool("is_enabled")?,
        is_builtin: row.bool("is_builtin")?,
        source_kind: parse_source_kind(&row.text("source_kind")?),
        wasm_encoding: parse_wasm_encoding(&row.text("wasm_encoding")?),
        wasm_digest_algo: row.opt_text("wasm_digest_algo")?,
        source_url: row.opt_text("source_url")?,
        support_tier: parse_support_tier(&row.text("support_tier")?),
        publisher: row.opt_text("publisher")?,
        docs_url: row.opt_text("docs_url")?,
        source_repo: row.opt_text("source_repo")?,
        manifest_url: row.opt_text("manifest_url")?,
        wasm_digest: row.opt_text("wasm_digest")?,
        artifact_digest: row.opt_text("artifact_digest")?,
        descriptor_json: descriptor_json_text(row)?,
        installed_at: timestamp_or_now(row, "installed_at")?,
        updated_at: timestamp_or_now(row, "updated_at")?,
    })
}

fn row_to_plugin_catalog_source(row: &SqlRow) -> AppResult<PluginCatalogSource> {
    Ok(PluginCatalogSource {
        source_key: row.text("source_key")?,
        source_kind: row.text("source_kind")?,
        source_url: row.text("source_url")?,
        github_repo: row.opt_text("github_repo")?,
        support_tier: parse_support_tier(&row.text("support_tier")?),
        catalog_json: row.opt_text("catalog_json")?,
        last_success_at: optional_timestamp_or_none(row, "last_success_at")?,
        last_error: row.opt_text("last_error")?,
        updated_at: timestamp_or_now(row, "updated_at")?,
    })
}

fn row_to_plugin_catalog_status(row: &SqlRow) -> AppResult<PluginCatalogStatusRecord> {
    Ok(PluginCatalogStatusRecord {
        status_key: row.text("status_key")?,
        status_json: row.text("status_json")?,
        checked_at: timestamp_or_now(row, "checked_at")?,
    })
}

fn descriptor_json_text(row: &SqlRow) -> AppResult<Option<String>> {
    opt_json_text(row, "descriptor_json")
}

fn timestamp_or_now(row: &SqlRow, column: &str) -> AppResult<DateTime<Utc>> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: String = row.try_get(column).map_err(repo_err)?;
            Ok(DateTime::parse_from_rfc3339(&raw)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()))
        }
        SqlRow::Postgres(_) => row.timestamp(column),
    }
}

fn optional_timestamp_or_none(row: &SqlRow, column: &str) -> AppResult<Option<DateTime<Utc>>> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.and_then(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            }))
        }
        SqlRow::Postgres(_) => row.opt_timestamp(column),
    }
}

fn row_is_incompatible_external_installation(
    row: &SqlRow,
    preserve_restored_recovery_targets: bool,
) -> bool {
    let is_builtin = row.bool("is_builtin").unwrap_or(false);
    if is_builtin {
        return false;
    }

    let source_kind = row
        .text("source_kind")
        .unwrap_or_else(|_| "downloaded".to_string());
    if !matches!(source_kind.as_str(), "downloaded" | "manual") {
        return false;
    }

    let wasm_bytes_present = row
        .bool("wasm_bytes_present")
        .unwrap_or_else(|_| row.opt_bytes("wasm_bytes").unwrap_or(None).is_some());
    let wasm_encoding = row
        .text("wasm_encoding")
        .unwrap_or_else(|_| "identity".to_string());
    let wasm_digest_algo = row.opt_text("wasm_digest_algo").unwrap_or(None);
    let wasm_digest = row.opt_text("wasm_digest").unwrap_or(None);
    let descriptor_supported = descriptor_json_text(row)
        .unwrap_or(None)
        .is_some_and(|value| !value.trim().is_empty());
    if preserve_restored_recovery_targets
        && !wasm_bytes_present
        && descriptor_supported
        && matches!(source_kind.as_str(), "downloaded" | "manual")
        && (source_kind == "downloaded"
            || row
                .opt_text("source_repo")
                .unwrap_or(None)
                .is_some_and(|value| !value.trim().is_empty()))
    {
        return false;
    }

    !external_plugin_installation_shape_is_supported(
        wasm_bytes_present,
        &wasm_encoding,
        wasm_digest_algo.as_deref(),
        wasm_digest.as_deref(),
        descriptor_supported,
    )
}

fn parse_source_kind(value: &str) -> PluginSourceKind {
    match value {
        "bundled" => PluginSourceKind::Bundled,
        "community" => PluginSourceKind::Community,
        "manual" => PluginSourceKind::Manual,
        _ => PluginSourceKind::Downloaded,
    }
}

fn source_kind_label(value: PluginSourceKind) -> &'static str {
    match value {
        PluginSourceKind::Bundled => "bundled",
        PluginSourceKind::Downloaded => "downloaded",
        PluginSourceKind::Community => "community",
        PluginSourceKind::Manual => "manual",
    }
}

fn parse_support_tier(value: &str) -> PluginSupportTier {
    match value {
        "verified_community" => PluginSupportTier::VerifiedCommunity,
        "unverified" => PluginSupportTier::Unverified,
        _ => PluginSupportTier::Official,
    }
}

fn support_tier_label(value: PluginSupportTier) -> &'static str {
    match value {
        PluginSupportTier::Official => "official",
        PluginSupportTier::VerifiedCommunity => "verified_community",
        PluginSupportTier::Unverified => "unverified",
    }
}

fn parse_wasm_encoding(value: &str) -> PluginWasmEncoding {
    match value {
        "brotli" => PluginWasmEncoding::Brotli,
        "zstd" => PluginWasmEncoding::Zstd,
        _ => PluginWasmEncoding::Identity,
    }
}

fn wasm_encoding_label(value: PluginWasmEncoding) -> &'static str {
    match value {
        PluginWasmEncoding::Identity => "identity",
        PluginWasmEncoding::Brotli => "brotli",
        PluginWasmEncoding::Zstd => "zstd",
    }
}
