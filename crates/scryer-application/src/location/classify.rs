//! Per-title classification of a requested destination.
//!
//! Every title in a selection classifies into exactly one class, and the preview
//! groups them with counts and omits none (FR-015, SC-005). The fileless
//! catalog-only fast path is its own class so it never presents a move-mode
//! choice (FR-076).
//!
//! # Facts in, classes out
//!
//! [`classify_selection`] performs no IO. Everything it needs about a title
//! arrives as a [`TitleClassificationFacts`] value the caller assembled, the way
//! [`crate::location::collisions`] takes its destination facts. That keeps the
//! whole FR-015/FR-016/FR-017/FR-076/FR-086 rule set unit-testable, and it means
//! the preview, the confirm path, and the executor's admission check can all ask
//! the same function the same question and never disagree.

use serde::{Deserialize, Serialize};

use scryer_domain::MediaFacet;

/// The single class a selected title falls into for a requested destination.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TitleLocationClass {
    /// Destination is in another library and the transfer is supported.
    CrossLibraryTransfer,
    /// Destination is another root inside the title's current library.
    RootMove,
    /// The title already lives at the requested destination; nothing to do.
    NoOp,
    /// Monitored title with no tracked files on disk: catalog reassignment only,
    /// no filesystem work and no move-mode selection (FR-076).
    CatalogOnly,
    /// The destination can never accept this title (facet or media-kind rules in
    /// spec "Boundaries"); the reason names the incompatible source library or
    /// facet (FR-017).
    Incompatible,
    /// The title could go, but a user decision is outstanding: ambiguous
    /// destination-title identity (FR-055), an active download or import
    /// (FR-086), or unmapped episode-scoped merge records (FR-066).
    NeedsResolution,
}

impl TitleLocationClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CrossLibraryTransfer => "cross_library_transfer",
            Self::RootMove => "root_move",
            Self::NoOp => "no_op",
            Self::CatalogOnly => "catalog_only",
            Self::Incompatible => "incompatible",
            Self::NeedsResolution => "needs_resolution",
        }
    }

    /// Parse a persisted class value (checkpoint rows carry the class the title
    /// was previewed as). Unknown values are rejected rather than defaulted.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "cross_library_transfer" => Some(Self::CrossLibraryTransfer),
            "root_move" => Some(Self::RootMove),
            "no_op" => Some(Self::NoOp),
            "catalog_only" => Some(Self::CatalogOnly),
            "incompatible" => Some(Self::Incompatible),
            "needs_resolution" => Some(Self::NeedsResolution),
            _ => None,
        }
    }

    /// Whether a title in this class moves bytes. `NoOp`, `CatalogOnly`,
    /// `Incompatible`, and `NeedsResolution` never do.
    pub fn moves_files(&self) -> bool {
        matches!(self, Self::CrossLibraryTransfer | Self::RootMove)
    }

    /// A bulk operation must not start while any title is in a blocking class;
    /// the user resolves it or removes it from the selection (FR-016).
    pub fn blocks_start(&self) -> bool {
        matches!(self, Self::Incompatible | Self::NeedsResolution)
    }
}

/// One classified title in a preview, carrying the explanation the UI shows for
/// blocking classes (FR-017).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassifiedTitle {
    pub title_id: String,
    pub class: TitleLocationClass,
    /// Why this title landed in its class. Required for `Incompatible` and
    /// `NeedsResolution`; optional otherwise.
    pub reason: Option<String>,
}

/// Complete counts per class for a selection. Every selected title is counted
/// exactly once (SC-005).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ClassificationCounts {
    pub cross_library_transfer: i64,
    pub root_move: i64,
    pub no_op: i64,
    pub catalog_only: i64,
    pub incompatible: i64,
    pub needs_resolution: i64,
}

impl ClassificationCounts {
    pub fn total(&self) -> i64 {
        self.cross_library_transfer
            + self.root_move
            + self.no_op
            + self.catalog_only
            + self.incompatible
            + self.needs_resolution
    }

    /// Any non-zero blocking class prevents the operation from starting
    /// (FR-016).
    pub fn blocks_start(&self) -> bool {
        self.incompatible > 0 || self.needs_resolution > 0
    }
}

/// Machine-readable reason codes carried on a [`ClassifiedTitle`], so the UI can
/// group and translate without parsing prose (C3).
pub mod reason_codes {
    /// The destination library's facet can never accept the title's facet.
    pub const INCOMPATIBLE_FACET: &str = "incompatible_facet";
    /// The requested destination root does not belong to the destination
    /// library.
    pub const ROOT_NOT_IN_DESTINATION_LIBRARY: &str = "root_not_in_destination_library";
    /// An active download or import owns the title right now (FR-086).
    pub const ACTIVE_DOWNLOAD_OR_IMPORT: &str = "active_download_or_import";
    /// Another location operation already owns the title (FR-084).
    pub const OWNED_BY_LOCATION_OPERATION: &str = "owned_by_location_operation";
    /// The title has no tracked files, so the change is catalog-only (FR-076).
    pub const NO_TRACKED_FILES: &str = "no_tracked_files";
    /// The title already lives at the requested destination.
    pub const ALREADY_AT_DESTINATION: &str = "already_at_destination";
    /// The title cannot be placed because its folder is unknown.
    pub const NO_FOLDER_MATCH: &str = "no_folder_match";
}

/// The destination a selection was previewed against.
///
/// Both fields are optional because bulk editing exposes two independent
/// controls (FR-010): changing only the root keeps every title in its own
/// library, and changing only the library lets the destination library's root
/// selection decide.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DestinationRequest {
    /// Destination library, or `None` to keep each title in its own library.
    pub library_id: Option<String>,
    /// Destination root inside the destination library, or `None` to keep each
    /// title on its own root.
    pub root_id: Option<String>,
}

impl DestinationRequest {
    pub fn to_root(root_id: impl Into<String>) -> Self {
        Self {
            library_id: None,
            root_id: Some(root_id.into()),
        }
    }

    pub fn to_library(library_id: impl Into<String>) -> Self {
        Self {
            library_id: Some(library_id.into()),
            root_id: None,
        }
    }

    pub fn to_library_root(library_id: impl Into<String>, root_id: impl Into<String>) -> Self {
        Self {
            library_id: Some(library_id.into()),
            root_id: Some(root_id.into()),
        }
    }

    /// Nothing was asked for, so every title is a no-op.
    pub fn is_empty(&self) -> bool {
        self.library_id.is_none() && self.root_id.is_none()
    }
}

/// What the classifier knows about the destination library the request names.
///
/// `None` for the whole struct means "keep each title's own library", which is
/// the same-library root move US2 is about.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DestinationLibraryFacts {
    pub library_id: String,
    /// Shown in the FR-017 explanation for a disabled destination.
    pub library_name: String,
    pub facet: MediaFacet,
    /// Root ids configured on the destination library, used to reject a root id
    /// that belongs somewhere else.
    pub root_ids: Vec<String>,
}

/// Everything the classifier needs about one selected title.
///
/// Assembled by the caller from the catalog; the classifier never reads a
/// repository, so every rule below is testable from literals.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleClassificationFacts {
    pub title_id: String,
    /// Used only in explanations.
    pub title_name: String,
    pub facet: MediaFacet,
    pub monitored: bool,
    pub library_id: String,
    /// Named in the FR-017 incompatibility explanation.
    pub library_name: String,
    pub root_id: String,
    /// Whether the title owns a folder today. A title with tracked files but no
    /// folder match cannot have a destination folder calculated for it.
    pub has_folder_match: bool,
    /// Tracked media files on disk. Zero is the FR-076 fast path.
    pub tracked_file_count: i64,
    /// An active download or import on this title (FR-086). The string is the
    /// explanation shown beside the deselect control.
    pub active_work: Option<String>,
    /// Another location operation already owns this title (FR-084).
    pub owned_by_operation: Option<String>,
    /// Any other outstanding user decision the caller already knows about
    /// (ambiguous destination identity, unmapped merge records).
    pub unresolved: Option<String>,
}

impl TitleClassificationFacts {
    /// The minimum a caller has to supply; the optional blockers default to
    /// "nothing outstanding".
    pub fn new(
        title_id: impl Into<String>,
        facet: MediaFacet,
        library_id: impl Into<String>,
        root_id: impl Into<String>,
    ) -> Self {
        let title_id = title_id.into();
        Self {
            title_name: title_id.clone(),
            title_id,
            facet,
            monitored: true,
            library_id: library_id.into(),
            library_name: String::new(),
            root_id: root_id.into(),
            has_folder_match: true,
            tracked_file_count: 0,
            active_work: None,
            owned_by_operation: None,
            unresolved: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.title_name = name.into();
        self
    }

    pub fn with_library_name(mut self, name: impl Into<String>) -> Self {
        self.library_name = name.into();
        self
    }

    pub fn with_tracked_files(mut self, count: i64) -> Self {
        self.tracked_file_count = count;
        self
    }

    pub fn with_folder_match(mut self, has_folder_match: bool) -> Self {
        self.has_folder_match = has_folder_match;
        self
    }

    pub fn with_monitored(mut self, monitored: bool) -> Self {
        self.monitored = monitored;
        self
    }

    pub fn with_active_work(mut self, detail: impl Into<String>) -> Self {
        self.active_work = Some(detail.into());
        self
    }

    pub fn with_owned_by_operation(mut self, operation_id: impl Into<String>) -> Self {
        self.owned_by_operation = Some(operation_id.into());
        self
    }

    pub fn with_unresolved(mut self, detail: impl Into<String>) -> Self {
        self.unresolved = Some(detail.into());
        self
    }

    /// Whether the title has content on disk this operation would have to move.
    pub fn has_tracked_files(&self) -> bool {
        self.tracked_file_count > 0
    }
}

/// Whether `destination` can ever accept a title with `source` facet.
///
/// Spec "Boundaries": movies move only between movie libraries; series and anime
/// move within their own facet or between series and anime libraries; the
/// movie/episodic boundary is not crossed (the FR-060–062 carve-outs are
/// title-level dispositions handled by the cross-library phase, not a facet
/// rule).
pub fn facets_are_compatible(source: &MediaFacet, destination: &MediaFacet) -> bool {
    match (source, destination) {
        (MediaFacet::Movie, MediaFacet::Movie) => true,
        (MediaFacet::Movie, _) | (_, MediaFacet::Movie) => false,
        // Series and anime are both episodic; a crossover converts the facet
        // automatically (FR-057).
        _ => true,
    }
}

/// One classified title carrying the machine-readable reason beside the prose.
///
/// [`ClassifiedTitle`] is the persisted/serialized shape; this is the richer
/// value the planner works with before it reduces to it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleClassification {
    pub title_id: String,
    pub class: TitleLocationClass,
    /// Library the title ends up in.
    pub destination_library_id: String,
    /// Root the title ends up on.
    pub destination_root_id: String,
    pub reason_code: Option<String>,
    pub reason: Option<String>,
}

impl TitleClassification {
    pub fn to_classified_title(&self) -> ClassifiedTitle {
        ClassifiedTitle {
            title_id: self.title_id.clone(),
            class: self.class,
            reason: self.reason.clone(),
        }
    }

    /// Whether this title stops the operation from starting (FR-016).
    pub fn blocks_start(&self) -> bool {
        self.class.blocks_start()
    }
}

/// The complete result of classifying a selection: one entry per selected
/// title, in selection order, plus the grouped counts (FR-015, SC-005).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SelectionClassification {
    pub titles: Vec<TitleClassification>,
    pub counts: ClassificationCounts,
}

impl SelectionClassification {
    /// Ids the user must resolve or deselect before the job can start
    /// (FR-016, FR-086).
    pub fn blocking_title_ids(&self) -> Vec<String> {
        self.titles
            .iter()
            .filter(|title| title.blocks_start())
            .map(|title| title.title_id.clone())
            .collect()
    }

    /// Ids in one class, for the preview's grouped lists.
    pub fn title_ids_in(&self, class: TitleLocationClass) -> Vec<String> {
        self.titles
            .iter()
            .filter(|title| title.class == class)
            .map(|title| title.title_id.clone())
            .collect()
    }

    pub fn classification_of(&self, title_id: &str) -> Option<&TitleClassification> {
        self.titles.iter().find(|title| title.title_id == title_id)
    }

    /// The plan-level FR-016 check the preview and confirm paths enforce: a bulk
    /// job must not start while an included title is unresolved or incompatible.
    pub fn blocks_start(&self) -> bool {
        self.counts.blocks_start()
    }

    pub fn to_classified_titles(&self) -> Vec<ClassifiedTitle> {
        self.titles
            .iter()
            .map(TitleClassification::to_classified_title)
            .collect()
    }
}

/// Classify one title against the requested destination.
///
/// The order of the checks is the contract, not an implementation detail:
///
/// 1. **Incompatible** first — a destination that can never accept the title is
///    the answer regardless of anything else about it (FR-017).
/// 2. **No-op** — already at the destination, so nothing can go wrong and
///    nothing needs resolving.
/// 3. **Blocked** — an active download or import, another operation's ownership,
///    or a caller-supplied unresolved decision (FR-086, FR-084, FR-016). This
///    sits *above* the fileless fast path on purpose: a fileless title with an
///    in-flight import is exactly the case where files are about to land in the
///    old root.
/// 4. **Catalog-only** — no tracked files, so there is nothing to move (FR-076).
/// 5. The move itself: cross-library or same-library root move.
pub fn classify_title(
    facts: &TitleClassificationFacts,
    destination: &DestinationRequest,
    destination_library: Option<&DestinationLibraryFacts>,
) -> TitleClassification {
    let destination_library_id = destination
        .library_id
        .clone()
        .unwrap_or_else(|| facts.library_id.clone());
    let crosses_libraries = destination_library_id != facts.library_id;

    let classified = |class: TitleLocationClass,
                      root_id: String,
                      reason_code: Option<&str>,
                      reason: Option<String>| TitleClassification {
        title_id: facts.title_id.clone(),
        class,
        destination_library_id: destination_library_id.clone(),
        destination_root_id: root_id,
        reason_code: reason_code.map(str::to_string),
        reason,
    };

    // 1. Incompatible: FR-017's explanation names the source library and facet.
    if crosses_libraries {
        let Some(library) = destination_library else {
            return classified(
                TitleLocationClass::Incompatible,
                facts.root_id.clone(),
                Some(reason_codes::INCOMPATIBLE_FACET),
                Some(format!(
                    "destination library {destination_library_id} is unknown, so \"{}\" cannot be moved into it",
                    facts.title_name
                )),
            );
        };
        if !facets_are_compatible(&facts.facet, &library.facet) {
            return classified(
                TitleLocationClass::Incompatible,
                facts.root_id.clone(),
                Some(reason_codes::INCOMPATIBLE_FACET),
                Some(format!(
                    "\"{}\" is a {} title in {}; a {} library cannot accept it",
                    facts.title_name,
                    facts.facet.as_str(),
                    display_library(&facts.library_name, &facts.library_id),
                    library.facet.as_str()
                )),
            );
        }
    }

    // The destination root: the requested one, or the title's own when the
    // request left the root alone.
    let destination_root_id = match destination.root_id.as_deref() {
        Some(root_id) => root_id.to_string(),
        None => facts.root_id.clone(),
    };
    if let Some(requested_root) = destination.root_id.as_deref()
        && let Some(library) = destination_library
        && !library
            .root_ids
            .iter()
            .any(|root_id| root_id == requested_root)
    {
        return classified(
            TitleLocationClass::Incompatible,
            destination_root_id,
            Some(reason_codes::ROOT_NOT_IN_DESTINATION_LIBRARY),
            Some(format!(
                "root {requested_root} is not configured on {}",
                display_library(&library.library_name, &library.library_id)
            )),
        );
    }

    // 2. No-op: already exactly where the request asks for.
    if !crosses_libraries && destination_root_id == facts.root_id {
        return classified(
            TitleLocationClass::NoOp,
            destination_root_id,
            Some(reason_codes::ALREADY_AT_DESTINATION),
            Some(format!(
                "\"{}\" already lives on this root",
                facts.title_name
            )),
        );
    }

    // 3. Blocked: something must be resolved or deselected before the job starts.
    if let Some(detail) = facts.active_work.as_deref() {
        return classified(
            TitleLocationClass::NeedsResolution,
            destination_root_id,
            Some(reason_codes::ACTIVE_DOWNLOAD_OR_IMPORT),
            Some(detail.to_string()),
        );
    }
    if let Some(operation_id) = facts.owned_by_operation.as_deref() {
        return classified(
            TitleLocationClass::NeedsResolution,
            destination_root_id,
            Some(reason_codes::OWNED_BY_LOCATION_OPERATION),
            Some(format!(
                "location operation {operation_id} is already working on \"{}\"",
                facts.title_name
            )),
        );
    }
    if let Some(detail) = facts.unresolved.as_deref() {
        return classified(
            TitleLocationClass::NeedsResolution,
            destination_root_id,
            None,
            Some(detail.to_string()),
        );
    }
    // A title with files but no folder match has nothing to move *from*; the
    // repair for that is US1's change-folder flow, not a move.
    if facts.has_tracked_files() && !facts.has_folder_match {
        return classified(
            TitleLocationClass::NeedsResolution,
            destination_root_id,
            Some(reason_codes::NO_FOLDER_MATCH),
            Some(format!(
                "\"{}\" has tracked files but owns no folder; correct the folder match first",
                facts.title_name
            )),
        );
    }

    // 4. Catalog-only fast path (FR-076): nothing on disk, so no move-mode
    //    choice and no filesystem work.
    if !facts.has_tracked_files() {
        return classified(
            TitleLocationClass::CatalogOnly,
            destination_root_id,
            Some(reason_codes::NO_TRACKED_FILES),
            Some(format!(
                "\"{}\" has no tracked files on disk, so only its catalog record changes",
                facts.title_name
            )),
        );
    }

    // 5. The move.
    let class = if crosses_libraries {
        TitleLocationClass::CrossLibraryTransfer
    } else {
        TitleLocationClass::RootMove
    };
    classified(class, destination_root_id, None, None)
}

fn display_library(name: &str, id: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        id.to_string()
    } else {
        format!("\"{name}\"")
    }
}

/// Classify a whole selection, counting every title exactly once (SC-005).
///
/// The returned list preserves selection order and has exactly one entry per
/// input, so a caller can never lose a title between the preview and the plan.
pub fn classify_selection(
    titles: &[TitleClassificationFacts],
    destination: &DestinationRequest,
    destination_library: Option<&DestinationLibraryFacts>,
) -> SelectionClassification {
    let mut counts = ClassificationCounts::default();
    let classified: Vec<TitleClassification> = titles
        .iter()
        .map(|facts| {
            let classification = classify_title(facts, destination, destination_library);
            match classification.class {
                TitleLocationClass::CrossLibraryTransfer => counts.cross_library_transfer += 1,
                TitleLocationClass::RootMove => counts.root_move += 1,
                TitleLocationClass::NoOp => counts.no_op += 1,
                TitleLocationClass::CatalogOnly => counts.catalog_only += 1,
                TitleLocationClass::Incompatible => counts.incompatible += 1,
                TitleLocationClass::NeedsResolution => counts.needs_resolution += 1,
            }
            classification
        })
        .collect();

    SelectionClassification {
        titles: classified,
        counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_file_bearing_classes_move_bytes() {
        assert!(TitleLocationClass::CrossLibraryTransfer.moves_files());
        assert!(TitleLocationClass::RootMove.moves_files());
        for class in [
            TitleLocationClass::NoOp,
            TitleLocationClass::CatalogOnly,
            TitleLocationClass::Incompatible,
            TitleLocationClass::NeedsResolution,
        ] {
            assert!(
                !class.moves_files(),
                "{} must not move files",
                class.as_str()
            );
        }
    }

    #[test]
    fn unresolved_or_incompatible_titles_block_the_start() {
        assert!(TitleLocationClass::Incompatible.blocks_start());
        assert!(TitleLocationClass::NeedsResolution.blocks_start());
        assert!(!TitleLocationClass::RootMove.blocks_start());

        let clean = ClassificationCounts {
            root_move: 3,
            no_op: 1,
            ..ClassificationCounts::default()
        };
        assert_eq!(clean.total(), 4);
        assert!(!clean.blocks_start());

        let blocked = ClassificationCounts {
            needs_resolution: 1,
            ..clean
        };
        assert_eq!(blocked.total(), 5);
        assert!(blocked.blocks_start());
    }

    fn movie_facts(id: &str, root: &str) -> TitleClassificationFacts {
        TitleClassificationFacts::new(id, MediaFacet::Movie, "lib-movies", root)
            .with_library_name("Movies")
            .with_tracked_files(3)
    }

    fn movies_library(root_ids: &[&str]) -> DestinationLibraryFacts {
        DestinationLibraryFacts {
            library_id: "lib-movies".to_string(),
            library_name: "Movies".to_string(),
            facet: MediaFacet::Movie,
            root_ids: root_ids.iter().map(|id| (*id).to_string()).collect(),
        }
    }

    /// US2.3: a bulk selection mixing titles on root A and root B with B as the
    /// destination classifies A-titles as moves and B-titles as no-ops, and
    /// omits nothing (FR-015, SC-005).
    #[test]
    fn bulk_root_move_classifies_every_title_exactly_once() {
        let titles = vec![
            movie_facts("on-a-1", "root-a"),
            movie_facts("on-b-1", "root-b"),
            movie_facts("on-a-2", "root-a"),
        ];
        let destination = DestinationRequest::to_root("root-b");
        let library = movies_library(&["root-a", "root-b"]);

        let result = classify_selection(&titles, &destination, Some(&library));

        assert_eq!(result.titles.len(), titles.len());
        assert_eq!(result.counts.total(), titles.len() as i64);
        assert_eq!(result.counts.root_move, 2);
        assert_eq!(result.counts.no_op, 1);
        assert!(!result.blocks_start());
        assert_eq!(
            result.title_ids_in(TitleLocationClass::RootMove),
            vec!["on-a-1".to_string(), "on-a-2".to_string()]
        );
        assert_eq!(
            result
                .classification_of("on-b-1")
                .expect("title present")
                .reason_code
                .as_deref(),
            Some(reason_codes::ALREADY_AT_DESTINATION)
        );
        // Every classified title carries the destination it would end up on.
        for title in &result.titles {
            assert_eq!(title.destination_root_id, "root-b");
            assert_eq!(title.destination_library_id, "lib-movies");
        }
    }

    /// US2.4 / FR-076: a monitored title with no tracked files is catalog-only
    /// and never enters move-mode selection.
    #[test]
    fn fileless_monitored_title_is_catalog_only() {
        let facts = movie_facts("fileless", "root-a")
            .with_tracked_files(0)
            .with_monitored(true);
        let library = movies_library(&["root-a", "root-b"]);

        let classification = classify_title(
            &facts,
            &DestinationRequest::to_root("root-b"),
            Some(&library),
        );

        assert_eq!(classification.class, TitleLocationClass::CatalogOnly);
        assert!(!classification.class.moves_files());
        assert!(!classification.blocks_start());
        assert_eq!(
            classification.reason_code.as_deref(),
            Some(reason_codes::NO_TRACKED_FILES)
        );
    }

    /// FR-086: an active download or import blocks the title, identifies it for
    /// deselection, and — because the import is about to land files in the old
    /// root — outranks the fileless fast path.
    #[test]
    fn active_download_blocks_a_title_even_when_it_is_fileless() {
        let titles = vec![
            movie_facts("busy", "root-a")
                .with_tracked_files(0)
                .with_active_work("a download for \"busy\" is still importing"),
            movie_facts("free", "root-a"),
        ];
        let library = movies_library(&["root-a", "root-b"]);

        let result = classify_selection(&titles, &DestinationRequest::to_root("root-b"), Some(&library));

        assert_eq!(result.counts.needs_resolution, 1);
        assert_eq!(result.counts.catalog_only, 0);
        assert_eq!(result.counts.root_move, 1);
        assert!(result.blocks_start(), "FR-016: the job must not start");
        assert_eq!(result.blocking_title_ids(), vec!["busy".to_string()]);
        assert_eq!(
            result
                .classification_of("busy")
                .expect("title present")
                .reason_code
                .as_deref(),
            Some(reason_codes::ACTIVE_DOWNLOAD_OR_IMPORT)
        );
    }

    /// FR-084: a title another operation already owns needs resolution rather
    /// than silently joining a second operation.
    #[test]
    fn title_owned_by_another_operation_needs_resolution() {
        let facts = movie_facts("owned", "root-a").with_owned_by_operation("op-1");
        let library = movies_library(&["root-a", "root-b"]);

        let classification = classify_title(
            &facts,
            &DestinationRequest::to_root("root-b"),
            Some(&library),
        );

        assert_eq!(classification.class, TitleLocationClass::NeedsResolution);
        assert_eq!(
            classification.reason_code.as_deref(),
            Some(reason_codes::OWNED_BY_LOCATION_OPERATION)
        );
        assert!(
            classification
                .reason
                .as_deref()
                .expect("reason")
                .contains("op-1")
        );
    }

    /// FR-017: an incompatible destination explains itself by naming the source
    /// library and facet.
    #[test]
    fn movie_into_series_library_is_incompatible_and_names_the_source() {
        let facts = movie_facts("a-movie", "root-a").with_name("A Movie");
        let series_library = DestinationLibraryFacts {
            library_id: "lib-series".to_string(),
            library_name: "Series".to_string(),
            facet: MediaFacet::Series,
            root_ids: vec!["root-s".to_string()],
        };

        let classification = classify_title(
            &facts,
            &DestinationRequest::to_library_root("lib-series", "root-s"),
            Some(&series_library),
        );

        assert_eq!(classification.class, TitleLocationClass::Incompatible);
        assert!(classification.blocks_start());
        let reason = classification.reason.expect("reason");
        assert!(reason.contains("Movies"), "names the source library: {reason}");
        assert!(reason.contains("movie"), "names the source facet: {reason}");
    }

    /// Series↔anime crossover is supported (spec Boundaries, FR-057); the
    /// movie/episodic boundary is not.
    #[test]
    fn facet_compatibility_matches_the_documented_boundaries() {
        assert!(facets_are_compatible(&MediaFacet::Series, &MediaFacet::Anime));
        assert!(facets_are_compatible(&MediaFacet::Anime, &MediaFacet::Series));
        assert!(facets_are_compatible(&MediaFacet::Movie, &MediaFacet::Movie));
        assert!(!facets_are_compatible(&MediaFacet::Movie, &MediaFacet::Series));
        assert!(!facets_are_compatible(&MediaFacet::Anime, &MediaFacet::Movie));
    }

    /// A root that belongs to another library is rejected rather than silently
    /// reinterpreted.
    #[test]
    fn root_outside_the_destination_library_is_incompatible() {
        let facts = movie_facts("a-movie", "root-a");
        let library = movies_library(&["root-a", "root-b"]);

        let classification = classify_title(
            &facts,
            &DestinationRequest::to_root("root-elsewhere"),
            Some(&library),
        );

        assert_eq!(classification.class, TitleLocationClass::Incompatible);
        assert_eq!(
            classification.reason_code.as_deref(),
            Some(reason_codes::ROOT_NOT_IN_DESTINATION_LIBRARY)
        );
    }

    /// A selection spanning three source libraries into one destination
    /// classifies 100% of titles into exactly one class (SC-005).
    #[test]
    fn selection_spanning_three_libraries_omits_nothing() {
        let titles = vec![
            TitleClassificationFacts::new("m1", MediaFacet::Movie, "lib-a", "root-a")
                .with_library_name("Movies A")
                .with_tracked_files(1),
            TitleClassificationFacts::new("s1", MediaFacet::Series, "lib-b", "root-b")
                .with_library_name("Series B")
                .with_tracked_files(1),
            TitleClassificationFacts::new("a1", MediaFacet::Anime, "lib-c", "root-c")
                .with_library_name("Anime C")
                .with_tracked_files(0),
        ];
        let destination_library = DestinationLibraryFacts {
            library_id: "lib-b".to_string(),
            library_name: "Series B".to_string(),
            facet: MediaFacet::Series,
            root_ids: vec!["root-b".to_string()],
        };

        let result = classify_selection(
            &titles,
            &DestinationRequest::to_library_root("lib-b", "root-b"),
            Some(&destination_library),
        );

        assert_eq!(result.counts.total(), 3);
        // The movie cannot enter an episodic library; the series is already
        // there; the fileless anime converts facet as a catalog-only change.
        assert_eq!(result.counts.incompatible, 1);
        assert_eq!(result.counts.no_op, 1);
        assert_eq!(result.counts.catalog_only, 1);
        assert!(result.blocks_start());
        assert_eq!(result.to_classified_titles().len(), 3);
    }
}
