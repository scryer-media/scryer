//! Cost accounting for the background acquisition walk.
//!
//! Every figure here is wall time measured with [`std::time::Instant`] around
//! one section of the walk, plus the process CPU time a walk consumed where
//! the platform reports it. The figures are what the walk's time share and
//! the derivation cache are judged by, so each section is named for what it
//! does rather than for the function that happens to run it.

/// Wall time of one target-derivation sub-pass, labelled by `pass`.
pub(crate) const DERIVATION_SECONDS: &str = "scryer_background_acquisition_derivation_seconds";

/// Wall time of one section of a walk, labelled by `section`.
pub(crate) const WALK_SECTION_SECONDS: &str = "scryer_background_acquisition_walk_section_seconds";

/// Wall time of one whole walk, from the first derivation read to the
/// summary line.
pub(crate) const WALK_SECONDS: &str = "scryer_background_acquisition_walk_seconds";

/// Process CPU time consumed while one walk ran. Other work running at the
/// same time is included, so this is an upper bound on the walk's own cost.
pub(crate) const WALK_PROCESS_CPU_SECONDS: &str =
    "scryer_background_acquisition_walk_process_cpu_seconds";

/// Entries held by the in-memory acquisition target cache.
pub(crate) const TARGET_CACHE_ENTRIES: &str = "scryer_background_acquisition_target_cache_entries";

/// Estimated bytes held by the in-memory acquisition target cache.
pub(crate) const TARGET_CACHE_BYTES: &str = "scryer_background_acquisition_target_cache_bytes";

/// How the walk obtained its target set, labelled by `source`.
pub(crate) const TARGET_CACHE_READS_TOTAL: &str =
    "scryer_background_acquisition_target_cache_reads_total";

/// Search results the walk evaluated, labelled by `outcome` (`evaluated`, or
/// the pre-filter that dropped the result before parsing).
pub(crate) const EVALUATED_RESULTS_TOTAL: &str =
    "scryer_background_acquisition_evaluated_results_total";

/// Derivation sub-passes, as the `pass` label of [`DERIVATION_SECONDS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DerivationPass {
    /// The missing-scope anti-join and its availability gates.
    Missing,
    /// The quality-tier cutoff sweep.
    CutoffQuality,
    /// The landed-bar format-score cutoff pass.
    FormatScoring,
    /// Location locks and paused scopes.
    Filters,
}

impl DerivationPass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::CutoffQuality => "cutoff_quality",
            Self::FormatScoring => "format_scoring",
            Self::Filters => "filters",
        }
    }
}

/// Walk sections, as the `section` label of [`WALK_SECTION_SECONDS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalkSection {
    /// Reading (or patching) the target set the walk rotates over.
    Targets,
    /// Choosing the batch from the rotation cursors.
    BatchSelection,
    /// One scope's indexer search, including scoring of its results.
    ScopeSearch,
    /// Admission evaluation of one scope's scored results.
    Evaluation,
    /// Coverage and attempt bookkeeping written after one scope's search.
    LearningWrites,
}

impl WalkSection {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Targets => "targets",
            Self::BatchSelection => "batch_selection",
            Self::ScopeSearch => "scope_search",
            Self::Evaluation => "evaluation",
            Self::LearningWrites => "learning_writes",
        }
    }
}

pub(crate) fn record_derivation(pass: DerivationPass, elapsed: std::time::Duration) {
    metrics::histogram!(DERIVATION_SECONDS, "pass" => pass.as_str()).record(elapsed.as_secs_f64());
}

/// Records the wall time of one walk section when dropped.
pub(crate) struct SectionTimer {
    section: WalkSection,
    started: std::time::Instant,
}

impl SectionTimer {
    pub(crate) fn start(section: WalkSection) -> Self {
        Self {
            section,
            started: std::time::Instant::now(),
        }
    }
}

impl Drop for SectionTimer {
    fn drop(&mut self) {
        metrics::histogram!(WALK_SECTION_SECONDS, "section" => self.section.as_str())
            .record(self.started.elapsed().as_secs_f64());
    }
}

/// CPU time this process has consumed so far, user plus system, where the
/// platform reports it.
pub(crate) fn process_cpu_seconds() -> Option<f64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: `getrusage` writes a complete `rusage` into the pointer on
        // success and reads nothing from it; the value is only read after a
        // zero return.
        let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        if status != 0 {
            return None;
        }
        // SAFETY: a zero return means the struct was filled in.
        let usage = unsafe { usage.assume_init() };
        let seconds = |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1_000_000.0;
        Some(seconds(usage.ru_utime) + seconds(usage.ru_stime))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

pub fn describe_walk_metrics() {
    metrics::describe_histogram!(
        DERIVATION_SECONDS,
        metrics::Unit::Seconds,
        "Wall time of one background acquisition target-derivation sub-pass, labelled by pass (missing, cutoff_quality, format_scoring, filters)."
    );
    metrics::describe_histogram!(
        WALK_SECTION_SECONDS,
        metrics::Unit::Seconds,
        "Wall time of one section of a background acquisition walk, labelled by section (targets, batch_selection, scope_search, evaluation, learning_writes)."
    );
    metrics::describe_histogram!(
        WALK_SECONDS,
        metrics::Unit::Seconds,
        "Wall time of one background acquisition walk."
    );
    metrics::describe_histogram!(
        WALK_PROCESS_CPU_SECONDS,
        metrics::Unit::Seconds,
        "Process CPU time consumed while one background acquisition walk ran; concurrent work is included, so this bounds the walk's own cost from above."
    );
    metrics::describe_gauge!(
        TARGET_CACHE_ENTRIES,
        "Acquisition targets held by the in-memory target cache the walk rotates over."
    );
    metrics::describe_gauge!(
        TARGET_CACHE_BYTES,
        metrics::Unit::Bytes,
        "Estimated bytes held by the in-memory acquisition target cache."
    );
    metrics::describe_counter!(
        TARGET_CACHE_READS_TOTAL,
        "Walk reads of the acquisition target set, labelled by source (built, patched, reused)."
    );
    metrics::describe_counter!(
        EVALUATED_RESULTS_TOTAL,
        "Search results seen by background release scoring, labelled by outcome (scored, duplicate, blocklisted, over_cap)."
    );
}
