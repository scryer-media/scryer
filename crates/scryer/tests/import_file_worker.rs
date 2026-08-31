use std::io::{Read, Write};
use std::process::{Command, Stdio};

fn run_worker(request: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_scryer"))
        .arg("__import-file-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start hidden import file worker");
    let mut control = child.stdin.take().expect("worker stdin");
    writeln!(
        control,
        "{}",
        serde_json::to_string(request).expect("request JSON")
    )
    .expect("write worker request");
    control.flush().expect("flush worker request");

    let mut stdout = child.stdout.take().expect("worker stdout");
    let mut stderr = child.stderr.take().expect("worker stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).expect("read worker stdout");
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).expect("read worker stderr");
        bytes
    });
    let status = child.wait().expect("wait for worker");
    drop(control);
    let stdout = stdout_reader.join().expect("join stdout reader");
    let stderr = stderr_reader.join().expect("join stderr reader");

    assert!(
        status.success(),
        "worker failed: {}",
        String::from_utf8_lossy(&stderr)
    );
    String::from_utf8(stdout)
        .expect("worker output UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("worker event JSON"))
        .collect()
}

#[test]
fn hidden_import_file_worker_snapshots_a_source_over_ndjson() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let events = run_worker(&serde_json::json!({
        "command": "snapshot",
        "version": 1,
        "nonce": 42,
        "source": source,
    }));
    assert!(events.iter().any(|event| {
        event.get("event").and_then(|value| value.as_str()) == Some("snapshot_finished")
            && event.get("nonce").and_then(|value| value.as_u64()) == Some(42)
    }));
}

#[test]
fn hidden_import_file_worker_reuses_prepared_state_for_copy() {
    let temp = tempfile::tempdir().expect("temp directory");
    let source = temp.path().join("source.mkv");
    let destination = temp.path().join("library").join("destination.mkv");
    std::fs::write(&source, b"prepared-copy-payload").expect("write source");

    let prepared_events = run_worker(&serde_json::json!({
        "command": "prepare",
        "version": 1,
        "nonce": 51,
        "source": source,
        "dest": destination,
        "mode": "hardlink_or_copy",
        "expected_source": null,
        "permissions": {
            "set_permissions_linux": false,
            "file_chmod": null,
            "folder_chmod": null,
            "chown_group": null
        }
    }));
    let prepared = prepared_events
        .iter()
        .find(|event| event.get("event").and_then(|value| value.as_str()) == Some("prepared"))
        .and_then(|event| event.get("prepared"))
        .cloned()
        .expect("prepared worker event");

    let copy_events = run_worker(&serde_json::json!({
        "command": "copy",
        "version": 1,
        "nonce": 52,
        "prepared": prepared,
    }));
    assert!(copy_events.iter().any(|event| {
        event.get("event").and_then(|value| value.as_str()) == Some("import_finished")
            && event.get("nonce").and_then(|value| value.as_u64()) == Some(52)
    }));
    assert_eq!(
        std::fs::read(destination).expect("read destination"),
        b"prepared-copy-payload"
    );
}
