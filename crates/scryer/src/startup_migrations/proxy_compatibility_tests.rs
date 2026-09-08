use super::*;
use scryer_infrastructure_acquisition::proxy_config_store::ProxyConfigStore;
use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};
use scryer_infrastructure_sql::runtime::SqlArg;

#[tokio::test]
async fn proxy_compatibility_retains_invalid_configs_assignments_and_retries_after_repair() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("proxy.db").to_string_lossy().into_owned();
    let services = SqliteServices::new_with_mode(path.clone(), MigrationMode::Apply)
        .await
        .unwrap();
    let datastore = services.datastore();
    for (id, timeout, enabled) in [
        ("valid", 60, true),
        ("invalid", 180, true),
        ("disabled", 60, false),
    ] {
        SqlRuntime::execute_write(&datastore, "seed_proxy",
            "INSERT INTO proxy_configs (id, name, provider_type, base_url, request_timeout_seconds, is_enabled)
             VALUES ({}, {}, 'http', 'http://proxy.test:8080', {}, {})",
            vec![SqlArg::Text(id.into()), SqlArg::Text(id.into()), SqlArg::I64(timeout), SqlArg::Bool(enabled)]).await.unwrap();
    }
    for proxy in ["valid", "invalid", "disabled", "missing", ""] {
        for (table, type_column, provider) in [
            ("indexers", "provider_type", "newznab"),
            ("download_clients", "client_type", "sabnzbd"),
        ] {
            SqlRuntime::execute_write(&datastore, "seed_assignment",
                &format!("INSERT INTO {table} (id, name, {type_column}, base_url, proxy_config_id, created_at, updated_at)
                 VALUES ({{}}, {{}}, {{}}, 'http://consumer.test', {{}}, {{}}, {{}})"),
                vec![SqlArg::Text(format!("{table}:{proxy}")), SqlArg::Text(proxy.into()), SqlArg::Text(provider.into()),
                    SqlArg::Text(proxy.into()), SqlArg::Timestamp(chrono::Utc::now()), SqlArg::Timestamp(chrono::Utc::now())]).await.unwrap();
        }
    }
    let proxies = Arc::new(ProxyConfigStore::new(
        datastore.clone(),
        services.encryption_key_state(),
    ));
    for _ in 0..2 {
        assert!(!migrate(&datastore, proxies.clone()).await.unwrap());
    }
    assert!(proxies.get_by_id("invalid").await.is_err());
    assert_eq!(
        proxies
            .get_for_edit("invalid")
            .await
            .unwrap()
            .unwrap()
            .request_timeout_seconds,
        180
    );
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT subject_id FROM application_compatibility_journal WHERE status = 'blocked'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 9); // One proxy, four assignments in each family.
    let assignments = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT proxy_config_id FROM download_clients ORDER BY id",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(assignments.len(), 5);
    assert!(
        assignments
            .iter()
            .any(|row| row.text("proxy_config_id").unwrap().is_empty())
    );
    // Explicit operator repairs; the startup migration never performs these.
    SqlRuntime::execute_write(
        &datastore,
        "repair_timeout",
        "UPDATE proxy_configs SET request_timeout_seconds = 60, is_enabled = {}",
        vec![SqlArg::Bool(true)],
    )
    .await
    .unwrap();
    for table in ["indexers", "download_clients"] {
        SqlRuntime::execute_write(&datastore, "repair_assignment",
            &format!("UPDATE {table} SET proxy_config_id = 'valid' WHERE proxy_config_id IN ('', 'missing')"), vec![]).await.unwrap();
    }
    drop(proxies);
    drop(datastore);
    drop(services);
    let services = SqliteServices::new_with_mode(path, MigrationMode::Apply)
        .await
        .unwrap();
    let datastore = services.datastore();
    let proxies = Arc::new(ProxyConfigStore::new(
        datastore.clone(),
        services.encryption_key_state(),
    ));
    assert!(migrate(&datastore, proxies).await.unwrap());
}
