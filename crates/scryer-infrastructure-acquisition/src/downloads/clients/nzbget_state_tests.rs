use super::*;
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub(super) fn history_entry(overrides: Value) -> Value {
    let mut entry = json!({
        "NZBID":42, "Name":"Synthetic.Release", "Status":"SUCCESS/ALL",
        "HistoryTime":Utc::now().timestamp(), "FinalDir":"/downloads/job",
        "DestDir":"/downloads/job", "ParStatus":"SUCCESS", "UnpackStatus":"SUCCESS",
        "MoveStatus":"SUCCESS", "ScriptStatus":"SUCCESS", "DeleteStatus":"NONE",
        "MarkStatus":"NONE", "UrlStatus":"NONE", "Parameters":[]
    });
    entry
        .as_object_mut()
        .unwrap()
        .extend(overrides.as_object().unwrap().clone());
    entry
}

async fn mount_rpc(server: &MockServer, method_name: &str, result: Value) {
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":method_name})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":result})))
        .mount(server)
        .await;
}

// All history statuses emitted by NzbInfo::MakeTextStatus, plus native
// mark overrides and ordered mixed-stage failures from Sonarr's GetHistory.
fn history_cases() -> Vec<(&'static str, Value, Option<DownloadQueueState>)> {
    use DownloadQueueState::{Completed as C, Failed as F, Warning as W};
    vec![
        ("all", json!({"Status":"SUCCESS/ALL"}), Some(C)),
        (
            "unpack",
            json!({"Status":"SUCCESS/UNPACK","ScriptStatus":"NONE"}),
            Some(C),
        ),
        (
            "par",
            json!({"Status":"SUCCESS/PAR","UnpackStatus":"NONE","ScriptStatus":"NONE"}),
            Some(C),
        ),
        (
            "health",
            json!({"Status":"SUCCESS/HEALTH","ParStatus":"NONE","UnpackStatus":"NONE","ScriptStatus":"NONE"}),
            Some(C),
        ),
        (
            "marked good",
            json!({"Status":"SUCCESS/GOOD","MarkStatus":"GOOD"}),
            Some(C),
        ),
        (
            "marked success",
            json!({"Status":"SUCCESS/MARK","MarkStatus":"SUCCESS"}),
            Some(C),
        ),
        (
            "par failure",
            json!({"Status":"FAILURE/PAR","ParStatus":"FAILURE"}),
            Some(F),
        ),
        (
            "unpack failure",
            json!({"Status":"FAILURE/UNPACK","UnpackStatus":"FAILURE"}),
            Some(F),
        ),
        (
            "health failure",
            json!({"Status":"FAILURE/HEALTH","DeleteStatus":"HEALTH"}),
            Some(F),
        ),
        (
            "bad",
            json!({"Status":"FAILURE/BAD","MarkStatus":"BAD"}),
            Some(F),
        ),
        (
            "fetch",
            json!({"Status":"FAILURE/FETCH","UrlStatus":"FAILURE"}),
            Some(F),
        ),
        (
            "scan",
            json!({"Status":"FAILURE/SCAN","UrlStatus":"SCAN_FAILURE"}),
            Some(F),
        ),
        (
            "internal",
            json!({"Status":"FAILURE/INTERNAL_ERROR","UrlStatus":"UNKNOWN"}),
            Some(F),
        ),
        (
            "move",
            json!({"Status":"FAILURE/MOVE","MoveStatus":"FAILURE"}),
            Some(W),
        ),
        (
            "damaged",
            json!({"Status":"WARNING/DAMAGED","ParStatus":"MANUAL"}),
            Some(F),
        ),
        (
            "repairable",
            json!({"Status":"WARNING/REPAIRABLE","ParStatus":"REPAIR_POSSIBLE"}),
            Some(F),
        ),
        (
            "health warning",
            json!({"Status":"WARNING/HEALTH","ParStatus":"NONE","UnpackStatus":"NONE","ScriptStatus":"NONE"}),
            Some(C),
        ),
        (
            "space",
            json!({"Status":"WARNING/SPACE","UnpackStatus":"SPACE"}),
            Some(W),
        ),
        (
            "password",
            json!({"Status":"WARNING/PASSWORD","UnpackStatus":"PASSWORD"}),
            Some(F),
        ),
        (
            "script",
            json!({"Status":"WARNING/SCRIPT","ScriptStatus":"FAILURE"}),
            Some(F),
        ),
        (
            "skipped",
            json!({"Status":"WARNING/SKIPPED","UrlStatus":"SCAN_SKIPPED"}),
            Some(W),
        ),
        (
            "manual",
            json!({"Status":"DELETED/MANUAL","DeleteStatus":"MANUAL"}),
            None,
        ),
        (
            "dupe",
            json!({"Status":"DELETED/DUPE","DeleteStatus":"DUPE"}),
            Some(F),
        ),
        (
            "copy",
            json!({"Status":"DELETED/COPY","DeleteStatus":"COPY"}),
            Some(F),
        ),
        (
            "deleted good",
            json!({"Status":"DELETED/GOOD","DeleteStatus":"GOOD"}),
            Some(W),
        ),
        (
            "manual bad",
            json!({"Status":"FAILURE/BAD","DeleteStatus":"MANUAL","MarkStatus":"BAD"}),
            Some(F),
        ),
        (
            "manual good failed stages",
            json!({"Status":"SUCCESS/GOOD","DeleteStatus":"MANUAL","MarkStatus":"GOOD","ParStatus":"FAILURE","UnpackStatus":"FAILURE"}),
            None,
        ),
        (
            "marked good failed stages",
            json!({"Status":"SUCCESS/GOOD","MarkStatus":"GOOD","ParStatus":"FAILURE","UnpackStatus":"FAILURE"}),
            Some(F),
        ),
        (
            "par then space",
            json!({"Status":"FAILURE/PAR","ParStatus":"FAILURE","UnpackStatus":"SPACE"}),
            Some(W),
        ),
        (
            "unpack then move",
            json!({"Status":"FAILURE/UNPACK","UnpackStatus":"FAILURE","MoveStatus":"FAILURE"}),
            Some(W),
        ),
        (
            "move then script",
            json!({"Status":"FAILURE/MOVE","MoveStatus":"FAILURE","ScriptStatus":"FAILURE"}),
            Some(F),
        ),
        (
            "script then deleted good",
            json!({"Status":"DELETED/GOOD","DeleteStatus":"GOOD","ScriptStatus":"FAILURE"}),
            Some(W),
        ),
        (
            "password then script",
            json!({"Status":"WARNING/PASSWORD","UnpackStatus":"PASSWORD","ScriptStatus":"FAILURE"}),
            Some(F),
        ),
    ]
}

#[tokio::test]
async fn native_history_matrix_agrees_across_all_reads_and_cleanup_admission() {
    for (label, overrides, expected) in history_cases() {
        let entry = history_entry(overrides);
        let server = MockServer::start().await;
        mount_rpc(&server, "history", json!([entry])).await;
        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
        let rows = client.list_history().await.unwrap();
        assert_eq!(rows.first().map(|row| row.state), expected, "{label}");
        if let Some(row) = rows.first() {
            assert_eq!(
                row.attention_required,
                row.state != DownloadQueueState::Completed,
                "{label}"
            );
            if row.attention_required {
                assert!(row.attention_reason.is_some(), "{label}");
            }
        }
        let importable = expected == Some(DownloadQueueState::Completed);
        assert_eq!(
            !client.list_completed_downloads().await.unwrap().is_empty(),
            importable,
            "{label}"
        );
        assert_eq!(
            client
                .get_completed_download_for_source("client", "nzbget", "42")
                .await
                .unwrap()
                .is_some(),
            importable,
            "{label}"
        );
        let (recent, completed) = client
            .list_recent_activity_with_completed_with_feedback_scope(10, &Default::default())
            .await
            .unwrap();
        assert_eq!(recent.first().map(|row| row.state), expected, "{label}");
        assert_eq!(!completed.unwrap().is_empty(), importable, "{label}");
        let password = entry["UnpackStatus"] == "PASSWORD";
        if password {
            assert!(rows[0].password_failure().is_some(), "{label}");
        }
        let cleanup = client
            .get_cleanup_payload_for_source("client", "nzbget", "42")
            .await;
        if password || expected.is_none() || expected == Some(DownloadQueueState::Warning) {
            assert!(
                cleanup.is_err(),
                "retained job must block stale cleanup: {label}"
            );
        } else {
            assert!(cleanup.unwrap().is_some(), "{label}");
        }
    }
}

#[tokio::test]
async fn malformed_history_never_proves_completion_cleanup_or_absence() {
    // Structural malformation: the listing itself cannot be trusted.
    let mut structural = vec![
        json!(false),
        json!([{"Status":"SUCCESS/ALL"}]),
        json!([history_entry(json!({})), 1]),
    ];
    for overrides in [
        json!({"NZBID":0}),
        json!({"NZBID":42.5}),
        json!({"nzbId":43}),
    ] {
        structural.push(json!([history_entry(overrides)]));
    }
    structural.push(json!([
        history_entry(json!({})),
        history_entry(json!({"Status":"FAILURE/PAR","ParStatus":"FAILURE"}))
    ]));
    // Per-entry state problems: only that entry degrades to a retained Warning.
    let mut per_entry = Vec::new();
    for overrides in [
        json!({"Status":""}),
        json!({"Status":null}),
        json!({"Status":"SUCCESSFUL_BUT_NOT_A_REAL_STATE"}),
        json!({"Status":"WARNING/FUTURE"}),
        json!({"status":"FAILURE/PAR"}),
        json!({"unpackStatus":"FAILURE"}),
        json!({"Status":"SUCCESS/ALL","MarkStatus":"BAD"}),
        json!({"Status":"SUCCESS/ALL","UrlStatus":"FAILURE"}),
        json!({"Status":"SUCCESS/ALL","UnpackStatus":"SPACE"}),
        json!({"Status":"DELETED/MANUAL","DeleteStatus":"NONE"}),
        json!({"Status":"WARNING/SCRIPT","ScriptStatus":"SUCCESS"}),
        json!({"ParStatus":"FUTURE"}),
    ] {
        per_entry.push(history_entry(overrides));
    }
    for field in [
        "Status",
        "ParStatus",
        "UnpackStatus",
        "MoveStatus",
        "ScriptStatus",
        "DeleteStatus",
        "MarkStatus",
        "UrlStatus",
    ] {
        let mut entry = history_entry(json!({}));
        entry.as_object_mut().unwrap().remove(field);
        per_entry.push(entry);
    }
    let locator = scryer_application::ClientJobLocator::new(Some("client"), "nzbget", "42");
    for entry in per_entry {
        // A healthy neighbour must still list and import.
        let result = json!([entry, history_entry(json!({"NZBID":41}))]);
        let server = MockServer::start().await;
        mount_rpc(&server, "history", result.clone()).await;
        mount_rpc(&server, "listgroups", json!([])).await;
        mount_rpc(&server, "postqueue", json!([])).await;
        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
        let assert_retained = |row: &DownloadQueueItem| {
            assert_eq!(row.state, DownloadQueueState::Warning, "{result}");
            assert!(row.attention_required, "{result}");
            let reason = row.attention_reason.as_deref().unwrap_or("");
            assert!(
                reason.contains("NZBGet") && !reason.starts_with("repository:"),
                "{result}: {reason}"
            );
        };
        let rows = client.list_history().await.unwrap();
        assert_eq!(rows.len(), 2, "{result}");
        assert_retained(rows.iter().find(|row| row.id == "42").unwrap());
        assert_eq!(
            rows.iter().find(|row| row.id == "41").unwrap().state,
            DownloadQueueState::Completed,
            "{result}"
        );
        match client.observe_download(&locator, 0).await.unwrap() {
            scryer_application::DownloadClientObservation::Present(row) => assert_retained(&row),
            _ => panic!("retained entry must be observed present: {result}"),
        }
        let completed = client.list_completed_downloads().await.unwrap();
        assert_eq!(
            completed
                .iter()
                .map(|download| download.download_client_item_id.as_str())
                .collect::<Vec<_>>(),
            vec!["41"],
            "{result}"
        );
        assert!(
            client
                .get_completed_download_for_source("client", "nzbget", "42")
                .await
                .unwrap()
                .is_none(),
            "{result}"
        );
        assert!(
            client
                .get_cleanup_payload_for_source("client", "nzbget", "42")
                .await
                .is_err(),
            "{result}"
        );
        let (recent, completed) = client
            .list_recent_activity_with_completed_with_feedback_scope(10, &Default::default())
            .await
            .unwrap();
        assert_retained(recent.iter().find(|row| row.id == "42").unwrap());
        assert!(
            completed
                .unwrap()
                .iter()
                .all(|download| download.download_client_item_id != "42"),
            "{result}"
        );
    }
    for result in structural {
        let server = MockServer::start().await;
        mount_rpc(&server, "history", result.clone()).await;
        mount_rpc(&server, "listgroups", json!([])).await;
        mount_rpc(&server, "postqueue", json!([])).await;
        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
        assert!(client.list_history().await.is_err(), "{result}");
        assert!(
            client
                .observe_download(
                    &scryer_application::ClientJobLocator::new(Some("client"), "nzbget", "42"),
                    0
                )
                .await
                .is_err(),
            "{result}"
        );
        assert!(client.list_completed_downloads().await.is_err(), "{result}");
        assert!(
            client
                .get_completed_download_for_source("client", "nzbget", "42")
                .await
                .is_err(),
            "{result}"
        );
        assert!(
            client
                .get_cleanup_payload_for_source("client", "nzbget", "42")
                .await
                .is_err(),
            "{result}"
        );
        assert!(
            client
                .list_recent_activity_with_completed_with_feedback_scope(1, &Default::default())
                .await
                .is_err(),
            "{result}"
        );
    }
}

const QUEUE_STATES: &[&str] = &[
    "QUEUED",
    "PAUSED",
    "DOWNLOADING",
    "FETCHING",
    "PP_QUEUED",
    "LOADING_PARS",
    "VERIFYING_SOURCES",
    "REPAIRING",
    "VERIFYING_REPAIRED",
    "RENAMING",
    "UNPACKING",
    "MOVING",
    "POST_UNPACK_RENAMING",
    "EXECUTING_SCRIPT",
    "PP_FINISHED",
    "POST_DOWNLOAD_RENAMING",
    "QS_EXECUTING",
    "QS_QUEUED",
];

#[tokio::test]
async fn every_native_queue_phase_survives_merge_and_independent_pause_controls() {
    for merge in [false, true] {
        for (download_paused, post_paused, priority) in [
            (false, false, 0),
            (true, false, 0),
            (false, true, 0),
            (true, true, 0),
            (true, true, 900),
        ] {
            let server = MockServer::start().await;
            let groups:Vec<_>=QUEUE_STATES.iter().enumerate().map(|(i,status)|json!({
                "NZBID":i+1,"Status":status,"MaxPriority":priority,"FileSizeMB":10,
                "RemainingSizeMB":0,"PostInfoText":"UNPACKING stale text","PostStageProgress":300
            })).collect();
            let post: Vec<_> = if merge {
                QUEUE_STATES.iter().enumerate().skip(4).map(|(i,status)|json!({
                "NZBID":i+1,"Stage":match *status { "PP_QUEUED"|"QS_EXECUTING"|"QS_QUEUED"=>"QUEUED","PP_FINISHED"=>"FINISHED",other=>other },
                "StageProgress":300
            })).collect()
            } else {
                vec![]
            };
            mount_rpc(&server, "listgroups", json!(groups)).await;
            mount_rpc(&server, "postqueue", json!(post)).await;
            mount_rpc(
                &server,
                "status",
                json!({"DownloadPaused":download_paused,"PostPaused":post_paused}),
            )
            .await;
            let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
            let rows = client.list_queue().await.unwrap();
            assert_eq!(rows.len(), QUEUE_STATES.len());
            for (i, row) in rows.iter().enumerate() {
                let status = QUEUE_STATES[i];
                let pp = i >= 4;
                let expected = if status == "PAUSED"
                    || (priority < 900
                        && if pp {
                            post_paused
                                && matches!(
                                    status,
                                    "PP_QUEUED"
                                        | "LOADING_PARS"
                                        | "VERIFYING_SOURCES"
                                        | "REPAIRING"
                                        | "VERIFYING_REPAIRED"
                                        | "EXECUTING_SCRIPT"
                                )
                        } else {
                            download_paused && status != "FETCHING"
                        }) {
                    DownloadQueueState::Paused
                } else if status == "QUEUED" {
                    DownloadQueueState::Queued
                } else {
                    DownloadQueueState::Downloading
                };
                assert_eq!(
                    row.state, expected,
                    "{status} merge={merge} download={download_paused} post={post_paused} priority={priority}"
                );
                assert_eq!(
                    row.attention_reason.as_deref(),
                    pp.then_some(status),
                    "{status}"
                );
                assert_eq!(row.progress_percent, if pp { 30 } else { 100 }, "{status}");
            }
        }
    }
}

async fn mount_rpc_error(server: &MockServer, method_name: &str) {
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":method_name})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"error":{"code":-32601,"message":"synthetic rpc failure"}})),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn malformed_queue_and_postqueue_are_not_authoritative_empty_snapshots() {
    // Structural listgroups malformation still fails the listing.
    for groups in [
        json!(false),
        json!([1]),
        json!([{"Status":"QUEUED"}]),
        json!([{"NZBID":1,"Status":"QUEUED"},{"NZBID":1,"Status":"DOWNLOADING"}]),
    ] {
        let server = MockServer::start().await;
        mount_rpc(&server, "listgroups", groups.clone()).await;
        mount_rpc(&server, "postqueue", json!([])).await;
        mount_rpc(
            &server,
            "status",
            json!({"DownloadPaused":false,"PostPaused":false}),
        )
        .await;
        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
        assert!(client.list_queue().await.is_err(), "{groups}");
    }

    // A per-entry state problem retains only that row as Warning, even when
    // the postqueue claims a recognised stage for it.
    for (bad, post) in [
        (json!({"NZBID":1}), json!([])),
        (json!({"NZBID":1,"Status":"FUTURE"}), json!([])),
        (
            json!({"NZBID":1,"Status":"QUEUED","status":"DOWNLOADING"}),
            json!([]),
        ),
        (
            json!({"NZBID":1,"Status":"FUTURE"}),
            json!([{"NZBID":1,"Stage":"UNPACKING"}]),
        ),
    ] {
        let server = MockServer::start().await;
        mount_rpc(
            &server,
            "listgroups",
            json!([bad, {"NZBID":2,"Status":"QUEUED"}]),
        )
        .await;
        mount_rpc(&server, "postqueue", post.clone()).await;
        mount_rpc(
            &server,
            "status",
            json!({"DownloadPaused":false,"PostPaused":false}),
        )
        .await;
        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
        let rows = client.list_queue().await.unwrap();
        assert_eq!(rows.len(), 2, "{bad}");
        let row = rows.iter().find(|row| row.id == "1").unwrap();
        assert_eq!(row.state, DownloadQueueState::Warning, "{bad} {post}");
        assert!(row.attention_required, "{bad}");
        assert!(
            row.attention_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("NZBGet")),
            "{bad}"
        );
        let healthy = rows.iter().find(|row| row.id == "2").unwrap();
        assert_eq!(healthy.state, DownloadQueueState::Queued, "{bad}");
        assert!(!healthy.attention_required, "{bad}");
    }

    // A failed or malformed postqueue only skips that enrichment; it never
    // turns the queue into an error or an empty snapshot.
    for post in [
        None,
        Some(json!(false)),
        Some(json!([{"Stage":"UNPACKING"}])),
        Some(json!([{"NZBID":1,"Stage":"QUEUED"},{"NZBID":1,"Stage":"UNPACKING"}])),
        Some(json!([{"NZBID":1,"Stage":"FUTURE"}])),
    ] {
        let server = MockServer::start().await;
        mount_rpc(
            &server,
            "listgroups",
            json!([{"NZBID":1,"Status":"DOWNLOADING"}]),
        )
        .await;
        match &post {
            Some(post) => mount_rpc(&server, "postqueue", post.clone()).await,
            None => mount_rpc_error(&server, "postqueue").await,
        }
        mount_rpc(
            &server,
            "status",
            json!({"DownloadPaused":false,"PostPaused":false}),
        )
        .await;
        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
        let rows = client.list_queue().await.unwrap();
        assert_eq!(rows.len(), 1, "{post:?}");
        assert_eq!(rows[0].state, DownloadQueueState::Downloading, "{post:?}");
        assert_eq!(rows[0].attention_reason, None, "{post:?}");
    }

    // One unreadable postqueue entry is skipped; its readable neighbour merges.
    let server = MockServer::start().await;
    mount_rpc(&server, "listgroups", json!([])).await;
    mount_rpc(
        &server,
        "postqueue",
        json!([{"NZBID":3,"Stage":"FUTURE"},{"NZBID":4,"Stage":"UNPACKING"}]),
    )
    .await;
    mount_rpc(
        &server,
        "status",
        json!({"DownloadPaused":false,"PostPaused":false}),
    )
    .await;
    let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
    let rows = client.list_queue().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "4");
    assert_eq!(rows[0].attention_reason.as_deref(), Some("UNPACKING"));
}

#[tokio::test]
async fn contradictory_queue_history_and_unreadable_pause_state_remain_unknown() {
    let server = MockServer::start().await;
    mount_rpc(
        &server,
        "listgroups",
        json!([{"NZBID":42,"Status":"DOWNLOADING"}]),
    )
    .await;
    mount_rpc(&server, "postqueue", json!([])).await;
    mount_rpc(
        &server,
        "status",
        json!({"DownloadPaused":false,"PostPaused":false}),
    )
    .await;
    mount_rpc(&server, "history", json!([history_entry(json!({}))])).await;
    let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
    assert!(
        client
            .observe_download(
                &scryer_application::ClientJobLocator::new(Some("client"), "nzbget", "42"),
                0
            )
            .await
            .is_err()
    );
    // An unreadable pause control skips only that detection: the readable
    // control still applies and the queue still lists.
    for (status, expected) in [
        (
            Some(json!({"DownloadPaused":true})),
            DownloadQueueState::Paused,
        ),
        (
            Some(json!({"PostPaused":true})),
            DownloadQueueState::Downloading,
        ),
        (Some(json!(false)), DownloadQueueState::Downloading),
        (None, DownloadQueueState::Downloading),
    ] {
        let server = MockServer::start().await;
        mount_rpc(
            &server,
            "listgroups",
            json!([{"NZBID":42,"Status":"DOWNLOADING"}]),
        )
        .await;
        mount_rpc(&server, "postqueue", json!([])).await;
        match &status {
            Some(status) => mount_rpc(&server, "status", status.clone()).await,
            None => mount_rpc_error(&server, "status").await,
        }
        let client = NzbgetDownloadClient::new(server.uri(), None, None, "SCORE".into());
        let rows = client.list_queue().await.unwrap();
        assert_eq!(rows.len(), 1, "{status:?}");
        assert_eq!(rows[0].state, expected, "{status:?}");
    }
}
