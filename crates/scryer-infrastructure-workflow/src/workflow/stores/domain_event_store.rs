use super::*;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, DashboardActivityStats, DomainEventRepository};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventType, NewDomainEvent, TitleHistoryEventType,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct DomainEventStore {
    datastore: StoreDatastore,
}

impl DomainEventStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl DomainEventRepository for DomainEventStore {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        append_domain_events(&self.datastore, vec![event])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Repository("failed to append domain event".into()))
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        append_domain_events(&self.datastore, events).await
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        let (sql, args) = build_domain_event_list_sql(filter);
        fetch_domain_events(self.datastore.read_exec(), &sql, &args).await
    }

    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64> {
        let (where_sql, args) =
            build_title_history_filter_sql(&self.datastore, event_types, title_ids, download_id);
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!("SELECT COUNT(*) AS count FROM domain_events{where_sql}"),
            &args,
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing domain event count".into()))?;
        row.i64("count")
    }

    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let page_size = if limit == 0 { 50 } else { limit.min(500) };
        let (where_sql, mut args) =
            build_title_history_filter_sql(&self.datastore, event_types, title_ids, download_id);
        args.push(SqlArg::I64(page_size as i64));
        args.push(SqlArg::I64(offset as i64));
        fetch_domain_events(
            self.datastore.read_exec(),
            &format!(
                "SELECT {DOMAIN_EVENT_COLUMNS} FROM domain_events{where_sql} ORDER BY sequence DESC LIMIT {{}} OFFSET {{}}"
            ),
            &args,
        )
        .await
    }

    async fn count_dashboard_activity_events(
        &self,
        library_ids: &[String],
        previous_start: DateTime<Utc>,
        current_start: DateTime<Utc>,
        current_end: DateTime<Utc>,
    ) -> AppResult<DashboardActivityStats> {
        if library_ids.is_empty() {
            return Ok(DashboardActivityStats::default());
        }

        let (sql, args) = build_dashboard_activity_stats_sql(
            &self.datastore,
            library_ids,
            previous_start,
            current_start,
            current_end,
        );
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;

        let mut stats = DashboardActivityStats::default();
        for row in rows {
            let window_key = row.text("window_key")?;
            let event_type = row.text("event_type")?;
            let count = row.i64("event_count")?;
            let window = if window_key == "current" {
                &mut stats.current
            } else {
                &mut stats.previous
            };
            // `import_rejected` reaches this point only with a failed status:
            // the aggregate already filtered skipped rejections out.
            match DomainEventType::parse(&event_type) {
                Some(DomainEventType::ReleaseGrabbed) => window.grabbed += count,
                Some(DomainEventType::MediaFileUpgraded) => window.upgraded += count,
                Some(DomainEventType::ImportCompleted) => window.imported += count,
                Some(DomainEventType::ImportRejected) => window.import_failed += count,
                Some(DomainEventType::DownloadFailed) => window.download_failed += count,
                _ => {}
            }
        }
        Ok(stats)
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let filter = DomainEventFilter {
            after_sequence: Some(after_sequence),
            limit,
            ..DomainEventFilter::default()
        };
        self.list(&filter).await
    }

    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        if title_ids.is_empty() {
            return Ok(0);
        }
        let mut args = Vec::with_capacity(title_ids.len() + 1);
        args.extend(title_ids.iter().cloned().map(SqlArg::Text));
        args.push(SqlArg::Text(DomainEventType::TitleDeleted.as_str().into()));
        let rows = execute_write(
            &self.datastore,
            "delete_domain_events_for_title_ids",
            format!(
                "DELETE FROM domain_events WHERE title_id IN ({}) AND event_type <> {{}}",
                placeholders(title_ids.len())
            ),
            args,
        )
        .await?;
        Ok(rows as u32)
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT sequence FROM event_subscriber_offsets WHERE subscriber_name = {}",
            &[SqlArg::Text(subscriber.to_string())],
        )
        .await?;
        Ok(row.map(|row| row.i64("sequence")).transpose()?.unwrap_or(0))
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        let subscriber = subscriber.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "set_event_subscriber_offset", move |tx| {
            let subscriber = subscriber.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO event_subscriber_offsets (subscriber_name, sequence, updated_at)
                         VALUES ({}, {}, {})
                         ON CONFLICT(subscriber_name) DO UPDATE SET
                            sequence = excluded.sequence,
                            updated_at = excluded.updated_at",
                    &[
                        SqlArg::Text(subscriber),
                        SqlArg::I64(sequence),
                        SqlArg::Timestamp(Utc::now()),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }
}

#[cfg(test)]
mod title_history_filter_tests {
    use std::sync::Arc;

    use scryer_domain::{
        DomainEventActorKind, DomainEventPayload, DomainEventStream, DomainExternalIds,
        DownloadIgnoredEventData, ImportRejectedEventData, ImportStatus, MediaFacet,
        MediaFileDeletedEventData, MediaFileDeletedReason, ReleaseGrabbedEventData,
        TitleContextSnapshot,
    };
    use sqlx::{Row, sqlite::SqlitePoolOptions};

    use super::*;

    async fn store() -> DomainEventStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::query(
            "CREATE TABLE domain_events(
                 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                 event_id TEXT NOT NULL UNIQUE,
                 occurred_at TEXT NOT NULL,
                 actor_kind TEXT NOT NULL DEFAULT 'system',
                 actor_user_id TEXT,
                 actor_display_name TEXT NOT NULL DEFAULT 'System',
                 title_id TEXT,
                 facet TEXT,
                 correlation_id TEXT,
                 causation_id TEXT,
                 schema_version INTEGER NOT NULL,
                 stream_kind TEXT NOT NULL,
                 stream_id TEXT,
                 event_type TEXT NOT NULL,
                 payload_json BLOB NOT NULL,
                 import_status TEXT,
                 media_file_delete_reason TEXT,
                 download_id TEXT
             )",
        )
        .execute(&pool)
        .await
        .expect("domain event table should be created");
        DomainEventStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    fn download_ignored_event() -> NewDomainEvent {
        NewDomainEvent {
            event_id: "event-1".to_string(),
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some("title-1".to_string()),
            facet: Some(MediaFacet::Movie),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: "title-1".to_string(),
            },
            payload: DomainEventPayload::DownloadIgnored(DownloadIgnoredEventData {
                title: None,
                download_client_item_id: "job-1".to_string(),
                client_id: Some("primary".to_string()),
                client_type: Some("qbittorrent".to_string()),
                source_provider: None,
                source_title: Some("Some Release".to_string()),
            }),
        }
    }

    fn title_snapshot() -> TitleContextSnapshot {
        TitleContextSnapshot {
            title_name: "Projection Test".to_string(),
            facet: MediaFacet::Movie,
            external_ids: DomainExternalIds::default(),
            poster_url: None,
            year: None,
        }
    }

    fn event_with_payload(event_id: &str, payload: DomainEventPayload) -> NewDomainEvent {
        let mut event = download_ignored_event();
        event.event_id = event_id.to_string();
        event.payload = payload;
        event
    }

    /// Filtering the history page by "download ignored" used to push a literal
    /// never-match clause, so the filtered page came back empty while the
    /// unfiltered page showed the very same rows.
    #[tokio::test]
    async fn a_download_ignored_row_survives_its_own_filter() {
        let store = store().await;
        store
            .append(download_ignored_event())
            .await
            .expect("event should append");

        let unfiltered = store
            .list_title_history_page_events(None, None, None, 50, 0)
            .await
            .expect("unfiltered page should load");
        assert_eq!(unfiltered.len(), 1, "the row is on the unfiltered page");

        let filtered = store
            .list_title_history_page_events(
                Some(&[TitleHistoryEventType::DownloadIgnored]),
                None,
                None,
                50,
                0,
            )
            .await
            .expect("filtered page should load");
        assert_eq!(filtered.len(), 1, "and must survive its own filter");
        assert_eq!(filtered[0].event_id, "event-1");

        assert_eq!(
            store
                .count_title_history_page_events(
                    Some(&[TitleHistoryEventType::DownloadIgnored]),
                    None,
                    None
                )
                .await
                .expect("filtered count should load"),
            1
        );
    }

    /// `DownloadCompleted` has no stored domain-event equivalent, so its
    /// never-match clause is correct and must stay.
    #[tokio::test]
    async fn download_completed_has_nothing_to_match() {
        let store = store().await;
        store
            .append(download_ignored_event())
            .await
            .expect("event should append");

        let filtered = store
            .list_title_history_page_events(
                Some(&[TitleHistoryEventType::DownloadCompleted]),
                None,
                None,
                50,
                0,
            )
            .await
            .expect("filtered page should load");
        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn append_populates_query_projections() {
        let store = store().await;
        store
            .append_many(vec![
                event_with_payload(
                    "import-rejected",
                    DomainEventPayload::ImportRejected(ImportRejectedEventData {
                        title: None,
                        status: ImportStatus::Failed,
                        import_id: None,
                        source_system: None,
                        source_ref: None,
                        source_title: None,
                        source_path: None,
                        dest_path: None,
                        quality: None,
                        reason: None,
                        skip_reason: None,
                        episode_ids: Vec::new(),
                    }),
                ),
                event_with_payload(
                    "media-file-deleted",
                    DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                        title: title_snapshot(),
                        media_updates: Vec::new(),
                        file_id: None,
                        reason: MediaFileDeletedReason::UpgradeCleanup,
                        episode_ids: Vec::new(),
                    }),
                ),
                event_with_payload(
                    "release-grabbed",
                    DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                        title: title_snapshot(),
                        source_title: None,
                        source_hint: None,
                        source_provider: None,
                        download_id: Some("download-1".to_string()),
                        episode_ids: Vec::new(),
                    }),
                ),
            ])
            .await
            .expect("events should append");

        let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
            unreachable!("test store is sqlite")
        };
        let rows = sqlx::query(
            "SELECT event_id, import_status, media_file_delete_reason, download_id
               FROM domain_events
              ORDER BY event_id",
        )
        .fetch_all(pool)
        .await
        .expect("projection rows should load");

        assert_eq!(rows[0].get::<String, _>("event_id"), "import-rejected");
        assert_eq!(rows[0].get::<String, _>("import_status"), "failed");
        assert_eq!(
            rows[1].get::<String, _>("media_file_delete_reason"),
            "upgrade_cleanup"
        );
        assert_eq!(rows[2].get::<String, _>("download_id"), "download-1");
    }
}
