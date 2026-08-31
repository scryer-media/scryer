//! 0179 — establish the first durable, canonical download identity.
//!
//! This hook deliberately works from the legacy submission/client tuple.  The
//! application still reads and writes those columns until the follow-up
//! migration makes the canonical relation authoritative.

use std::collections::{BTreeMap, HashMap, HashSet};

use scryer_application::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct ClientTuple {
    client_config_id: Option<String>,
    client_type: Option<String>,
    native_item_id: Option<String>,
}

#[derive(Clone, Debug)]
struct SubmissionRow {
    id: String,
    title_id: String,
    client: ClientTuple,
    client_name: Option<String>,
    download_id: Option<String>,
    submitted_at: Option<String>,
}

#[derive(Clone, Debug)]
struct IdentityStateRow {
    id: String,
    identity_key: String,
    client: ClientTuple,
    canonical_download_id: Option<String>,
}

#[derive(Clone, Debug)]
struct TupleDependentRow {
    id: String,
    client: ClientTuple,
    canonical_download_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum DependentTable {
    IdentityStates,
    Imports,
    ImportArtifacts,
    QueueCommands,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DependentRef {
    table: DependentTable,
    id: String,
}

#[derive(Clone, Debug)]
struct ForeignGroup {
    client: ClientTuple,
    members: Vec<DependentRef>,
}

#[derive(Clone, Debug)]
struct MigrationInput {
    client_names: HashMap<String, String>,
    submissions: Vec<SubmissionRow>,
    identity_states: Vec<IdentityStateRow>,
    imports: Vec<TupleDependentRow>,
    import_artifacts: Vec<TupleDependentRow>,
    queue_commands: Vec<TupleDependentRow>,
}

#[derive(Clone, Debug)]
struct DownloadPlan {
    id: String,
    origin: &'static str,
    created_at: String,
}

#[derive(Clone, Debug)]
struct BindingPlan {
    download_id: String,
    client: ClientTuple,
    client_name_snapshot: Option<String>,
    created_at: String,
    ended_at: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct BackfillPlan {
    submission_id_updates: Vec<(String, String)>,
    downloads: Vec<DownloadPlan>,
    bindings: Vec<BindingPlan>,
    dependent_updates: HashMap<DependentRef, String>,
}

pub async fn backfill_canonical_download_identity_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let input = load_sqlite_input(tx).await?;
    if sqlite_backfill_is_complete(tx, &input).await? {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339();
    let plan = build_backfill_plan(&input, &now)?;
    apply_sqlite_plan(tx, &plan).await?;
    verify_sqlite_backfill(tx).await
}

pub async fn backfill_canonical_download_identity_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let input = load_postgres_input(tx).await?;
    if postgres_backfill_is_complete(tx, &input).await? {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339();
    let plan = build_backfill_plan(&input, &now)?;
    apply_postgres_plan(tx, &plan).await?;
    verify_postgres_backfill(tx).await
}

fn build_backfill_plan(input: &MigrationInput, now: &str) -> AppResult<BackfillPlan> {
    let mut plan = BackfillPlan::default();
    let mut submission_by_tuple = HashMap::<ClientTuple, String>::new();
    let mut submission_download_ids = Vec::<(String, Option<String>)>::new();
    let mut used_canonical_uuids = HashMap::<String, String>::new();
    let mut used_download_ids = HashSet::<String>::new();

    for submission in &input.submissions {
        let canonical_id = canonical_submission_id(submission, &mut used_canonical_uuids)?;
        used_download_ids.insert(canonical_id.clone());

        if canonical_id != submission.id {
            plan.submission_id_updates
                .push((submission.id.clone(), canonical_id.clone()));
        }

        if submission_by_tuple
            .insert(submission.client.clone(), canonical_id.clone())
            .is_some()
        {
            return Err(AppError::Repository(format!(
                "duplicate download submission client tuple while backfilling row '{}'",
                submission.id
            )));
        }

        submission_download_ids.push((canonical_id.clone(), submission.download_id.clone()));
        plan.downloads.push(DownloadPlan {
            id: canonical_id.clone(),
            origin: if submission.title_id.trim().is_empty() {
                "foreign_observation"
            } else {
                "scryer_submission"
            },
            created_at: nonblank(&submission.submitted_at).unwrap_or_else(|| now.to_string()),
        });
        plan.bindings.push(binding_plan(
            canonical_id,
            submission.client.clone(),
            submission.client_name.clone(),
            now,
        ));
    }

    let mut foreign_by_tuple = BTreeMap::<ClientTuple, Vec<DependentRef>>::new();
    let mut one_per_global_state = Vec::<ForeignGroup>::new();

    for state in &input.identity_states {
        let reference = DependentRef {
            table: DependentTable::IdentityStates,
            id: state.id.clone(),
        };
        if state.client.client_config_id.is_none() && state.client.client_type.is_none() {
            if let Some(download_id) =
                match_global_identity_key(&state.identity_key, &submission_download_ids)
            {
                plan.dependent_updates.insert(reference, download_id);
            } else {
                one_per_global_state.push(ForeignGroup {
                    client: ClientTuple::default(),
                    members: vec![reference],
                });
            }
        } else {
            assign_tuple_dependent(
                reference,
                state.client.clone(),
                &submission_by_tuple,
                &mut plan.dependent_updates,
                &mut foreign_by_tuple,
            );
        }
    }

    collect_tuple_dependents(
        DependentTable::Imports,
        &input.imports,
        &submission_by_tuple,
        &mut plan.dependent_updates,
        &mut foreign_by_tuple,
    );
    collect_tuple_dependents(
        DependentTable::ImportArtifacts,
        &input.import_artifacts,
        &submission_by_tuple,
        &mut plan.dependent_updates,
        &mut foreign_by_tuple,
    );
    collect_tuple_dependents(
        DependentTable::QueueCommands,
        &input.queue_commands,
        &submission_by_tuple,
        &mut plan.dependent_updates,
        &mut foreign_by_tuple,
    );

    let foreign_groups = foreign_by_tuple
        .into_iter()
        .map(|(client, members)| ForeignGroup { client, members })
        .chain(one_per_global_state)
        .collect::<Vec<_>>();
    for group in foreign_groups {
        let download_id = next_unused_uuid(&mut used_download_ids);
        for member in group.members {
            plan.dependent_updates.insert(member, download_id.clone());
        }
        plan.downloads.push(DownloadPlan {
            id: download_id.clone(),
            origin: "foreign_observation",
            created_at: now.to_string(),
        });
        let client_name = group
            .client
            .client_config_id
            .as_ref()
            .and_then(|id| input.client_names.get(id))
            .cloned();
        plan.bindings
            .push(binding_plan(download_id, group.client, client_name, now));
    }

    Ok(plan)
}

fn canonical_submission_id(
    submission: &SubmissionRow,
    used_canonical_uuids: &mut HashMap<String, String>,
) -> AppResult<String> {
    let canonical_id = parse_scryer_download_token(submission.download_id.as_deref())
        .map(|uuid| uuid.to_string())
        .or_else(|| {
            Uuid::parse_str(&submission.id)
                .ok()
                .map(|_| submission.id.clone())
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let normalized_uuid = Uuid::parse_str(&canonical_id)
        .expect("canonical submission id must be a UUID")
        .to_string();

    if let Some(existing) = used_canonical_uuids.insert(normalized_uuid, submission.id.clone()) {
        return Err(AppError::Repository(format!(
            "canonical download identity collision between submission rows '{existing}' and '{}'",
            submission.id
        )));
    }
    Ok(canonical_id)
}

fn parse_scryer_download_token(value: Option<&str>) -> Option<Uuid> {
    value
        .map(str::trim)
        .and_then(|value| value.strip_prefix("scryer-download:"))
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn binding_plan(
    download_id: String,
    client: ClientTuple,
    client_name: Option<String>,
    now: &str,
) -> BindingPlan {
    let client_name_snapshot = client_name.or_else(|| client.client_type.clone());
    let ended_at = client.client_config_id.is_none().then(|| now.to_string());
    BindingPlan {
        download_id,
        client,
        client_name_snapshot,
        created_at: now.to_string(),
        ended_at,
    }
}

fn collect_tuple_dependents(
    table: DependentTable,
    rows: &[TupleDependentRow],
    submission_by_tuple: &HashMap<ClientTuple, String>,
    dependent_updates: &mut HashMap<DependentRef, String>,
    foreign_by_tuple: &mut BTreeMap<ClientTuple, Vec<DependentRef>>,
) {
    for row in rows {
        assign_tuple_dependent(
            DependentRef {
                table,
                id: row.id.clone(),
            },
            row.client.clone(),
            submission_by_tuple,
            dependent_updates,
            foreign_by_tuple,
        );
    }
}

fn assign_tuple_dependent(
    reference: DependentRef,
    client: ClientTuple,
    submission_by_tuple: &HashMap<ClientTuple, String>,
    dependent_updates: &mut HashMap<DependentRef, String>,
    foreign_by_tuple: &mut BTreeMap<ClientTuple, Vec<DependentRef>>,
) {
    if let Some(download_id) = submission_by_tuple.get(&client) {
        dependent_updates.insert(reference, download_id.clone());
    } else {
        foreign_by_tuple.entry(client).or_default().push(reference);
    }
}

fn match_global_identity_key(
    identity_key: &str,
    submission_download_ids: &[(String, Option<String>)],
) -> Option<String> {
    let legacy_value = identity_key.strip_prefix("download:")?.trim();
    submission_download_ids
        .iter()
        .find_map(|(canonical_id, download_id)| {
            (legacy_value == canonical_id
                || download_id
                    .as_deref()
                    .is_some_and(|download_id| download_id.trim() == legacy_value))
            .then(|| canonical_id.clone())
        })
}

fn next_unused_uuid(used_download_ids: &mut HashSet<String>) -> String {
    loop {
        let id = Uuid::new_v4().to_string();
        if used_download_ids.insert(id.clone()) {
            return id;
        }
    }
}

fn nonblank(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalized(value: Option<String>) -> Option<String> {
    nonblank(&value)
}

fn tuple_from_parts(
    client_config_id: Option<String>,
    client_type: Option<String>,
    native_item_id: Option<String>,
) -> ClientTuple {
    ClientTuple {
        client_config_id: normalized(client_config_id),
        client_type: normalized(client_type),
        native_item_id: normalized(native_item_id),
    }
}

async fn load_sqlite_input(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<MigrationInput> {
    let submission_rows = sqlx::query(
        "SELECT s.id, s.title_id, s.download_client_id, s.download_client_type,
                s.download_client_item_id, s.download_id, s.submitted_at, dc.name AS client_name
           FROM download_submissions s
           LEFT JOIN download_clients dc ON dc.id = s.download_client_id
          ORDER BY s.id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    let submissions = submission_rows
        .into_iter()
        .map(|row| {
            Ok(SubmissionRow {
                id: row.try_get("id").map_err(repo_err)?,
                title_id: row.try_get("title_id").map_err(repo_err)?,
                client: tuple_from_parts(
                    row.try_get("download_client_id").map_err(repo_err)?,
                    row.try_get("download_client_type").map_err(repo_err)?,
                    row.try_get("download_client_item_id").map_err(repo_err)?,
                ),
                client_name: row.try_get("client_name").map_err(repo_err)?,
                download_id: row.try_get("download_id").map_err(repo_err)?,
                submitted_at: row.try_get("submitted_at").map_err(repo_err)?,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;

    let client_names: HashMap<String, String> =
        sqlx::query_as("SELECT id, name FROM download_clients")
            .fetch_all(&mut **tx)
            .await
            .map_err(repo_err)?
            .into_iter()
            .collect();

    Ok(MigrationInput {
        client_names,
        submissions,
        identity_states: load_sqlite_identity_states(tx).await?,
        imports: load_sqlite_imports(tx).await?,
        import_artifacts: load_sqlite_import_artifacts(tx).await?,
        queue_commands: load_sqlite_queue_commands(tx).await?,
    })
}

async fn load_sqlite_identity_states(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<IdentityStateRow>> {
    sqlx::query(
        "SELECT id, identity_key, client_id, client_type, download_client_item_id,
                canonical_download_id
           FROM download_identity_states ORDER BY id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?
    .into_iter()
    .map(|row| {
        Ok(IdentityStateRow {
            id: row.try_get("id").map_err(repo_err)?,
            identity_key: row.try_get("identity_key").map_err(repo_err)?,
            client: tuple_from_parts(
                row.try_get("client_id").map_err(repo_err)?,
                row.try_get("client_type").map_err(repo_err)?,
                row.try_get("download_client_item_id").map_err(repo_err)?,
            ),
            canonical_download_id: row.try_get("canonical_download_id").map_err(repo_err)?,
        })
    })
    .collect()
}

async fn load_sqlite_imports(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<TupleDependentRow>> {
    load_sqlite_tuple_dependents(
        tx,
        "SELECT id, source_client_id, source_system, source_ref, canonical_download_id FROM imports ORDER BY id",
    )
    .await
}

async fn load_sqlite_import_artifacts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<TupleDependentRow>> {
    load_sqlite_tuple_dependents(
        tx,
        "SELECT id, source_client_id, source_system, source_ref, canonical_download_id FROM download_import_artifacts ORDER BY id",
    )
    .await
}

async fn load_sqlite_queue_commands(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<TupleDependentRow>> {
    sqlx::query(
        "SELECT id, client_id, client_type, download_client_item_id, canonical_download_id
           FROM download_queue_commands ORDER BY id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?
    .into_iter()
    .map(|row| {
        Ok(TupleDependentRow {
            id: row.try_get("id").map_err(repo_err)?,
            client: tuple_from_parts(
                row.try_get("client_id").map_err(repo_err)?,
                row.try_get("client_type").map_err(repo_err)?,
                row.try_get("download_client_item_id").map_err(repo_err)?,
            ),
            canonical_download_id: row.try_get("canonical_download_id").map_err(repo_err)?,
        })
    })
    .collect()
}

async fn load_sqlite_tuple_dependents(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    query: &'static str,
) -> AppResult<Vec<TupleDependentRow>> {
    sqlx::query(query)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?
        .into_iter()
        .filter_map(|row| {
            let source_system: Option<String> = row.try_get("source_system").ok()?;
            let source_ref: Option<String> = row.try_get("source_ref").ok()?;
            (nonblank(&source_system).is_some() && nonblank(&source_ref).is_some()).then(|| {
                Ok(TupleDependentRow {
                    id: row.try_get("id").map_err(repo_err)?,
                    client: tuple_from_parts(
                        row.try_get("source_client_id").map_err(repo_err)?,
                        source_system,
                        source_ref,
                    ),
                    canonical_download_id: row
                        .try_get("canonical_download_id")
                        .map_err(repo_err)?,
                })
            })
        })
        .collect()
}

async fn load_postgres_input(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<MigrationInput> {
    let submission_rows = sqlx::query(
        "SELECT s.id, s.title_id, s.download_client_id, s.download_client_type,
                s.download_client_item_id, s.download_id, s.submitted_at::text AS submitted_at,
                dc.name AS client_name
           FROM download_submissions s
           LEFT JOIN download_clients dc ON dc.id = s.download_client_id
          ORDER BY s.id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    let submissions = submission_rows
        .into_iter()
        .map(|row| {
            Ok(SubmissionRow {
                id: row.try_get("id").map_err(repo_err)?,
                title_id: row.try_get("title_id").map_err(repo_err)?,
                client: tuple_from_parts(
                    row.try_get("download_client_id").map_err(repo_err)?,
                    row.try_get("download_client_type").map_err(repo_err)?,
                    row.try_get("download_client_item_id").map_err(repo_err)?,
                ),
                client_name: row.try_get("client_name").map_err(repo_err)?,
                download_id: row.try_get("download_id").map_err(repo_err)?,
                submitted_at: row.try_get("submitted_at").map_err(repo_err)?,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;

    let client_names: HashMap<String, String> =
        sqlx::query_as("SELECT id, name FROM download_clients")
            .fetch_all(&mut **tx)
            .await
            .map_err(repo_err)?
            .into_iter()
            .collect();

    Ok(MigrationInput {
        client_names,
        submissions,
        identity_states: load_postgres_identity_states(tx).await?,
        imports: load_postgres_imports(tx).await?,
        import_artifacts: load_postgres_import_artifacts(tx).await?,
        queue_commands: load_postgres_queue_commands(tx).await?,
    })
}

async fn load_postgres_identity_states(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<IdentityStateRow>> {
    sqlx::query(
        "SELECT id, identity_key, client_id, client_type, download_client_item_id,
                canonical_download_id
           FROM download_identity_states ORDER BY id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?
    .into_iter()
    .map(|row| {
        Ok(IdentityStateRow {
            id: row.try_get("id").map_err(repo_err)?,
            identity_key: row.try_get("identity_key").map_err(repo_err)?,
            client: tuple_from_parts(
                row.try_get("client_id").map_err(repo_err)?,
                row.try_get("client_type").map_err(repo_err)?,
                row.try_get("download_client_item_id").map_err(repo_err)?,
            ),
            canonical_download_id: row.try_get("canonical_download_id").map_err(repo_err)?,
        })
    })
    .collect()
}

async fn load_postgres_imports(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<TupleDependentRow>> {
    load_postgres_tuple_dependents(
        tx,
        "SELECT id, source_client_id, source_system, source_ref, canonical_download_id FROM imports ORDER BY id",
    )
    .await
}

async fn load_postgres_import_artifacts(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<TupleDependentRow>> {
    load_postgres_tuple_dependents(
        tx,
        "SELECT id, source_client_id, source_system, source_ref, canonical_download_id FROM download_import_artifacts ORDER BY id",
    )
    .await
}

async fn load_postgres_queue_commands(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<TupleDependentRow>> {
    sqlx::query(
        "SELECT id, client_id, client_type, download_client_item_id, canonical_download_id
           FROM download_queue_commands ORDER BY id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?
    .into_iter()
    .map(|row| {
        Ok(TupleDependentRow {
            id: row.try_get("id").map_err(repo_err)?,
            client: tuple_from_parts(
                row.try_get("client_id").map_err(repo_err)?,
                row.try_get("client_type").map_err(repo_err)?,
                row.try_get("download_client_item_id").map_err(repo_err)?,
            ),
            canonical_download_id: row.try_get("canonical_download_id").map_err(repo_err)?,
        })
    })
    .collect()
}

async fn load_postgres_tuple_dependents(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    query: &'static str,
) -> AppResult<Vec<TupleDependentRow>> {
    sqlx::query(query)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?
        .into_iter()
        .filter_map(|row| {
            let source_system: Option<String> = row.try_get("source_system").ok()?;
            let source_ref: Option<String> = row.try_get("source_ref").ok()?;
            (nonblank(&source_system).is_some() && nonblank(&source_ref).is_some()).then(|| {
                Ok(TupleDependentRow {
                    id: row.try_get("id").map_err(repo_err)?,
                    client: tuple_from_parts(
                        row.try_get("source_client_id").map_err(repo_err)?,
                        source_system,
                        source_ref,
                    ),
                    canonical_download_id: row
                        .try_get("canonical_download_id")
                        .map_err(repo_err)?,
                })
            })
        })
        .collect()
}

async fn sqlite_backfill_is_complete(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    input: &MigrationInput,
) -> AppResult<bool> {
    let downloads = load_sqlite_download_ids(tx).await?;
    let bindings = load_sqlite_binding_ids(tx).await?;
    shape_is_backfilled(input, &downloads, &bindings)
}

async fn postgres_backfill_is_complete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: &MigrationInput,
) -> AppResult<bool> {
    let downloads = load_postgres_download_ids(tx).await?;
    let bindings = load_postgres_binding_ids(tx).await?;
    shape_is_backfilled(input, &downloads, &bindings)
}

fn shape_is_backfilled(
    input: &MigrationInput,
    downloads: &HashSet<String>,
    bindings: &HashSet<String>,
) -> AppResult<bool> {
    if input
        .submissions
        .iter()
        .any(|submission| Uuid::parse_str(&submission.id).is_err())
    {
        return Ok(false);
    }
    if input
        .submissions
        .iter()
        .any(|submission| !downloads.contains(&submission.id) || !bindings.contains(&submission.id))
    {
        return Ok(false);
    }

    if input
        .identity_states
        .iter()
        .any(|row| row.canonical_download_id.is_none())
        || input
            .imports
            .iter()
            .any(|row| row.canonical_download_id.is_none())
        || input
            .import_artifacts
            .iter()
            .any(|row| row.canonical_download_id.is_none())
        || input
            .queue_commands
            .iter()
            .any(|row| row.canonical_download_id.is_none())
    {
        return Ok(false);
    }

    let mut all_canonical_ids = input
        .identity_states
        .iter()
        .filter_map(|row| row.canonical_download_id.as_deref())
        .chain(
            input
                .imports
                .iter()
                .filter_map(|row| row.canonical_download_id.as_deref()),
        )
        .chain(
            input
                .import_artifacts
                .iter()
                .filter_map(|row| row.canonical_download_id.as_deref()),
        )
        .chain(
            input
                .queue_commands
                .iter()
                .filter_map(|row| row.canonical_download_id.as_deref()),
        );
    Ok(all_canonical_ids.all(|id| downloads.contains(id)))
}

async fn load_sqlite_download_ids(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<HashSet<String>> {
    sqlx::query_scalar("SELECT id FROM downloads")
        .fetch_all(&mut **tx)
        .await
        .map(|ids: Vec<String>| ids.into_iter().collect())
        .map_err(repo_err)
}

async fn load_sqlite_binding_ids(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<HashSet<String>> {
    sqlx::query_scalar("SELECT download_id FROM download_client_bindings")
        .fetch_all(&mut **tx)
        .await
        .map(|ids: Vec<String>| ids.into_iter().collect())
        .map_err(repo_err)
}

async fn load_postgres_download_ids(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<HashSet<String>> {
    sqlx::query_scalar("SELECT id FROM downloads")
        .fetch_all(&mut **tx)
        .await
        .map(|ids: Vec<String>| ids.into_iter().collect())
        .map_err(repo_err)
}

async fn load_postgres_binding_ids(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<HashSet<String>> {
    sqlx::query_scalar("SELECT download_id FROM download_client_bindings")
        .fetch_all(&mut **tx)
        .await
        .map(|ids: Vec<String>| ids.into_iter().collect())
        .map_err(repo_err)
}

async fn apply_sqlite_plan(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    plan: &BackfillPlan,
) -> AppResult<()> {
    for (old_id, new_id) in &plan.submission_id_updates {
        sqlx::query("UPDATE download_submissions SET id = ?1 WHERE id = ?2")
            .bind(new_id)
            .bind(old_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    for download in &plan.downloads {
        sqlx::query("INSERT INTO downloads (id, origin, created_at) VALUES (?1, ?2, ?3)")
            .bind(&download.id)
            .bind(download.origin)
            .bind(&download.created_at)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    for binding in &plan.bindings {
        sqlx::query(
            "INSERT INTO download_client_bindings (
                download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                native_item_id, created_at, ended_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(&binding.download_id)
        .bind(&binding.client.client_config_id)
        .bind(&binding.client.client_type)
        .bind(&binding.client_name_snapshot)
        .bind(&binding.client.native_item_id)
        .bind(&binding.created_at)
        .bind(&binding.ended_at)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    apply_sqlite_dependent_updates(tx, &plan.dependent_updates).await
}

async fn apply_sqlite_dependent_updates(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    updates: &HashMap<DependentRef, String>,
) -> AppResult<()> {
    for (reference, download_id) in updates {
        match reference.table {
            DependentTable::IdentityStates => sqlx::query(
                "UPDATE download_identity_states SET canonical_download_id = ?1 WHERE id = ?2",
            ),
            DependentTable::Imports => {
                sqlx::query("UPDATE imports SET canonical_download_id = ?1 WHERE id = ?2")
            }
            DependentTable::ImportArtifacts => sqlx::query(
                "UPDATE download_import_artifacts SET canonical_download_id = ?1 WHERE id = ?2",
            ),
            DependentTable::QueueCommands => sqlx::query(
                "UPDATE download_queue_commands SET canonical_download_id = ?1 WHERE id = ?2",
            ),
        }
        .bind(download_id)
        .bind(&reference.id)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    Ok(())
}

async fn apply_postgres_plan(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &BackfillPlan,
) -> AppResult<()> {
    for (old_id, new_id) in &plan.submission_id_updates {
        sqlx::query("UPDATE download_submissions SET id = $1 WHERE id = $2")
            .bind(new_id)
            .bind(old_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    for download in &plan.downloads {
        sqlx::query(
            "INSERT INTO downloads (id, origin, created_at)
             VALUES ($1, $2, ($3::text)::timestamptz)",
        )
        .bind(&download.id)
        .bind(download.origin)
        .bind(&download.created_at)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    for binding in &plan.bindings {
        sqlx::query(
            "INSERT INTO download_client_bindings (
                download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                native_item_id, created_at, ended_at
             ) VALUES ($1, $2, $3, $4, $5, ($6::text)::timestamptz,
                       ($7::text)::timestamptz)",
        )
        .bind(&binding.download_id)
        .bind(&binding.client.client_config_id)
        .bind(&binding.client.client_type)
        .bind(&binding.client_name_snapshot)
        .bind(&binding.client.native_item_id)
        .bind(&binding.created_at)
        .bind(&binding.ended_at)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    apply_postgres_dependent_updates(tx, &plan.dependent_updates).await
}

async fn apply_postgres_dependent_updates(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    updates: &HashMap<DependentRef, String>,
) -> AppResult<()> {
    for (reference, download_id) in updates {
        match reference.table {
            DependentTable::IdentityStates => sqlx::query(
                "UPDATE download_identity_states SET canonical_download_id = $1 WHERE id = $2",
            ),
            DependentTable::Imports => {
                sqlx::query("UPDATE imports SET canonical_download_id = $1 WHERE id = $2")
            }
            DependentTable::ImportArtifacts => sqlx::query(
                "UPDATE download_import_artifacts SET canonical_download_id = $1 WHERE id = $2",
            ),
            DependentTable::QueueCommands => sqlx::query(
                "UPDATE download_queue_commands SET canonical_download_id = $1 WHERE id = $2",
            ),
        }
        .bind(download_id)
        .bind(&reference.id)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    Ok(())
}

async fn verify_sqlite_backfill(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> AppResult<()> {
    let submission_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM download_submissions")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    let downloads = load_sqlite_download_ids(tx).await?;
    let bindings = load_sqlite_binding_ids(tx).await?;
    verify_common_shape(&submission_ids, &downloads, &bindings)?;
    verify_sqlite_canonical_references(tx, &downloads).await?;
    verify_sqlite_active_bindings(tx).await
}

async fn verify_postgres_backfill(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> AppResult<()> {
    let submission_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM download_submissions")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    let downloads = load_postgres_download_ids(tx).await?;
    let bindings = load_postgres_binding_ids(tx).await?;
    verify_common_shape(&submission_ids, &downloads, &bindings)?;
    verify_postgres_canonical_references(tx, &downloads).await?;
    verify_postgres_active_bindings(tx).await
}

fn verify_common_shape(
    submission_ids: &[String],
    downloads: &HashSet<String>,
    bindings: &HashSet<String>,
) -> AppResult<()> {
    for id in submission_ids {
        if Uuid::parse_str(id).is_err() {
            return Err(AppError::Repository(format!(
                "download submission id '{id}' is not a UUID after canonical identity backfill"
            )));
        }
        if !downloads.contains(id) || !bindings.contains(id) {
            return Err(AppError::Repository(format!(
                "download submission '{id}' does not have exactly one canonical download and binding"
            )));
        }
    }
    Ok(())
}

async fn verify_sqlite_canonical_references(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    downloads: &HashSet<String>,
) -> AppResult<()> {
    for (table, query) in [
        (
            "download_identity_states",
            "SELECT canonical_download_id FROM download_identity_states WHERE canonical_download_id IS NOT NULL",
        ),
        (
            "imports",
            "SELECT canonical_download_id FROM imports WHERE canonical_download_id IS NOT NULL",
        ),
        (
            "download_import_artifacts",
            "SELECT canonical_download_id FROM download_import_artifacts WHERE canonical_download_id IS NOT NULL",
        ),
        (
            "download_queue_commands",
            "SELECT canonical_download_id FROM download_queue_commands WHERE canonical_download_id IS NOT NULL",
        ),
    ] {
        let ids: Vec<String> = sqlx::query_scalar(query)
            .fetch_all(&mut **tx)
            .await
            .map_err(repo_err)?;
        verify_canonical_references(table, &ids, downloads)?;
    }
    Ok(())
}

async fn verify_postgres_canonical_references(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    downloads: &HashSet<String>,
) -> AppResult<()> {
    for (table, query) in [
        (
            "download_identity_states",
            "SELECT canonical_download_id FROM download_identity_states WHERE canonical_download_id IS NOT NULL",
        ),
        (
            "imports",
            "SELECT canonical_download_id FROM imports WHERE canonical_download_id IS NOT NULL",
        ),
        (
            "download_import_artifacts",
            "SELECT canonical_download_id FROM download_import_artifacts WHERE canonical_download_id IS NOT NULL",
        ),
        (
            "download_queue_commands",
            "SELECT canonical_download_id FROM download_queue_commands WHERE canonical_download_id IS NOT NULL",
        ),
    ] {
        let ids: Vec<String> = sqlx::query_scalar(query)
            .fetch_all(&mut **tx)
            .await
            .map_err(repo_err)?;
        verify_canonical_references(table, &ids, downloads)?;
    }
    Ok(())
}

fn verify_canonical_references(
    table: &str,
    ids: &[String],
    downloads: &HashSet<String>,
) -> AppResult<()> {
    for id in ids {
        if !downloads.contains(id) {
            return Err(AppError::Repository(format!(
                "{table}.canonical_download_id '{id}' has no downloads row"
            )));
        }
    }
    Ok(())
}

async fn verify_sqlite_active_bindings(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT client_config_id, client_type_snapshot, native_item_id, download_id
           FROM download_client_bindings
          WHERE ended_at IS NULL
            AND client_config_id IS NOT NULL
            AND client_type_snapshot IS NOT NULL
            AND native_item_id IS NOT NULL",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    verify_active_binding_rows(rows)
}

async fn verify_postgres_active_bindings(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT client_config_id, client_type_snapshot, native_item_id, download_id
           FROM download_client_bindings
          WHERE ended_at IS NULL
            AND client_config_id IS NOT NULL
            AND client_type_snapshot IS NOT NULL
            AND native_item_id IS NOT NULL",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    verify_active_binding_rows(rows)
}

fn verify_active_binding_rows(rows: Vec<(String, String, String, String)>) -> AppResult<()> {
    let mut downloads_by_locator = BTreeMap::<(String, String, String), Vec<String>>::new();
    for (client_config_id, client_type, native_item_id, download_id) in rows {
        downloads_by_locator
            .entry((client_config_id, client_type, native_item_id))
            .or_default()
            .push(download_id);
    }
    for (locator, mut download_ids) in downloads_by_locator {
        if download_ids.len() > 1 {
            download_ids.sort();
            return Err(AppError::Repository(format!(
                "active download bindings conflict for locator {:?}: {}",
                locator,
                download_ids.join(", ")
            )));
        }
    }
    Ok(())
}

fn repo_err(error: sqlx::Error) -> AppError {
    AppError::Repository(error.to_string())
}
