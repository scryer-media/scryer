//! The library graph: the gateway's similarity graph over one facet's owned
//! titles, stored per facet and served to each viewer pruned to the titles
//! that viewer may see.
//!
//! A facet is rebuilt by the discovery sync job after a full context snapshot
//! commits (every facet) or after an incremental reload commits (the facets
//! its pending changes touched). Every sync run also rebuilds a facet with
//! owned titles whose build never landed or last failed for a transient
//! reason, once [`library_graph_retry_due`] says the backoff has passed. A
//! facet left with no owned titles loses its stored graph.

use super::*;
use crate::error_reasons;
use crate::library_scan::{LibraryGraphAxis, LibraryGraphGatewayInput, LibraryGraphGatewayResult};
use crate::ports::{
    DiscoveryContextTitle, LibraryGraphClusterRecord, LibraryGraphEdgeRecord,
    LibraryGraphNodeRecord, LibraryGraphRunRecord, LibraryGraphSnapshotRecord,
};

/// The embedding space every library graph is built in. This is the one place
/// a future per-instance choice of axis would hang off; nothing else in Scryer
/// names an axis.
pub(crate) const LIBRARY_GRAPH_AXIS: LibraryGraphAxis = LibraryGraphAxis::Semantic;

/// Nearest owned neighbours each subject contributes to the graph.
pub(crate) const LIBRARY_GRAPH_NEIGHBORS_PER_NODE: i32 = 8;

/// Cosine floor below which two titles get no semantic edge.
pub(crate) const LIBRARY_GRAPH_MIN_COSINE: f64 = 0.55;

/// Most subjects one gateway call carries.
///
/// A facet with more owned subjects than this submits a deterministic subset:
/// the most recently added subjects first (a subject's added time is the
/// newest `created_at` among the titles that share it), ties broken by subject
/// key ascending. The run records both counts and the subject keys it left
/// out, so a reader can tell whether its own titles are missing.
pub(crate) const LIBRARY_GRAPH_MAX_SUBJECTS: usize = 10_000;

const LIBRARY_GRAPH_RUN_COMPLETE: &str = "complete";
const LIBRARY_GRAPH_RUN_FAILED: &str = "failed";

/// The cluster id of a node the gateway did not place in a cluster.
const UNPLACED_CLUSTER_ID: i64 = -1;

/// The shortest wait before a failed build is tried again: the sync's own
/// incremental cadence. A longer wait the gateway asked for wins.
pub(crate) const LIBRARY_GRAPH_RETRY_BACKOFF_SECONDS: i64 = 4 * 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryGraphQuery {
    pub facet: MediaFacet,
    pub library_ids: Vec<String>,
    /// The `view_key` of the view the caller already holds. When the view
    /// this read would give has the same key, the read answers "unchanged"
    /// without loading the graph.
    pub known_view_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryGraphNode {
    pub subject_key: String,
    pub target_key: String,
    pub cluster_id: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryGraphEdge {
    pub from: String,
    pub to: String,
    pub weight: f64,
    pub kinds: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryGraphCluster {
    pub id: i64,
    pub label_key: Option<String>,
    pub label: String,
    /// Target key of the visible member that best represents the cluster.
    pub medoid: String,
    pub size: i64,
}

/// A facet's library graph as one viewer may see it.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryGraphView {
    pub generated_at: String,
    /// Names this exact view: the build it was pruned from and the titles
    /// the viewer could see behind it. A change of build, of the viewer's
    /// libraries or of a visible title's name or ids gives a new key.
    pub view_key: String,
    pub nodes: Vec<LibraryGraphNode>,
    pub edges: Vec<LibraryGraphEdge>,
    pub clusters: Vec<LibraryGraphCluster>,
    /// Owned subjects of the facet in the libraries this read covers.
    pub subject_count: i64,
    /// True when the build left out at least one subject this read covers
    /// because the facet was over [`LIBRARY_GRAPH_MAX_SUBJECTS`].
    pub truncated: bool,
    /// True when this view is the one the caller said it holds. The
    /// nodes, edges and clusters are then left empty, and the subject count
    /// and truncation flag are not computed.
    pub unchanged: bool,
}

/// What one facet's rebuild did, for the sync job's run summary.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct LibraryGraphRebuildSummary {
    pub(crate) facet: String,
    pub(crate) run_id: String,
    pub(crate) status: String,
    pub(crate) subject_count: usize,
    pub(crate) submitted_count: usize,
    pub(crate) node_count: usize,
    pub(crate) edge_count: usize,
    pub(crate) cluster_count: usize,
    pub(crate) error_code: Option<&'static str>,
}

/// One owned subject as the graph submits it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LibraryGraphSubject {
    pub(super) subject_key: String,
    pub(super) subject: DiscoverySubjectInput,
    pub(super) title_ids: Vec<String>,
    pub(super) added_at: DateTime<Utc>,
}

/// The facets an incremental reload's pending changes touched.
pub(crate) fn library_graph_facets_for_changes(
    changes: &[DiscoveryPendingContextChangeRecord],
) -> BTreeSet<String> {
    changes
        .iter()
        .filter_map(|change| change.library_facet.as_deref())
        .map(str::trim)
        .filter(|facet| !facet.is_empty())
        .map(str::to_string)
        .collect()
}

/// Every owned subject of `facet`, in submission order: newest first, then by
/// subject key.
pub(super) fn library_graph_subjects(
    context: &DiscoveryLibraryContext,
    titles: &[DiscoveryContextTitle],
    facet: &str,
) -> Vec<LibraryGraphSubject> {
    let added_at_by_title = titles
        .iter()
        .map(|title| (title.id.as_str(), title.created_at))
        .collect::<HashMap<_, _>>();
    let mut by_key: HashMap<&str, LibraryGraphSubject> = HashMap::new();
    for provenance in context
        .subject_provenance
        .iter()
        .filter(|provenance| provenance.facet == facet)
    {
        let added_at = added_at_by_title
            .get(provenance.title_id.as_str())
            .copied()
            .unwrap_or(DateTime::<Utc>::MIN_UTC);
        let entry = by_key
            .entry(provenance.subject_key.as_str())
            .or_insert_with(|| LibraryGraphSubject {
                subject_key: provenance.subject_key.clone(),
                subject: library_graph_subject_input(&provenance.subject_key, &provenance.subject),
                title_ids: Vec::new(),
                added_at,
            });
        if !entry.title_ids.contains(&provenance.title_id) {
            entry.title_ids.push(provenance.title_id.clone());
        }
        entry.added_at = entry.added_at.max(added_at);
    }
    let mut subjects = by_key.into_values().collect::<Vec<_>>();
    for subject in &mut subjects {
        subject.title_ids.sort();
    }
    subjects.sort_by(|left, right| {
        right
            .added_at
            .cmp(&left.added_at)
            .then_with(|| left.subject_key.cmp(&right.subject_key))
    });
    subjects
}

/// The submitted subject carries Scryer's own subject key, so every node the
/// gateway returns names the subject it came from. A locally hashed key means
/// no supported id was found; that subject goes by its ids alone and the
/// gateway names its node by the key it resolves.
fn library_graph_subject_input(
    subject_key: &str,
    subject: &DiscoverySubjectInput,
) -> DiscoverySubjectInput {
    let mut subject = subject.clone();
    if !subject_key.starts_with("local:") {
        subject.key = Some(subject_key.to_string());
    }
    subject
}

/// Keeps the first [`LIBRARY_GRAPH_MAX_SUBJECTS`] of `subjects`, which are
/// already in submission order. Returns the kept subjects, the total and the
/// keys of the subjects left out, sorted.
pub(super) fn select_library_graph_subjects(
    mut subjects: Vec<LibraryGraphSubject>,
) -> (Vec<LibraryGraphSubject>, usize, Vec<String>) {
    let total = subjects.len();
    let mut omitted = subjects
        .split_off(subjects.len().min(LIBRARY_GRAPH_MAX_SUBJECTS))
        .into_iter()
        .map(|subject| subject.subject_key)
        .collect::<Vec<_>>();
    omitted.sort();
    (subjects, total, omitted)
}

/// How long the gateway asked Scryer to wait, when it said.
fn library_graph_retry_after_seconds(error: &AppError) -> Option<i64> {
    match error {
        AppError::TemporaryUnavailable {
            retry_after: Some(retry_after),
            ..
        } => Some(i64::try_from(retry_after.as_secs()).unwrap_or(i64::MAX)),
        _ => None,
    }
}

/// Whether a sync run should rebuild a facet that has owned subjects, given
/// the facet's newest run and whether a complete graph is stored.
///
/// A facet with no run at all is built. A failed run is retried once the
/// backoff from its completion has passed, the longer of
/// [`LIBRARY_GRAPH_RETRY_BACKOFF_SECONDS`] and what the gateway asked for: a
/// transient failure always, any other failure only while the facet has no
/// complete graph to serve.
pub(crate) fn library_graph_retry_due(
    latest: Option<&LibraryGraphRunRecord>,
    has_complete: bool,
    now: DateTime<Utc>,
) -> bool {
    let Some(run) = latest else {
        return !has_complete;
    };
    if run.status != LIBRARY_GRAPH_RUN_FAILED {
        return !has_complete;
    }
    let transient = matches!(
        run.error_code.as_deref(),
        Some(
            error_reasons::library_graph::GATEWAY_QUEUE_FULL
                | error_reasons::library_graph::GATEWAY_UNAVAILABLE
        )
    );
    if !transient && has_complete {
        return false;
    }
    let backoff_seconds = run
        .retry_after_seconds
        .unwrap_or_default()
        .max(LIBRARY_GRAPH_RETRY_BACKOFF_SECONDS);
    let failed_at = run.completed_at.unwrap_or(run.updated_at);
    failed_at
        .checked_add_signed(chrono::Duration::seconds(backoff_seconds))
        .is_some_and(|retry_at| now >= retry_at)
}

/// The run-level code a failed build records.
fn library_graph_failure_code(error: &AppError) -> &'static str {
    if error.reason() == Some(error_reasons::metadata_gateway::QUEUE_FULL) {
        return error_reasons::library_graph::GATEWAY_QUEUE_FULL;
    }
    match error {
        AppError::TemporaryUnavailable { .. } => error_reasons::library_graph::GATEWAY_UNAVAILABLE,
        _ => error_reasons::library_graph::GATEWAY_FAILED,
    }
}

/// Turns the gateway's answer into the stored graph. Each node carries the
/// owned titles behind it, matched on the subject key Scryer submitted.
pub(super) fn library_graph_snapshot_record(
    run: LibraryGraphRunRecord,
    submitted: &[LibraryGraphSubject],
    result: LibraryGraphGatewayResult,
) -> LibraryGraphSnapshotRecord {
    let title_ids_by_key = submitted
        .iter()
        .map(|subject| (subject.subject_key.as_str(), &subject.title_ids))
        .collect::<HashMap<_, _>>();
    let mut seen_nodes = HashSet::new();
    let nodes = result
        .nodes
        .into_iter()
        .filter(|node| seen_nodes.insert(node.subject_key.clone()))
        .map(|node| LibraryGraphNodeRecord {
            title_ids: title_ids_by_key
                .get(node.subject_key.as_str())
                .map(|ids| (*ids).clone())
                .unwrap_or_default(),
            subject_key: node.subject_key,
            target_key: node.target_key,
            cluster_id: node.cluster_id,
        })
        .collect();
    let mut seen_edges = HashSet::new();
    let edges = result
        .edges
        .into_iter()
        .map(|edge| {
            // Stored once per pair with from < to, whatever order arrived.
            let (from_key, to_key) = if edge.from <= edge.to {
                (edge.from, edge.to)
            } else {
                (edge.to, edge.from)
            };
            LibraryGraphEdgeRecord {
                from_key,
                to_key,
                weight: edge.weight,
                kinds: edge.kinds,
            }
        })
        .filter(|edge| seen_edges.insert((edge.from_key.clone(), edge.to_key.clone())))
        .collect();
    let mut seen_clusters = HashSet::new();
    let clusters = result
        .clusters
        .into_iter()
        .filter(|cluster| seen_clusters.insert(cluster.id))
        .map(|cluster| LibraryGraphClusterRecord {
            cluster_id: cluster.id,
            label_key: cluster.label_key,
            label: cluster.label,
            medoid: cluster.medoid,
            size: cluster.size,
        })
        .collect();
    LibraryGraphSnapshotRecord {
        run,
        nodes,
        edges,
        clusters,
    }
}

/// Prunes a stored graph to the titles a viewer may see.
///
/// A node is visible when any owned title behind it is in
/// `visible_title_ids`. An edge stays only when both of its target keys belong
/// to visible nodes. Cluster sizes are recounted over visible nodes and empty
/// clusters are dropped. When a cluster's medoid is no longer visible, the
/// visible member with the highest total incident edge weight stands in, ties
/// broken by subject key. A cluster without a label key is labelled with its
/// medoid's title, so a replaced medoid also replaces the label with the name
/// of a visible title behind the new medoid.
///
/// `visible_titles` maps each visible title id to its name.
pub(super) fn prune_library_graph_for_viewer(
    snapshot: &LibraryGraphSnapshotRecord,
    visible_titles: &HashMap<&str, &str>,
) -> (
    Vec<LibraryGraphNode>,
    Vec<LibraryGraphEdge>,
    Vec<LibraryGraphCluster>,
) {
    let visible_nodes = snapshot
        .nodes
        .iter()
        .filter(|node| {
            node.title_ids
                .iter()
                .any(|title_id| visible_titles.contains_key(title_id.as_str()))
        })
        .collect::<Vec<_>>();
    let visible_targets = visible_nodes
        .iter()
        .map(|node| node.target_key.as_str())
        .collect::<HashSet<_>>();

    let edges = snapshot
        .edges
        .iter()
        .filter(|edge| {
            visible_targets.contains(edge.from_key.as_str())
                && visible_targets.contains(edge.to_key.as_str())
        })
        .map(|edge| LibraryGraphEdge {
            from: edge.from_key.clone(),
            to: edge.to_key.clone(),
            weight: edge.weight,
            kinds: edge.kinds.clone(),
        })
        .collect::<Vec<_>>();

    let mut incident_weight: HashMap<&str, f64> = HashMap::new();
    for edge in &edges {
        *incident_weight.entry(edge.from.as_str()).or_default() += edge.weight;
        *incident_weight.entry(edge.to.as_str()).or_default() += edge.weight;
    }

    let mut members_by_cluster: HashMap<i64, Vec<&LibraryGraphNodeRecord>> = HashMap::new();
    for node in &visible_nodes {
        if node.cluster_id != UNPLACED_CLUSTER_ID {
            members_by_cluster
                .entry(node.cluster_id)
                .or_default()
                .push(node);
        }
    }

    let clusters = snapshot
        .clusters
        .iter()
        .filter_map(|cluster| {
            let members = members_by_cluster.get(&cluster.cluster_id)?;
            let medoid_visible = members
                .iter()
                .any(|member| member.target_key == cluster.medoid);
            if medoid_visible {
                return Some(LibraryGraphCluster {
                    id: cluster.cluster_id,
                    label_key: cluster.label_key.clone(),
                    label: cluster.label.clone(),
                    medoid: cluster.medoid.clone(),
                    size: members.len() as i64,
                });
            }
            let replacement = members.iter().max_by(|left, right| {
                let left_weight = incident_weight
                    .get(left.target_key.as_str())
                    .copied()
                    .unwrap_or_default();
                let right_weight = incident_weight
                    .get(right.target_key.as_str())
                    .copied()
                    .unwrap_or_default();
                left_weight
                    .total_cmp(&right_weight)
                    // Lower subject key wins a tie, so it must
                    // compare as the greater one here.
                    .then_with(|| right.subject_key.cmp(&left.subject_key))
            })?;
            // The stored label of an unkeyed cluster is the hidden medoid's
            // title; it must not reach this viewer.
            let label = if cluster.label_key.is_some() {
                cluster.label.clone()
            } else {
                replacement
                    .title_ids
                    .iter()
                    .filter_map(|title_id| visible_titles.get_key_value(title_id.as_str()))
                    .min_by(|left, right| left.0.cmp(right.0))
                    .map(|(_, name)| (*name).to_string())
                    .unwrap_or_default()
            };
            Some(LibraryGraphCluster {
                id: cluster.cluster_id,
                label_key: cluster.label_key.clone(),
                label,
                medoid: replacement.target_key.clone(),
                size: members.len() as i64,
            })
        })
        .collect::<Vec<_>>();
    let kept_clusters = clusters
        .iter()
        .map(|cluster| cluster.id)
        .collect::<HashSet<_>>();

    let nodes = visible_nodes
        .iter()
        .map(|node| LibraryGraphNode {
            subject_key: node.subject_key.clone(),
            target_key: node.target_key.clone(),
            // A node whose cluster the store no longer has reads as unplaced.
            cluster_id: if kept_clusters.contains(&node.cluster_id) {
                node.cluster_id
            } else {
                UNPLACED_CLUSTER_ID
            },
        })
        .collect();
    (nodes, edges, clusters)
}

impl AppUseCase {
    /// A facet's stored library graph, pruned to what `actor` may see. `None`
    /// when personalized discovery is off, the actor cannot see the facet or
    /// any of the requested libraries, or no graph has been built yet. When
    /// the query names the build time the caller holds and it is still the
    /// newest, the answer is an empty view marked `unchanged`.
    pub async fn library_graph(
        &self,
        actor: &User,
        query: LibraryGraphQuery,
    ) -> AppResult<Option<LibraryGraphView>> {
        let visibility = self.discovery_visibility(actor).await?;
        if !visibility.allows_facet(&query.facet) {
            return Ok(None);
        }
        let effective_library_ids = self
            .discovery_effective_library_ids(actor, &query.facet, &query.library_ids)
            .await?;
        if effective_library_ids.is_empty() || !self.personalized_discovery_enabled().await? {
            return Ok(None);
        }
        let titles = self
            .services
            .catalog
            .titles
            .list_discovery_context_titles()
            .await?;
        let facet = query.facet;
        if let Some(known) = query.known_view_key.filter(|known| !known.is_empty()) {
            let Some(latest) = self
                .services
                .library
                .discovery
                .get_latest_library_graph_generated_at(facet.as_str())
                .await?
            else {
                return Ok(None);
            };
            // The held view is still right only when the build and every
            // visible title behind it are the same as now.
            let generated_at = latest.unwrap_or_default();
            let build = generated_at.clone();
            let (titles_back, facet_back, libraries_back, view_key) =
                tokio::task::spawn_blocking(move || {
                    let view_key = Self::library_graph_view_key(
                        &build,
                        &Self::library_graph_visible_titles(
                            &titles,
                            &facet,
                            &effective_library_ids,
                        ),
                    );
                    (titles, facet, effective_library_ids, view_key)
                })
                .await
                .map_err(|error| {
                    if error.is_panic() {
                        std::panic::resume_unwind(error.into_panic())
                    }
                    AppError::Repository(error.to_string())
                })?;
            if view_key == known {
                return Ok(Some(LibraryGraphView {
                    generated_at,
                    view_key,
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    clusters: Vec::new(),
                    subject_count: 0,
                    truncated: false,
                    unchanged: true,
                }));
            }
            return self
                .library_graph_full_view(titles_back, facet_back, libraries_back)
                .await;
        }
        self.library_graph_full_view(titles, facet, effective_library_ids)
            .await
    }

    /// The newest stored graph scoped for the viewer, or `None` when no graph
    /// has been built yet.
    async fn library_graph_full_view(
        &self,
        titles: Vec<DiscoveryContextTitle>,
        facet: MediaFacet,
        effective_library_ids: HashSet<String>,
    ) -> AppResult<Option<LibraryGraphView>> {
        let Some(snapshot) = self
            .services
            .library
            .discovery
            .get_latest_library_graph(facet.as_str())
            .await?
        else {
            return Ok(None);
        };
        // Scoping and pruning a full graph is CPU work; it runs on the
        // blocking pool so it does not hold an async worker.
        let view = tokio::task::spawn_blocking(move || {
            Self::library_graph_view_for_viewer(&snapshot, &titles, &facet, &effective_library_ids)
        })
        .await;
        match view {
            Ok(view) => Ok(Some(view)),
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(error) => Err(AppError::Repository(error.to_string())),
        }
    }

    /// The titles of `facet` in `effective_library_ids`, each with its
    /// discovery subject key when it has one.
    fn library_graph_visible_titles<'a>(
        titles: &'a [DiscoveryContextTitle],
        facet: &MediaFacet,
        effective_library_ids: &HashSet<String>,
    ) -> Vec<(&'a DiscoveryContextTitle, Option<String>)> {
        titles
            .iter()
            .filter(|title| {
                title.facet == *facet && effective_library_ids.contains(&title.library_id)
            })
            .map(|title| {
                let subject_key = build_discovery_subject_parts(
                    &title.facet,
                    normalize_external_ids(&title.external_ids),
                )
                .map(|parts| parts.subject_key);
                (title, subject_key)
            })
            .collect()
    }

    /// The key of the view built from the graph generated at `generated_at`
    /// over `visible_titles`: everything the pruned view depends on besides
    /// the stored graph itself.
    fn library_graph_view_key(
        generated_at: &str,
        visible_titles: &[(&DiscoveryContextTitle, Option<String>)],
    ) -> String {
        fn field(hasher: &mut blake3::Hasher, value: &str) {
            hasher.update(&(value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        }
        let mut entries = visible_titles
            .iter()
            .map(|(title, subject_key)| {
                (
                    title.id.as_str(),
                    title.name.as_str(),
                    subject_key.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        entries.sort_unstable();
        let mut hasher = blake3::Hasher::new();
        field(&mut hasher, generated_at);
        for (id, name, subject_key) in entries {
            field(&mut hasher, id);
            field(&mut hasher, name);
            match subject_key {
                Some(subject_key) => {
                    hasher.update(&[1]);
                    field(&mut hasher, subject_key);
                }
                None => {
                    hasher.update(&[0]);
                }
            }
        }
        format!("v1:{}", hasher.finalize().to_hex())
    }

    /// The stored graph scoped to the titles the viewer can see in
    /// `effective_library_ids`.
    fn library_graph_view_for_viewer(
        snapshot: &LibraryGraphSnapshotRecord,
        titles: &[DiscoveryContextTitle],
        facet: &MediaFacet,
        effective_library_ids: &HashSet<String>,
    ) -> LibraryGraphView {
        let visible_titles =
            Self::library_graph_visible_titles(titles, facet, effective_library_ids);
        let visible_title_names = visible_titles
            .iter()
            .map(|(title, _)| (title.id.as_str(), title.name.as_str()))
            .collect::<HashMap<_, _>>();
        let visible_subject_keys = visible_titles
            .iter()
            .filter_map(|(_, subject_key)| subject_key.clone())
            .collect::<HashSet<_>>();
        // Only the viewer's own subjects count: a hidden library pushing the
        // facet over the cap is not this viewer's business.
        let truncated = snapshot
            .run
            .omitted_subject_keys
            .iter()
            .any(|subject_key| visible_subject_keys.contains(subject_key));

        let (nodes, edges, clusters) =
            prune_library_graph_for_viewer(snapshot, &visible_title_names);
        let generated_at = snapshot.run.generated_at.clone().unwrap_or_default();
        LibraryGraphView {
            view_key: Self::library_graph_view_key(&generated_at, &visible_titles),
            generated_at,
            nodes,
            edges,
            clusters,
            subject_count: visible_subject_keys.len() as i64,
            truncated,
            unchanged: false,
        }
    }

    /// The facets among `owned_facets` whose graph a sync run should rebuild
    /// although nothing in the run touched them: see
    /// [`library_graph_retry_due`]. A facet whose runs cannot be read is
    /// skipped and logged.
    pub(crate) async fn library_graph_facets_due_for_retry(
        &self,
        owned_facets: &BTreeSet<String>,
        now: DateTime<Utc>,
    ) -> BTreeSet<String> {
        let discovery = &self.services.library.discovery;
        let mut due = BTreeSet::new();
        for facet in owned_facets {
            let latest = match discovery.get_latest_library_graph_run(facet).await {
                Ok(latest) => latest,
                Err(error) => {
                    warn!(facet = facet.as_str(), error = %error, "library graph runs could not be read");
                    continue;
                }
            };
            // A complete run counts only with its stored graph: a run row
            // without one reads as no graph, so the facet must be rebuilt.
            let has_complete = match discovery.has_complete_library_graph(facet).await {
                Ok(has_complete) => has_complete,
                Err(error) => {
                    warn!(facet = facet.as_str(), error = %error, "library graph runs could not be read");
                    continue;
                }
            };
            if library_graph_retry_due(latest.as_ref(), has_complete, now) {
                due.insert(facet.clone());
            }
        }
        due
    }

    /// Rebuilds the library graph of each facet in `facets` that has owned
    /// subjects, and deletes the stored runs of each one that has none left.
    /// A failure is logged and recorded on that facet's run and never
    /// returned: the sync run that asked for the rebuild carries on. Each
    /// facet is asked for once; a gateway at capacity is retried by a later
    /// sync run, see [`library_graph_retry_due`].
    pub(crate) async fn rebuild_library_graphs(
        &self,
        titles: &[DiscoveryContextTitle],
        context: &DiscoveryLibraryContext,
        facets: &BTreeSet<String>,
    ) -> Vec<LibraryGraphRebuildSummary> {
        let mut summaries = Vec::new();
        for facet in facets {
            let subjects = library_graph_subjects(context, titles, facet);
            if subjects.is_empty() {
                if let Err(error) = self
                    .services
                    .library
                    .discovery
                    .delete_library_graph_runs(facet)
                    .await
                {
                    warn!(
                        facet = facet.as_str(),
                        error = %error,
                        "library graph of a facet with no owned titles could not be deleted"
                    );
                }
                continue;
            }
            summaries.push(self.rebuild_library_graph_facet(facet, subjects).await);
        }
        summaries
    }

    async fn rebuild_library_graph_facet(
        &self,
        facet: &str,
        subjects: Vec<LibraryGraphSubject>,
    ) -> LibraryGraphRebuildSummary {
        let started_at = self.runtime.environment.now();
        let (submitted, subject_count, omitted_subject_keys) =
            select_library_graph_subjects(subjects);
        let run_id = format!("library-graph-{}", uuid::Uuid::new_v4());
        let mut run = LibraryGraphRunRecord {
            id: run_id.clone(),
            facet: facet.to_string(),
            axis: LIBRARY_GRAPH_AXIS.as_str().to_string(),
            status: LIBRARY_GRAPH_RUN_COMPLETE.to_string(),
            subject_count: subject_count as i64,
            submitted_count: submitted.len() as i64,
            generated_at: None,
            error_code: None,
            error_text: None,
            retry_after_seconds: None,
            omitted_subject_keys,
            started_at,
            completed_at: None,
            created_at: started_at,
            updated_at: started_at,
        };
        let mut summary = LibraryGraphRebuildSummary {
            facet: facet.to_string(),
            run_id,
            status: LIBRARY_GRAPH_RUN_COMPLETE.to_string(),
            subject_count,
            submitted_count: submitted.len(),
            node_count: 0,
            edge_count: 0,
            cluster_count: 0,
            error_code: None,
        };

        let input = LibraryGraphGatewayInput {
            subjects: submitted
                .iter()
                .map(|subject| subject.subject.clone())
                .collect(),
            axis: LIBRARY_GRAPH_AXIS,
            neighbors_per_node: LIBRARY_GRAPH_NEIGHBORS_PER_NODE,
            min_cosine: LIBRARY_GRAPH_MIN_COSINE,
        };
        let result = match self
            .services
            .library
            .metadata_gateway
            .library_graph(&input)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                let code = library_graph_failure_code(&error);
                warn!(
                    facet,
                    error_code = code,
                    error = %error,
                    "library graph build failed"
                );
                self.record_failed_library_graph_run(&mut run, &mut summary, code, &error)
                    .await;
                return summary;
            }
        };

        let completed_at = self.runtime.environment.now();
        run.generated_at = Some(result.generated_at.clone());
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let snapshot = library_graph_snapshot_record(run.clone(), &submitted, result);
        summary.node_count = snapshot.nodes.len();
        summary.edge_count = snapshot.edges.len();
        summary.cluster_count = snapshot.clusters.len();
        if let Err(error) = self
            .services
            .library
            .discovery
            .commit_library_graph(&snapshot)
            .await
        {
            let code = error_reasons::library_graph::STORE_FAILED;
            warn!(facet, error_code = code, error = %error, "library graph could not be stored");
            run.generated_at = None;
            self.record_failed_library_graph_run(&mut run, &mut summary, code, &error)
                .await;
            return summary;
        }
        debug!(
            facet,
            subject_count,
            submitted_count = summary.submitted_count,
            node_count = summary.node_count,
            edge_count = summary.edge_count,
            cluster_count = summary.cluster_count,
            "library graph stored"
        );
        summary
    }

    async fn record_failed_library_graph_run(
        &self,
        run: &mut LibraryGraphRunRecord,
        summary: &mut LibraryGraphRebuildSummary,
        code: &'static str,
        error: &AppError,
    ) {
        let failed_at = self.runtime.environment.now();
        run.status = LIBRARY_GRAPH_RUN_FAILED.to_string();
        run.error_code = Some(code.to_string());
        run.error_text = Some(error.to_string());
        run.retry_after_seconds = library_graph_retry_after_seconds(error);
        run.completed_at = Some(failed_at);
        run.updated_at = failed_at;
        summary.status = LIBRARY_GRAPH_RUN_FAILED.to_string();
        summary.error_code = Some(code);
        summary.node_count = 0;
        summary.edge_count = 0;
        summary.cluster_count = 0;
        if let Err(record_error) = self
            .services
            .library
            .discovery
            .record_failed_library_graph_run(run)
            .await
        {
            warn!(
                facet = run.facet.as_str(),
                error = %record_error,
                "library graph failure could not be recorded"
            );
        }
    }
}

#[cfg(test)]
#[path = "library_graph_tests.rs"]
mod tests;
