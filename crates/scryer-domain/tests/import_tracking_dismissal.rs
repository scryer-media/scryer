use scryer_domain::TrackedDownloadState;

#[test]
fn dismissed_import_tracking_survives_restart_without_claiming_an_import() {
    let state = TrackedDownloadState::from_str_opt("import_dismissed")
        .expect("a persisted dismissal must be understood after restart");
    assert!(state.is_terminal());
    assert!(state.is_import_settled());
    assert!(!state.counts_as_imported());
    assert_eq!(state.as_str(), "import_dismissed");
    let encoded = serde_json::to_string(&state).unwrap();
    let restored: TrackedDownloadState = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored, state);
}
