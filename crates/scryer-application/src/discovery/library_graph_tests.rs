use super::*;
use chrono::TimeZone;

fn node(
    subject_key: &str,
    target_key: &str,
    cluster_id: i64,
    titles: &[&str],
) -> LibraryGraphNodeRecord {
    LibraryGraphNodeRecord {
        subject_key: subject_key.to_string(),
        target_key: target_key.to_string(),
        cluster_id,
        title_ids: titles.iter().map(|title| title.to_string()).collect(),
    }
}

fn edge(from_key: &str, to_key: &str, weight: f64) -> LibraryGraphEdgeRecord {
    LibraryGraphEdgeRecord {
        from_key: from_key.to_string(),
        to_key: to_key.to_string(),
        weight,
        kinds: vec!["semantic".to_string()],
    }
}

fn cluster(cluster_id: i64, medoid: &str, size: i64) -> LibraryGraphClusterRecord {
    LibraryGraphClusterRecord {
        cluster_id,
        label_key: None,
        label: format!("Cluster {cluster_id}"),
        medoid: medoid.to_string(),
        size,
    }
}

fn run(subject_count: i64, submitted_count: i64) -> LibraryGraphRunRecord {
    let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    LibraryGraphRunRecord {
        id: "library-graph-fixture".to_string(),
        facet: "movie".to_string(),
        axis: LIBRARY_GRAPH_AXIS.as_str().to_string(),
        status: LIBRARY_GRAPH_RUN_COMPLETE.to_string(),
        subject_count,
        submitted_count,
        generated_at: Some("2026-01-01T00:00:00Z".to_string()),
        error_code: None,
        error_text: None,
        retry_after_seconds: None,
        omitted_subject_keys: Vec::new(),
        started_at: at,
        completed_at: Some(at),
        created_at: at,
        updated_at: at,
    }
}

/// The viewer's visible titles, each named after its id.
fn visible_titles<'a>(title_ids: &[&'a str]) -> HashMap<&'a str, &'a str> {
    title_ids
        .iter()
        .map(|title_id| (*title_id, *title_id))
        .collect()
}

/// Four titles: a, b and c in one cluster with medoid b, d alone in another.
/// The viewer sees a and c (library one) but not b or d (library two).
fn fixture_snapshot() -> LibraryGraphSnapshotRecord {
    LibraryGraphSnapshotRecord {
        run: run(4, 4),
        nodes: vec![
            node("tmdb:movie:1", "tmdb:movie:1", 0, &["title-a"]),
            node("tmdb:movie:2", "tmdb:movie:2", 0, &["title-b"]),
            node("tmdb:movie:3", "tmdb:movie:3", 0, &["title-c"]),
            node("tmdb:movie:4", "tmdb:movie:4", 1, &["title-d"]),
        ],
        edges: vec![
            edge("tmdb:movie:1", "tmdb:movie:2", 0.9),
            edge("tmdb:movie:1", "tmdb:movie:3", 0.6),
            edge("tmdb:movie:2", "tmdb:movie:3", 0.8),
            edge("tmdb:movie:3", "tmdb:movie:4", 0.7),
        ],
        clusters: vec![cluster(0, "tmdb:movie:2", 3), cluster(1, "tmdb:movie:4", 1)],
    }
}

#[test]
fn visibility_pruning_never_leaks_an_edge_or_node_to_a_title_the_viewer_cannot_see() {
    let snapshot = fixture_snapshot();
    let visible = visible_titles(&["title-a", "title-c"]);
    let (nodes, edges, clusters) = prune_library_graph_for_viewer(&snapshot, &visible);

    let node_keys = nodes
        .iter()
        .map(|node| node.target_key.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(node_keys, HashSet::from(["tmdb:movie:1", "tmdb:movie:3"]));
    for edge in &edges {
        assert!(
            node_keys.contains(edge.from.as_str()),
            "edge leaks {}",
            edge.from
        );
        assert!(
            node_keys.contains(edge.to.as_str()),
            "edge leaks {}",
            edge.to
        );
    }
    assert_eq!(edges.len(), 1);
    for cluster in &clusters {
        assert!(node_keys.contains(cluster.medoid.as_str()), "medoid leaks");
    }
    // The hidden-only cluster is dropped; the shared one shrinks to the
    // visible members and gets a visible medoid.
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].id, 0);
    assert_eq!(clusters[0].size, 2);
    assert!(clusters.iter().all(|cluster| cluster.id != 1));
}

#[test]
fn hidden_medoid_is_replaced_by_heaviest_visible_member_then_subject_key() {
    let mut snapshot = fixture_snapshot();
    let visible = visible_titles(&["title-a", "title-c"]);
    // a and c share the only visible edge, so their weights tie: a wins on
    // subject key.
    let (_, _, clusters) = prune_library_graph_for_viewer(&snapshot, &visible);
    assert_eq!(clusters[0].medoid, "tmdb:movie:1");

    // With b visible, c carries the most visible weight (0.6 + 0.8) once the
    // medoid b is hidden by moving it to another title the viewer lacks.
    snapshot.nodes[1].title_ids = vec!["title-hidden".to_string()];
    snapshot
        .nodes
        .push(node("tmdb:movie:5", "tmdb:movie:5", 0, &["title-e"]));
    snapshot
        .edges
        .push(edge("tmdb:movie:3", "tmdb:movie:5", 0.9));
    let visible = visible_titles(&["title-a", "title-c", "title-e"]);
    let (_, _, clusters) = prune_library_graph_for_viewer(&snapshot, &visible);
    assert_eq!(clusters[0].medoid, "tmdb:movie:3");
    assert_eq!(clusters[0].size, 3);
}

#[test]
fn visible_medoid_and_unplaced_nodes_are_kept_as_stored() {
    let mut snapshot = fixture_snapshot();
    snapshot.nodes.push(node(
        "tmdb:movie:6",
        "tmdb:movie:6",
        UNPLACED_CLUSTER_ID,
        &["title-f"],
    ));
    let visible = visible_titles(&["title-a", "title-b", "title-c", "title-d", "title-f"]);
    let (nodes, edges, clusters) = prune_library_graph_for_viewer(&snapshot, &visible);
    assert_eq!(nodes.len(), 5);
    assert_eq!(edges.len(), 4);
    assert_eq!(clusters[0].medoid, "tmdb:movie:2");
    assert_eq!(clusters[0].size, 3);
    assert_eq!(
        nodes
            .iter()
            .find(|node| node.subject_key == "tmdb:movie:6")
            .map(|node| node.cluster_id),
        Some(UNPLACED_CLUSTER_ID)
    );
}

fn context_title(id: &str, tmdb_id: u32, created_day: u32) -> DiscoveryContextTitle {
    DiscoveryContextTitle {
        id: id.to_string(),
        library_id: "library-movies".to_string(),
        name: format!("Synthetic Title {id}"),
        facet: MediaFacet::Movie,
        external_ids: vec![ExternalId {
            source: "tmdb".to_string(),
            kind: None,
            value: tmdb_id.to_string(),
        }],
        genres: Vec::new(),
        created_at: Utc.with_ymd_and_hms(2026, 1, created_day, 0, 0, 0).unwrap(),
    }
}

#[test]
fn truncation_keeps_the_newest_subjects_and_records_both_counts() {
    let titles = (1..=(LIBRARY_GRAPH_MAX_SUBJECTS as u32 + 3))
        .map(|index| context_title(&format!("title-{index}"), index, 1 + index % 28))
        .collect::<Vec<_>>();
    let context =
        build_projected_discovery_library_context(&titles, DiscoveryContextDefaults::default());
    let subjects = library_graph_subjects(&context, &titles, "movie");
    let (submitted, total, omitted) = select_library_graph_subjects(subjects.clone());

    assert_eq!(total, LIBRARY_GRAPH_MAX_SUBJECTS + 3);
    assert_eq!(submitted.len(), LIBRARY_GRAPH_MAX_SUBJECTS);
    let mut left_out_keys = subjects[LIBRARY_GRAPH_MAX_SUBJECTS..]
        .iter()
        .map(|subject| subject.subject_key.clone())
        .collect::<Vec<_>>();
    left_out_keys.sort();
    assert_eq!(omitted, left_out_keys);
    let cutoff = submitted.last().unwrap();
    for left_out in &subjects[LIBRARY_GRAPH_MAX_SUBJECTS..] {
        assert!(
            left_out.added_at < cutoff.added_at
                || (left_out.added_at == cutoff.added_at
                    && left_out.subject_key > cutoff.subject_key),
            "{} should have been kept",
            left_out.subject_key
        );
    }
    // Deterministic: the same library selects the same subjects.
    let (again, _, _) =
        select_library_graph_subjects(library_graph_subjects(&context, &titles, "movie"));
    assert_eq!(again, submitted);
    // Each submitted subject names itself so returned nodes map back.
    assert!(
        submitted
            .iter()
            .all(|subject| subject.subject.key.as_deref() == Some(subject.subject_key.as_str()))
    );

    let snapshot = library_graph_snapshot_record(
        run(total as i64, submitted.len() as i64),
        &submitted,
        LibraryGraphGatewayResult::default(),
    );
    assert!(snapshot.run.submitted_count < snapshot.run.subject_count);
}

#[test]
fn titles_sharing_a_subject_are_one_node_with_every_title_id() {
    let titles = vec![
        context_title("title-a", 7, 2),
        DiscoveryContextTitle {
            library_id: "library-other".to_string(),
            ..context_title("title-b", 7, 5)
        },
    ];
    let context =
        build_projected_discovery_library_context(&titles, DiscoveryContextDefaults::default());
    let subjects = library_graph_subjects(&context, &titles, "movie");
    assert_eq!(subjects.len(), 1);
    assert_eq!(subjects[0].title_ids, vec!["title-a", "title-b"]);
    assert_eq!(subjects[0].added_at, titles[1].created_at);
}

#[test]
fn queue_full_maps_to_a_transient_run_code() {
    let queue_full = AppError::temporary_unavailable_reason(
        error_reasons::metadata_gateway::QUEUE_FULL,
        "at capacity",
        None,
    );
    assert_eq!(
        library_graph_failure_code(&queue_full),
        error_reasons::library_graph::GATEWAY_QUEUE_FULL
    );
    assert_eq!(
        library_graph_failure_code(&AppError::Repository("boom".to_string())),
        error_reasons::library_graph::GATEWAY_FAILED
    );
}

#[test]
fn incremental_changes_name_the_facets_to_rebuild() {
    let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let change = |facet: Option<&str>| DiscoveryPendingContextChangeRecord {
        id: "change".to_string(),
        scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
        subject_key: Some("tmdb:movie:1".to_string()),
        previous_subject_key: None,
        change_type: "added".to_string(),
        title_id: Some("title-a".to_string()),
        previous_title_id: None,
        library_facet: facet.map(str::to_string),
        raw_subject_json: None,
        raw_previous_subject_json: None,
        first_seen_sequence: None,
        last_seen_sequence: None,
        first_seen_at: at,
        last_seen_at: at,
    };
    let facets = library_graph_facets_for_changes(&[
        change(Some("anime")),
        change(Some("movie")),
        change(Some("movie")),
        change(None),
    ]);
    assert_eq!(
        facets.into_iter().collect::<Vec<_>>(),
        vec!["anime".to_string(), "movie".to_string()]
    );
}

#[test]
fn a_hidden_medoid_of_an_unkeyed_cluster_takes_its_label_with_it() {
    let mut snapshot = fixture_snapshot();
    // Unkeyed: the stored label is the medoid's own title.
    snapshot.clusters[0].label = "Hidden Synthetic Title B".to_string();
    snapshot.nodes[0].title_ids = vec!["title-a".to_string(), "title-a0".to_string()];
    let visible_titles = HashMap::from([
        ("title-a", "Visible Synthetic Title A"),
        ("title-c", "Visible Synthetic Title C"),
    ]);
    let (nodes, edges, clusters) = prune_library_graph_for_viewer(&snapshot, &visible_titles);
    assert_eq!(clusters[0].medoid, "tmdb:movie:1");
    // The new medoid's visible title names the cluster; title-a0 is not
    // visible, so it cannot lend its name either.
    assert_eq!(clusters[0].label, "Visible Synthetic Title A");
    let payload = format!("{nodes:?}{edges:?}{clusters:?}");
    assert!(!payload.contains("Hidden Synthetic Title B"), "{payload}");
    assert!(!payload.contains("title-b"), "{payload}");

    // A keyed cluster keeps its label: it names the theme, not a title.
    snapshot.clusters[0].label_key = Some("canonical:genre:heist".to_string());
    snapshot.clusters[0].label = "Heist".to_string();
    let (_, _, clusters) = prune_library_graph_for_viewer(&snapshot, &visible_titles);
    assert_eq!(clusters[0].label, "Heist");
}

fn failed_run(code: &str, completed_at: DateTime<Utc>) -> LibraryGraphRunRecord {
    LibraryGraphRunRecord {
        status: LIBRARY_GRAPH_RUN_FAILED.to_string(),
        error_code: Some(code.to_string()),
        completed_at: Some(completed_at),
        updated_at: completed_at,
        ..run(1, 1)
    }
}

#[test]
fn retry_waits_out_the_backoff_or_the_gateways_longer_ask() {
    let failed_at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let floor = chrono::Duration::seconds(LIBRARY_GRAPH_RETRY_BACKOFF_SECONDS);
    for code in [
        error_reasons::library_graph::GATEWAY_QUEUE_FULL,
        error_reasons::library_graph::GATEWAY_UNAVAILABLE,
    ] {
        let run = failed_run(code, failed_at);
        assert!(!library_graph_retry_due(Some(&run), true, failed_at));
        assert!(!library_graph_retry_due(
            Some(&run),
            true,
            failed_at + floor - chrono::Duration::seconds(1)
        ));
        assert!(library_graph_retry_due(Some(&run), true, failed_at + floor));

        // A longer wait from the gateway wins over the floor; a shorter one
        // does not shorten it.
        let longer = LibraryGraphRunRecord {
            retry_after_seconds: Some(LIBRARY_GRAPH_RETRY_BACKOFF_SECONDS * 2),
            ..run.clone()
        };
        assert!(!library_graph_retry_due(
            Some(&longer),
            true,
            failed_at + floor
        ));
        assert!(library_graph_retry_due(
            Some(&longer),
            true,
            failed_at + floor * 2
        ));
        let shorter = LibraryGraphRunRecord {
            retry_after_seconds: Some(30),
            ..run
        };
        assert!(!library_graph_retry_due(
            Some(&shorter),
            true,
            failed_at + chrono::Duration::seconds(30)
        ));
    }
}

#[test]
fn a_permanent_failure_is_retried_only_while_no_graph_is_stored() {
    let failed_at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let later = failed_at + chrono::Duration::seconds(LIBRARY_GRAPH_RETRY_BACKOFF_SECONDS);
    let run = failed_run(error_reasons::library_graph::GATEWAY_FAILED, failed_at);
    assert!(!library_graph_retry_due(Some(&run), true, later));
    assert!(!library_graph_retry_due(Some(&run), false, failed_at));
    assert!(library_graph_retry_due(Some(&run), false, later));
}

#[test]
fn a_facet_never_built_is_due_and_a_built_one_is_not() {
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    assert!(library_graph_retry_due(None, false, now));
    assert!(!library_graph_retry_due(Some(&run(1, 1)), true, now));
}
