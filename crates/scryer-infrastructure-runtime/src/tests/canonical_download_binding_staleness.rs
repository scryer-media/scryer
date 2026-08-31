//! A locator whose binding points at a download that already reached a
//! terminal outcome must not hand that identity to the next claim: the fresh
//! grab would inherit the old import or failure history and read as finished
//! before it starts. Live bindings keep being adopted — that is what makes
//! dedup-by-hash clients work.

use super::*;
use scryer_application::{
    ClientJobLocator, DownloadRegistryRepository, DownloadSubmissionIdentity,
    DownloadSubmissionPurpose, DownloadSubmissionRepository,
};

const CLIENT_ID: &str = "client-one";
const CLIENT_TYPE: &str = "qbittorrent";
const ITEM_ID: &str = "0123456789abcdef0123456789abcdef01234567";

struct StaleBindingFixture {
    services: SqliteServices,
    db: std::path::PathBuf,
}

impl StaleBindingFixture {
    async fn new(name: &str) -> Self {
        let db = std::env::temp_dir().join(format!(
            "scryer_stale_binding_{name}_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("database should migrate through the canonical binding schema");
        sqlx::query(
            "INSERT INTO download_clients (
                id, name, client_type, config_json, created_at, updated_at
             ) VALUES (?1, 'Primary Client', ?2, '{}', ?3, ?3)",
        )
        .bind(CLIENT_ID)
        .bind(CLIENT_TYPE)
        .bind(Utc::now().to_rfc3339())
        .execute(services.pool())
        .await
        .expect("configured client should insert");
        Self { services, db }
    }

    fn submissions(&self) -> DownloadSubmissionStore {
        DownloadSubmissionStore::new(self.services.datastore())
    }

    fn registry(&self) -> DownloadRegistryStore {
        DownloadRegistryStore::new(self.services.datastore())
    }

    async fn binding_rows(&self) -> Vec<(String, Option<String>)> {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT download_id, ended_at FROM download_client_bindings
              WHERE native_item_id = ?1
              ORDER BY created_at, download_id",
        )
        .bind(ITEM_ID)
        .fetch_all(self.services.pool())
        .await
        .expect("binding rows should load")
    }

    async fn tracked_state_of(&self, download_id: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT tracked_state FROM download_submissions WHERE id = ?1",
        )
        .bind(download_id)
        .fetch_optional(self.services.pool())
        .await
        .expect("submission row should load")
        .flatten()
    }

    fn cleanup(self) {
        let db = self.db.clone();
        drop(self.services);
        let _ = std::fs::remove_file(db);
    }
}

fn locator() -> ClientJobLocator {
    ClientJobLocator::new(Some(CLIENT_ID), CLIENT_TYPE, ITEM_ID)
}

fn accepted_submission(
    download_id: scryer_domain::download_identity::DownloadId,
    title_id: &str,
) -> DownloadSubmission {
    DownloadSubmission {
        download_id,
        scope: SubmissionScope::Title,
        title_id: title_id.to_string(),
        facet: "series".to_string(),
        download_client_id: Some(CLIENT_ID.to_string()),
        download_client_type: CLIENT_TYPE.to_string(),
        download_client_item_id: ITEM_ID.to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: None,
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        purpose: DownloadSubmissionPurpose::Standard,
    }
}

async fn record_accepted_grab(
    submissions: &DownloadSubmissionStore,
    download_id: scryer_domain::download_identity::DownloadId,
    title_id: &str,
) {
    submissions
        .record_submission_with_identity(
            accepted_submission(download_id, title_id),
            DownloadSubmissionIdentity {
                download_id: Some(download_id.to_wire()),
            },
            None,
        )
        .await
        .expect("accepted submission should claim its client locator");
}

/// The first grab imported; its client job is gone but the binding row lagged
/// behind. The re-grab must keep the identity it preallocated.
#[tokio::test]
async fn accepted_grab_over_an_imported_binding_binds_the_requested_identity() {
    let fixture = StaleBindingFixture::new("imported").await;
    let submissions = fixture.submissions();
    let registry = fixture.registry();
    let locator = locator();

    let imported_download_id = scryer_domain::download_identity::DownloadId::new();
    record_accepted_grab(&submissions, imported_download_id, "title-one").await;
    submissions
        .update_tracked_state(&locator, "imported")
        .await
        .expect("terminal tracked state should persist on the bound download");

    let regrab_download_id = scryer_domain::download_identity::DownloadId::new();
    record_accepted_grab(&submissions, regrab_download_id, "title-one").await;

    assert_eq!(
        registry
            .find_active_binding_by_locator(&locator)
            .await
            .expect("active binding lookup should succeed")
            .expect("the re-grab should own an active binding")
            .download_id,
        regrab_download_id,
        "the locator must resolve to the re-grab, not the imported download"
    );
    assert_eq!(
        submissions
            .find_by_client_item_id_for_download(None, &locator)
            .await
            .expect("submission lookup should succeed")
            .expect("the re-grab submission should exist")
            .download_id,
        regrab_download_id
    );
    assert!(
        registry
            .load_binding(&imported_download_id)
            .await
            .expect("stale binding should load")
            .expect("stale binding row should still exist")
            .ended_at
            .is_some(),
        "the stale binding must be ended, not deleted"
    );
    assert_eq!(
        fixture
            .tracked_state_of(&imported_download_id.to_string())
            .await,
        Some("imported".to_string()),
        "the imported download keeps its own history"
    );
    assert_eq!(
        fixture
            .tracked_state_of(&regrab_download_id.to_string())
            .await,
        None,
        "the fresh identity starts without inherited tracked state"
    );
    assert_eq!(
        registry
            .load_download(&regrab_download_id)
            .await
            .expect("re-grab parent should load")
            .expect("re-grab parent should exist")
            .origin,
        scryer_application::DownloadOrigin::ScryerSubmission
    );

    let bindings = fixture.binding_rows().await;
    assert_eq!(bindings.len(), 2, "each identity owns its own binding row");
    assert_eq!(
        bindings
            .iter()
            .filter(|(_, ended_at)| ended_at.is_none())
            .count(),
        1,
        "exactly one binding stays active for the locator"
    );

    fixture.cleanup();
}

/// Same guard, driven from the durable identity-state row (what the tracking
/// layer reads first) rather than the submission's tracked_state column.
#[tokio::test]
async fn accepted_grab_over_a_failed_binding_binds_the_requested_identity() {
    let fixture = StaleBindingFixture::new("failed").await;
    let submissions = fixture.submissions();
    let registry = fixture.registry();
    let locator = locator();

    let failed_download_id = scryer_domain::download_identity::DownloadId::new();
    record_accepted_grab(&submissions, failed_download_id, "title-one").await;
    submissions
        .record_identity_tracked_state_for_download(
            Some(&failed_download_id),
            &DownloadSubmissionIdentity {
                download_id: Some(failed_download_id.to_wire()),
            },
            Some(&locator),
            "failed",
            Some("import_gate_rejected"),
            None,
        )
        .await
        .expect("terminal identity state should persist");

    let regrab_download_id = scryer_domain::download_identity::DownloadId::new();
    record_accepted_grab(&submissions, regrab_download_id, "title-one").await;

    assert_eq!(
        registry
            .find_active_binding_by_locator(&locator)
            .await
            .expect("active binding lookup should succeed")
            .expect("the re-grab should own an active binding")
            .download_id,
        regrab_download_id
    );
    assert!(
        registry
            .load_binding(&failed_download_id)
            .await
            .expect("stale binding should load")
            .expect("stale binding row should still exist")
            .ended_at
            .is_some()
    );
    assert_eq!(
        submissions
            .get_identity_tracked_state_for_download(
                Some(&failed_download_id),
                &DownloadSubmissionIdentity {
                    download_id: Some(failed_download_id.to_wire()),
                },
                Some(&locator),
            )
            .await
            .expect("failed identity state should load"),
        Some("failed".to_string()),
        "the failed download keeps its own history"
    );

    fixture.cleanup();
}

/// Live jobs are the reason adoption exists: a dedup-by-hash client hands back
/// the same native job, and the accepted grab must take over that identity and
/// upgrade its parent.
#[tokio::test]
async fn accepted_grab_over_a_live_binding_still_adopts_and_upgrades_the_origin() {
    let fixture = StaleBindingFixture::new("live").await;
    let submissions = fixture.submissions();
    let registry = fixture.registry();
    let locator = locator();

    let live_download_id = scryer_domain::download_identity::DownloadId::new();
    record_accepted_grab(&submissions, live_download_id, "title-one").await;
    for state in ["downloading", "import_pending", "import_blocked"] {
        submissions
            .update_tracked_state(&locator, state)
            .await
            .expect("non-terminal tracked state should persist on the bound download");

        let requested_download_id = scryer_domain::download_identity::DownloadId::new();
        record_accepted_grab(&submissions, requested_download_id, "title-one").await;

        assert_eq!(
            registry
                .find_active_binding_by_locator(&locator)
                .await
                .expect("active binding lookup should succeed")
                .expect("the live binding should stay active")
                .download_id,
            live_download_id,
            "a {state} download is live; its binding must still be adopted"
        );
        assert!(
            registry
                .load_binding(&requested_download_id)
                .await
                .expect("requested binding lookup should succeed")
                .is_none(),
            "an adopted claim must not mint a second binding"
        );
    }
    assert_eq!(
        registry
            .load_download(&live_download_id)
            .await
            .expect("adopted parent should load")
            .expect("adopted parent should exist")
            .origin,
        scryer_application::DownloadOrigin::ScryerSubmission
    );

    let bindings = fixture.binding_rows().await;
    assert_eq!(bindings.len(), 1);
    assert!(bindings[0].1.is_none(), "the live binding stays active");

    fixture.cleanup();
}

/// Re-claiming the identity that already owns the binding is not a stale
/// adopt, even once that download is terminal: there is no other history to
/// inherit and the binding is its own.
#[tokio::test]
async fn reclaiming_the_bound_identity_keeps_its_binding_after_a_terminal_state() {
    let fixture = StaleBindingFixture::new("reclaim").await;
    let submissions = fixture.submissions();
    let registry = fixture.registry();
    let locator = locator();

    let download_id = scryer_domain::download_identity::DownloadId::new();
    record_accepted_grab(&submissions, download_id, "title-one").await;
    submissions
        .update_tracked_state(&locator, "imported")
        .await
        .expect("terminal tracked state should persist on the bound download");
    record_accepted_grab(&submissions, download_id, "title-two").await;

    assert_eq!(
        registry
            .find_active_binding_by_locator(&locator)
            .await
            .expect("active binding lookup should succeed")
            .expect("the binding should stay active")
            .download_id,
        download_id
    );
    let bindings = fixture.binding_rows().await;
    assert_eq!(bindings.len(), 1);
    assert!(bindings[0].1.is_none());

    fixture.cleanup();
}

/// The tracked-state stub path has no preallocated identity, so a terminal
/// *foreign* binding is retired and the state lands on a freshly minted
/// download: the client re-added a job under a native id whose previous
/// occupant is finished.
#[tokio::test]
async fn tracked_state_stub_over_a_terminal_foreign_binding_mints_a_fresh_identity() {
    let fixture = StaleBindingFixture::new("stub_foreign").await;
    let submissions = fixture.submissions();
    let registry = fixture.registry();
    let locator = locator();

    submissions
        .update_tracked_state(&locator, "failed")
        .await
        .expect("the stub writer should mint a foreign identity for an unbound locator");
    let failed_download_id = registry
        .find_active_binding_by_locator(&locator)
        .await
        .expect("active binding lookup should succeed")
        .expect("the stub writer should have bound a foreign identity")
        .download_id;
    assert_eq!(
        registry
            .load_download(&failed_download_id)
            .await
            .expect("foreign parent should load")
            .expect("foreign parent should exist")
            .origin,
        scryer_application::DownloadOrigin::ForeignObservation
    );

    submissions
        .update_tracked_state(&locator, "downloading")
        .await
        .expect("the stub writer should claim a fresh identity");

    let active = registry
        .find_active_binding_by_locator(&locator)
        .await
        .expect("active binding lookup should succeed")
        .expect("the fresh identity should own an active binding");
    assert_ne!(
        active.download_id, failed_download_id,
        "the failed download must not be reused by the stub writer"
    );
    assert!(
        registry
            .load_binding(&failed_download_id)
            .await
            .expect("stale binding should load")
            .expect("stale binding row should still exist")
            .ended_at
            .is_some()
    );
    assert_eq!(
        fixture
            .tracked_state_of(&failed_download_id.to_string())
            .await,
        Some("failed".to_string()),
        "the failed download keeps its own history"
    );
    assert_eq!(
        submissions
            .get_tracked_state(&locator)
            .await
            .expect("tracked state should load through the active binding"),
        Some("downloading".to_string())
    );
    assert_eq!(
        registry
            .load_download(&active.download_id)
            .await
            .expect("stub parent should load")
            .expect("stub parent should exist")
            .origin,
        scryer_application::DownloadOrigin::ForeignObservation
    );

    fixture.cleanup();
}

/// The stub writer is also what *records* terminal states, so a duplicate
/// terminal write for a Scryer-owned job that is still in the client must keep
/// the submission identity: minting a fresh one would detach the entry from
/// its title and seed goals. Scryer-owned bindings end through the
/// queue-delete / authoritative-absence lifecycle instead.
#[tokio::test]
async fn tracked_state_stub_over_a_terminal_scryer_binding_keeps_the_submission_identity() {
    let fixture = StaleBindingFixture::new("stub_scryer").await;
    let submissions = fixture.submissions();
    let registry = fixture.registry();
    let locator = locator();

    let download_id = scryer_domain::download_identity::DownloadId::new();
    record_accepted_grab(&submissions, download_id, "title-one").await;
    submissions
        .update_tracked_state(&locator, "imported")
        .await
        .expect("terminal tracked state should persist on the bound download");

    submissions
        .update_tracked_state(&locator, "imported")
        .await
        .expect("a duplicate terminal write should reuse the submission identity");

    assert_eq!(
        registry
            .find_active_binding_by_locator(&locator)
            .await
            .expect("active binding lookup should succeed")
            .expect("the submission binding should stay active")
            .download_id,
        download_id
    );
    let bindings = fixture.binding_rows().await;
    assert_eq!(bindings.len(), 1, "no second binding row is minted");
    assert!(bindings[0].1.is_none(), "the binding is not ended");
    let download_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM downloads")
        .fetch_one(fixture.services.pool())
        .await
        .expect("download row count should load");
    assert_eq!(download_rows, 1, "no second downloads row is minted");
    assert_eq!(
        submissions
            .get_tracked_state(&locator)
            .await
            .expect("tracked state should load through the active binding"),
        Some("imported".to_string())
    );

    fixture.cleanup();
}
