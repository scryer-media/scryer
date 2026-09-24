use scryer_application::AppResult;
use scryer_domain::import_space::{SpaceIncidentEvent, SpaceIncidentUpdate, SpaceMeasurement};
use scryer_domain::{
    DomainEvent, DomainEventActorKind, DomainEventPayload, DomainEventStream, Id, NewDomainEvent,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, SqlTx, StoreDatastore, repo_err};

pub async fn update(
    datastore: &StoreDatastore,
    update: SpaceIncidentUpdate,
) -> AppResult<Vec<DomainEvent>> {
    SqlRuntime::run_in_transaction(datastore, "update_import_space_incident", move |tx| {
        let update = update.clone();
        Box::pin(async move {
            // The first statement takes a write lock on both engines. Serialize
            // destination transitions, including cross-destination reassignment.
            SqlRuntime::execute(SqlExec::Tx(tx), "UPDATE import_space_incident_lock SET revision = 0 WHERE id = 1", &[]).await?;
            let mut events = Vec::new();
            match update {
                SpaceIncidentUpdate::Observed { member_key, job_key, download_id: expected_download_id, measurement, blocked } => {
                    let Some(download_id) = active_download_id(tx, &job_key, expected_download_id.as_deref()).await? else {
                        retire_job(tx, &job_key, expected_download_id.as_deref()).await?;
                        return Ok(events);
                    };
                    let previous = SqlRuntime::fetch_optional(SqlExec::Tx(tx), "SELECT destination_key, download_id FROM import_space_members WHERE member_key = {}", &[SqlArg::Text(member_key.clone())]).await?;
                    if let Some(previous) = previous {
                        let key = previous.text("destination_key")?;
                        let same_download = previous.text("download_id")? == download_id;
                        if key != measurement.destination_key || !blocked || !same_download {
                            let passed = (!blocked && key == measurement.destination_key && same_download).then(|| measurement.clone());
                            clear_member(tx, &key, &member_key, passed, &mut events).await?;
                        }
                    }
                    if blocked {
                        let key = &measurement.destination_key;
                        let existing = SqlRuntime::fetch_optional(SqlExec::Tx(tx), "SELECT incident_id FROM import_space_incidents WHERE destination_key = {}", &[SqlArg::Text(key.clone())]).await?;
                        let opening = existing.is_none();
                        let incident_id = match existing {
                            Some(row) => row.text("incident_id")?,
                            None => Id::new().0,
                        };
                        if opening {
                            SqlRuntime::execute(SqlExec::Tx(tx), "INSERT INTO import_space_incidents (destination_key, incident_id) VALUES ({}, {})", &[SqlArg::Text(key.clone()), SqlArg::Text(incident_id.clone())]).await?;
                        }
                        // Update only this file's evidence; a retry must not read or
                        // rewrite every other blocked file on the destination.
                        SqlRuntime::execute(SqlExec::Tx(tx), "INSERT INTO import_space_members (member_key, job_key, destination_key, measurement_json, download_id) VALUES ({}, {}, {}, {}, {}) ON CONFLICT(member_key) DO UPDATE SET job_key = excluded.job_key, destination_key = excluded.destination_key, measurement_json = excluded.measurement_json, download_id = excluded.download_id", &[SqlArg::Text(member_key), SqlArg::Text(job_key), SqlArg::Text(key.clone()), SqlArg::Text(serde_json::to_string(&measurement).map_err(repo_err)?), SqlArg::Text(download_id)]).await?;
                        if opening {
                            events.push(append(tx, SpaceIncidentEvent { incident_id, measurement, affected_import_count: 1, recovered: false }).await?);
                        }
                    }
                }
                SpaceIncidentUpdate::Retired { job_key, download_id } => {
                    retire_job(tx, &job_key, download_id.as_deref()).await?;
                }
            }
            Ok(events)
        })
    }).await
}

// None means positively retired; an empty ID represents a manual or unbound job.
async fn active_download_id(
    tx: &mut SqlTx<'_>,
    job: &str,
    expected: Option<&str>,
) -> AppResult<Option<String>> {
    let Ok((client_type, client_id, item_id)) =
        serde_json::from_str::<(String, String, String)>(job)
    else {
        return Ok(Some(String::new()));
    };
    let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
        "SELECT b.download_id, CASE WHEN b.ended_at IS NULL THEN 0 ELSE 1 END AS ended, s.tracked_state
         FROM download_client_bindings b LEFT JOIN download_submissions s ON s.id = b.download_id
         WHERE COALESCE(b.client_config_id, '') = {}
           AND LOWER(TRIM(COALESCE(b.client_type_snapshot, ''))) = {}
           AND b.native_item_id = {}
         ORDER BY CASE WHEN b.ended_at IS NULL THEN 0 ELSE 1 END, b.created_at DESC, b.download_id
         LIMIT 1",
        &[SqlArg::Text(client_id), SqlArg::Text(client_type.trim().to_ascii_lowercase()), SqlArg::Text(item_id)]).await?;
    match row {
        Some(row)
            if row.i64("ended")? != 0
                || row.opt_text("tracked_state")?.as_deref() == Some("ignored") =>
        {
            Ok(None)
        }
        Some(row) => {
            let actual = row.text("download_id")?;
            Ok((expected.is_none() || expected == Some(actual.as_str())).then_some(actual))
        }
        None => Ok(expected.is_none().then(String::new)),
    }
}

async fn retire_job(tx: &mut SqlTx<'_>, job: &str, download_id: Option<&str>) -> AppResult<()> {
    let members = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        "SELECT member_key, destination_key FROM import_space_members WHERE job_key = {} AND download_id = {}",
        &[SqlArg::Text(job.into()), SqlArg::Text(download_id.unwrap_or_default().into())],
    )
    .await?;
    for member in members {
        clear_member(
            tx,
            &member.text("destination_key")?,
            &member.text("member_key")?,
            None,
            &mut Vec::new(),
        )
        .await?;
    }
    Ok(())
}

/// Repair observer state after a crash between authoritative cancellation/removal
/// and its best-effort observer update. This never changes import workflow state.
pub async fn reconcile(datastore: &StoreDatastore) -> AppResult<()> {
    SqlRuntime::run_in_transaction(datastore, "reconcile_import_space_incidents", |tx| Box::pin(async move {
        SqlRuntime::execute(SqlExec::Tx(tx), "UPDATE import_space_incident_lock SET revision = 0 WHERE id = 1", &[]).await?;
        let members = SqlRuntime::fetch_all(SqlExec::Tx(tx),
            "SELECT m.member_key, m.destination_key FROM import_space_members m
             WHERE m.download_id <> '' AND (
                 NOT EXISTS (SELECT 1 FROM download_client_bindings b WHERE b.download_id = m.download_id AND b.ended_at IS NULL)
                 OR EXISTS (SELECT 1 FROM download_submissions s WHERE s.id = m.download_id AND s.tracked_state = 'ignored'))", &[]).await?;
        for member in members {
            clear_member(tx, &member.text("destination_key")?, &member.text("member_key")?, None, &mut Vec::new()).await?;
        }
        Ok(())
    })).await
}

async fn clear_member(
    tx: &mut SqlTx<'_>,
    key: &str,
    member: &str,
    passed: Option<SpaceMeasurement>,
    events: &mut Vec<DomainEvent>,
) -> AppResult<()> {
    let removed = SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM import_space_members WHERE member_key = {}",
        &[SqlArg::Text(member.into())],
    )
    .await?;
    if removed == 0
        || SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT member_key FROM import_space_members WHERE destination_key = {} LIMIT 1",
            &[SqlArg::Text(key.into())],
        )
        .await?
        .is_some()
    {
        return Ok(());
    }
    let incident = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT incident_id FROM import_space_incidents WHERE destination_key = {}",
        &[SqlArg::Text(key.into())],
    )
    .await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM import_space_incidents WHERE destination_key = {}",
        &[SqlArg::Text(key.into())],
    )
    .await?;
    if let (Some(incident), Some(measurement)) = (incident, passed) {
        events.push(
            append(
                tx,
                SpaceIncidentEvent {
                    incident_id: incident.text("incident_id")?,
                    measurement,
                    affected_import_count: 0,
                    recovered: true,
                },
            )
            .await?,
        );
    }
    Ok(())
}

async fn append(tx: &mut SqlTx<'_>, event: SpaceIncidentEvent) -> AppResult<DomainEvent> {
    let payload = if event.recovered {
        DomainEventPayload::ImportSpaceRestored(event)
    } else {
        DomainEventPayload::ImportSpaceBlocked(event)
    };
    super::core::append_domain_event_tx(
        tx,
        NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: chrono::Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "Scryer".into(),
            title_id: None,
            facet: None,
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Global,
            payload,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::super::DomainEventStore;
    use super::*;
    use scryer_application::DomainEventRepository;

    const SQLITE_MIGRATION: &str =
        include_str!("../../../../scryer/src/db/migrations/0256_import_space_incidents.sql");
    const POSTGRES_MIGRATION: &str = include_str!(
        "../../../../scryer/src/db/postgres/migrations/0256_import_space_incidents.sql"
    );
    const EVENT_COLUMNS: &str = "event_id TEXT NOT NULL UNIQUE, occurred_at TEXT NOT NULL, actor_kind TEXT NOT NULL, actor_user_id TEXT, actor_display_name TEXT NOT NULL, title_id TEXT, facet TEXT, correlation_id TEXT, causation_id TEXT, schema_version INTEGER NOT NULL, stream_kind TEXT NOT NULL, stream_id TEXT, event_type TEXT NOT NULL, payload_json BLOB NOT NULL, import_status TEXT, media_file_delete_reason TEXT, download_id TEXT";

    async fn sqlite() -> StoreDatastore {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(SQLITE_MIGRATION)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE TABLE domain_events (sequence INTEGER PRIMARY KEY AUTOINCREMENT, {EVENT_COLUMNS})"))).execute(&pool).await.unwrap();
        sqlx::raw_sql("CREATE TABLE download_client_bindings (download_id TEXT PRIMARY KEY, client_config_id TEXT, client_type_snapshot TEXT, native_item_id TEXT, created_at TEXT NOT NULL, ended_at TEXT); CREATE TABLE download_submissions (id TEXT PRIMARY KEY, tracked_state TEXT);").execute(&pool).await.unwrap();
        StoreDatastore::Sqlite {
            pool,
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn observed(job: &str, file: &str, destination: &str, blocked: bool) -> SpaceIncidentUpdate {
        SpaceIncidentUpdate::Observed {
            job_key: job.into(),
            download_id: None,
            member_key: format!("{job}/{file}"),
            blocked,
            measurement: SpaceMeasurement {
                destination_key: destination.into(),
                destination: format!("/library/{destination}"),
                available_bytes: if blocked { 5 } else { 20 },
                required_bytes: 10,
            },
        }
    }

    async fn lifecycle(datastore: StoreDatastore) {
        let store = DomainEventStore::new(datastore.clone());
        let first = store
            .update_import_space_incident(observed("job1", "file1", "a", true))
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        let DomainEventPayload::ImportSpaceBlocked(first_payload) = &first[0].payload else {
            panic!("expected blocked event")
        };
        assert_eq!(first_payload.measurement.available_bytes, 5);
        assert!(
            store
                .update_import_space_incident(observed("job1", "file1", "a", true))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .update_import_space_incident(observed("job1", "file2", "a", true))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .update_import_space_incident(observed("job2", "file1", "a", true))
                .await
                .unwrap()
                .is_empty()
        );
        let current = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT COUNT(DISTINCT job_key) AS affected FROM import_space_members WHERE destination_key = {}",
            &[SqlArg::Text("a".into())],
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(current.i64("affected").unwrap(), 2);
        assert_eq!(
            store
                .update_import_space_incident(observed("job3", "file1", "b", true))
                .await
                .unwrap()
                .len(),
            1
        );

        // A new store has no in-memory knowledge; persisted membership controls replay.
        let restarted = DomainEventStore::new(datastore);
        assert!(
            restarted
                .update_import_space_incident(observed("job1", "file1", "a", true))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            restarted
                .update_import_space_incident(observed("job1", "file1", "a", false))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            restarted
                .update_import_space_incident(observed("job1", "file2", "a", false))
                .await
                .unwrap()
                .is_empty()
        );
        let recovered = restarted
            .update_import_space_incident(observed("job2", "file1", "a", false))
            .await
            .unwrap();
        assert!(matches!(
            recovered.as_slice(),
            [DomainEvent {
                payload: DomainEventPayload::ImportSpaceRestored(_),
                ..
            }]
        ));
        assert!(
            restarted
                .update_import_space_incident(observed("job2", "file1", "a", false))
                .await
                .unwrap()
                .is_empty()
        );
        let again = restarted
            .update_import_space_incident(observed("job1", "file1", "a", true))
            .await
            .unwrap();
        let DomainEventPayload::ImportSpaceBlocked(again) = &again[0].payload else {
            panic!("expected new incident")
        };
        assert_ne!(again.incident_id, first_payload.incident_id);
        assert!(
            restarted
                .update_import_space_incident(SpaceIncidentUpdate::Retired {
                    job_key: "job1".into(),
                    download_id: None,
                })
                .await
                .unwrap()
                .is_empty()
        );
        // Redirecting the last member closes its old destination silently.
        let redirected = restarted
            .update_import_space_incident(observed("job3", "file1", "c", true))
            .await
            .unwrap();
        assert!(matches!(
            redirected.as_slice(),
            [DomainEvent {
                payload: DomainEventPayload::ImportSpaceBlocked(_),
                ..
            }]
        ));
        restarted
            .mark_import_space_notification_delivered(&first[0].event_id, "target1")
            .await
            .unwrap();
        restarted
            .mark_import_space_notification_delivered(&first[0].event_id, "target1")
            .await
            .unwrap();
        assert!(
            restarted
                .import_space_notification_delivered(&first[0].event_id, "target1")
                .await
                .unwrap()
        );
        assert!(
            !restarted
                .import_space_notification_delivered(&first[0].event_id, "target2")
                .await
                .unwrap()
        );
    }

    async fn binding_recovery(datastore: &StoreDatastore) {
        SqlRuntime::execute(datastore.read_exec(), "INSERT INTO download_client_bindings (download_id, client_config_id, client_type_snapshot, native_item_id, created_at) VALUES ('portable', 'portable-client', 'sabnzbd', 'portable-item', '2026-01-01')", &[]).await.unwrap();
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO download_submissions VALUES ('portable', 'import_pending')",
            &[],
        )
        .await
        .unwrap();
        let job = r#"["sabnzbd","portable-client","portable-item"]"#;
        let mut blocked = observed(job, "file", "portable-volume", true);
        if let SpaceIncidentUpdate::Observed { download_id, .. } = &mut blocked {
            *download_id = Some("portable".into());
        }
        assert_eq!(update(datastore, blocked.clone()).await.unwrap().len(), 1);
        SqlRuntime::execute(
            datastore.read_exec(),
            "UPDATE download_submissions SET tracked_state = 'ignored' WHERE id = 'portable'",
            &[],
        )
        .await
        .unwrap();
        reconcile(datastore).await.unwrap();
        assert!(update(datastore, blocked.clone()).await.unwrap().is_empty());
        let row = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT member_key FROM import_space_members WHERE download_id = 'portable'",
            &[],
        )
        .await
        .unwrap();
        assert!(row.is_none());
        SqlRuntime::execute(datastore.read_exec(), "UPDATE download_submissions SET tracked_state = 'import_pending' WHERE id = 'portable'", &[]).await.unwrap();
        assert_eq!(update(datastore, blocked.clone()).await.unwrap().len(), 1);
        if let SpaceIncidentUpdate::Observed {
            blocked,
            measurement,
            ..
        } = &mut blocked
        {
            *blocked = false;
            measurement.available_bytes = 20;
        }
        assert!(matches!(
            update(datastore, blocked).await.unwrap().as_slice(),
            [DomainEvent {
                payload: DomainEventPayload::ImportSpaceRestored(_),
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn sqlite_import_space_lifecycle() {
        let datastore = sqlite().await;
        lifecycle(datastore.clone()).await;
        binding_recovery(&datastore).await;
    }

    #[tokio::test]
    async fn sqlite_import_space_concurrent_failures_open_only_one_incident() {
        let datastore = sqlite().await;
        let (left, right) = tokio::join!(
            update(&datastore, observed("job1", "file", "a", true)),
            update(&datastore, observed("job2", "file", "a", true)),
        );
        assert_eq!(left.unwrap().len() + right.unwrap().len(), 1);
        let members = SqlRuntime::fetch_all(
            datastore.read_exec(),
            "SELECT member_key FROM import_space_members",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(members.len(), 2);
    }

    #[tokio::test]
    async fn sqlite_import_space_repairs_cancellation_without_preventing_explicit_retry() {
        let datastore = sqlite().await;
        let StoreDatastore::Sqlite { pool, .. } = &datastore else {
            unreachable!()
        };
        sqlx::raw_sql("INSERT INTO download_client_bindings VALUES ('download1', 'client', 'sabnzbd', 'item', '2026-01-01', NULL); INSERT INTO download_submissions VALUES ('download1', 'import_pending');").execute(pool).await.unwrap();
        let job = r#"["sabnzbd","client","item"]"#;
        assert_eq!(
            update(&datastore, observed(job, "file", "a", true))
                .await
                .unwrap()
                .len(),
            1
        );
        // Simulate a crash after the authoritative update, before observer retirement.
        sqlx::raw_sql("UPDATE download_submissions SET tracked_state = 'ignored'")
            .execute(pool)
            .await
            .unwrap();
        let restarted = DomainEventStore::new(datastore.clone());
        restarted.reconcile_import_space_incidents().await.unwrap();
        assert!(
            update(&datastore, observed(job, "file", "a", true))
                .await
                .unwrap()
                .is_empty(),
            "late observation must not reopen ignored work"
        );
        let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM import_space_members), (SELECT COUNT(*) FROM domain_events)").fetch_one(pool).await.unwrap();
        assert_eq!(counts, (0, 1), "retirement must not fabricate recovery");
        sqlx::raw_sql("UPDATE download_submissions SET tracked_state = 'import_pending'")
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            update(&datastore, observed(job, "file", "a", true))
                .await
                .unwrap()
                .len(),
            1,
            "explicit retry can open a new incident"
        );
        assert_eq!(
            update(&datastore, observed(job, "file", "a", false))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn sqlite_import_space_retired_binding_and_reused_locator_are_distinct() {
        let datastore = sqlite().await;
        let StoreDatastore::Sqlite { pool, .. } = &datastore else {
            unreachable!()
        };
        sqlx::raw_sql("INSERT INTO download_client_bindings VALUES ('download1', 'client', 'sabnzbd', 'item', '2026-01-01', NULL); INSERT INTO download_submissions VALUES ('download1', 'import_pending');").execute(pool).await.unwrap();
        let job = r#"["sabnzbd","client","item"]"#;
        update(&datastore, observed(job, "file", "a", true))
            .await
            .unwrap();
        sqlx::raw_sql("UPDATE download_client_bindings SET ended_at = '2026-01-02'")
            .execute(pool)
            .await
            .unwrap();
        reconcile(&datastore).await.unwrap();
        assert!(
            update(&datastore, observed(job, "file", "a", true))
                .await
                .unwrap()
                .is_empty()
        );
        sqlx::raw_sql("INSERT INTO download_client_bindings VALUES ('download2', 'client', 'sabnzbd', 'item', '2026-01-03', NULL); INSERT INTO download_submissions VALUES ('download2', 'import_pending');").execute(pool).await.unwrap();
        assert_eq!(
            update(&datastore, observed(job, "file", "a", true))
                .await
                .unwrap()
                .len(),
            1
        );
        reconcile(&datastore).await.unwrap();
        let member: (String,) = sqlx::query_as("SELECT download_id FROM import_space_members")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(member.0, "download2");
        for blocked in [false, true] {
            let mut stale = observed(job, "file", "a", blocked);
            if let SpaceIncidentUpdate::Observed { download_id, .. } = &mut stale {
                *download_id = Some("download1".into());
            }
            assert!(update(&datastore, stale).await.unwrap().is_empty());
        }
        update(
            &datastore,
            SpaceIncidentUpdate::Retired {
                job_key: job.into(),
                download_id: Some("download1".into()),
            },
        )
        .await
        .unwrap();
        let retained: (String,) = sqlx::query_as("SELECT download_id FROM import_space_members")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(
            retained.0, "download2",
            "late checks and retirement from the old identity must preserve the replacement"
        );
        let events: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM domain_events")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(
            events.0, 2,
            "source removal closes silently and reuse opens anew"
        );
    }

    #[tokio::test]
    async fn sqlite_import_space_retry_only_writes_its_own_member() {
        let datastore = sqlite().await;
        let StoreDatastore::Sqlite { pool, .. } = &datastore else {
            unreachable!()
        };
        for index in 0..200 {
            update(
                &datastore,
                observed(&format!("job{index}"), "file", "a", true),
            )
            .await
            .unwrap();
        }
        sqlx::raw_sql("CREATE TRIGGER reject_other_member BEFORE UPDATE ON import_space_members WHEN OLD.job_key <> 'job0' BEGIN SELECT RAISE(ABORT, 'unrelated member written'); END; CREATE TRIGGER reject_incident_rewrite BEFORE UPDATE ON import_space_incidents BEGIN SELECT RAISE(ABORT, 'whole incident rewritten'); END;").execute(pool).await.unwrap();
        assert!(
            update(&datastore, observed("job0", "file", "a", true))
                .await
                .unwrap()
                .is_empty()
        );
        let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM import_space_members), (SELECT COUNT(*) FROM domain_events)").fetch_one(pool).await.unwrap();
        assert_eq!(counts, (200, 1));
    }

    #[tokio::test]
    async fn sqlite_import_space_event_failure_rolls_back_membership() {
        let datastore = sqlite().await;
        let StoreDatastore::Sqlite { pool, .. } = &datastore else {
            unreachable!()
        };
        sqlx::raw_sql("CREATE TRIGGER reject_event BEFORE INSERT ON domain_events BEGIN SELECT RAISE(ABORT, 'injected persistence failure'); END;").execute(pool).await.unwrap();
        assert!(
            update(&datastore, observed("job", "file", "a", true))
                .await
                .is_err()
        );
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM import_space_members")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0);
        sqlx::raw_sql("DROP TRIGGER reject_event")
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            update(&datastore, observed("job", "file", "a", true))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires SCRYER_TEST_POSTGRES_URL pointing to an isolated fixture database"]
    async fn postgres_import_space_lifecycle() {
        let url = std::env::var("SCRYER_TEST_POSTGRES_URL").expect("isolated fixture database URL");
        let schema = format!("space_fixture_{}", Id::new().0.replace('-', ""));
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE SCHEMA {schema}; SET search_path TO {schema}"
        )))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(POSTGRES_MIGRATION)
            .execute(&pool)
            .await
            .unwrap();
        let columns = EVENT_COLUMNS
            .replace("occurred_at TEXT", "occurred_at TIMESTAMPTZ")
            .replace("payload_json BLOB", "payload_json BYTEA");
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE TABLE domain_events (sequence BIGSERIAL PRIMARY KEY, {columns})"
        )))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql("CREATE TABLE download_client_bindings (download_id TEXT PRIMARY KEY, client_config_id TEXT, client_type_snapshot TEXT, native_item_id TEXT, created_at TIMESTAMPTZ NOT NULL, ended_at TIMESTAMPTZ); CREATE TABLE download_submissions (id TEXT PRIMARY KEY, tracked_state TEXT);").execute(&pool).await.unwrap();
        let datastore = StoreDatastore::Postgres { pool: pool.clone() };
        lifecycle(datastore.clone()).await;
        binding_recovery(&datastore).await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }
}
