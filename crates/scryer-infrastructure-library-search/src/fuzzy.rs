//! The bounded-distance lane of the title name index.
//!
//! The exact lanes — same match form, same romanization, equal under a
//! collation profile, same lookup key, same external id — are SQL equality
//! over `title_search_terms` and are exhaustive by construction. What they
//! cannot answer is "within `n` edits of", which is not an equality and which
//! no index on a text column can serve. That lane lives here, in a tantivy
//! index built from the same projection rows, so there is exactly one place a
//! fuzzy title lookup happens for the UI library search, for release matching
//! and for import matching.
//!
//! Operational contract:
//!
//! * The index is **derived state**. It is never the source of truth and
//!   losing it costs a rebuild and nothing else. Backups are table-scoped —
//!   they carry rows out of the catalog, never files out of the data
//!   directory — so this directory is excluded by construction, and a
//!   restore bumps the projection generation, which invalidates whatever
//!   index the restoring installation happened to have. Opening and reads
//!   wait for required rebuilds. An unavailable index is an error, never
//!   evidence that there are no competing titles.
//! * The directory is opened through [`MmapDirectory`] only. No other
//!   directory implementation is compiled in.
//! * A writer is created per batch and dropped when the batch commits. A
//!   separate process lock protects the directory throughout its lifetime,
//!   including recovery. The OS releases ownership after a crash.
//! * `scryer_fuzzy.json` beside the segments records the schema version this
//!   build writes and the projection stamp the contents were built from. A
//!   mismatch, a missing file or an unreadable one requires a blocking rebuild.
//! * Incremental freshness rides on `title_search_index_queue`, which the
//!   projection writer fills in the *same database transaction* as the
//!   projection row. A queued title is therefore never lost to a rollback,
//!   and draining is idempotent: a drain rewrites a title's documents from
//!   the projection as it stands at drain time.
//! * Reads drain the queue at most once per [`QUEUE_DRAIN_INTERVAL`], so a
//!   burst of reads during a burst of writes costs one commit, not one per
//!   read. The stamp check and any rebuild it calls for still run on every
//!   read. [`TitleFuzzyIndex::sync`] drains unconditionally for a caller that
//!   must see a write it just made.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use scryer_application::{AppError, AppResult};
use tantivy::collector::{DocSetCollector, TopDocs};
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, FuzzyTermQuery, Occur, Query, QueryClone, TermQuery};
use tantivy::schema::{
    FAST, Field, INDEXED, IndexRecordOption, STORED, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{
    LowerCaser, NgramTokenizer, PreTokenizedString, RawTokenizer, SimpleTokenizer, TextAnalyzer,
    Token,
};
use tantivy::{Index, IndexReader, ReloadPolicy, TantivyDocument, Term};

/// Bumped whenever the schema or the analysis below changes in a way that
/// makes existing segments answer differently. A changed value is a rebuild.
///
/// 2: name rows also index the grams of their lenient form, which the
/// contains lane reads.
pub const FUZZY_SCHEMA_VERSION: u32 = 2;

const META_FILE: &str = "scryer_fuzzy.json";
/// The directory name under the data dir. Named so an operator can delete it
/// without wondering what it is.
pub const FUZZY_INDEX_DIR: &str = "title-fuzzy-index";

/// Terms per writer batch, both for the paged rebuild and for queue drains.
const BATCH_TERMS: usize = 20_000;
/// Titles read out of the queue per drain pass.
const QUEUE_DRAIN_BATCH: i64 = 512;
/// The most queue batches one search drains before it answers.
const SEARCH_DRAIN_MAX_BATCHES: usize = 4;
/// The longest a fuzzy read may lag a write the database has accepted.
///
/// Every drain is a tantivy commit: a new segment, a merge sooner or later,
/// and files replaced on disk. Reads arrive many times a minute from the
/// acquisition walk, RSS matching and import checks, and while a scan is
/// writing titles the queue is rarely empty, so draining on every read made
/// the commit rate the read rate. Ten seconds caps that at six commits a
/// minute however busy reads are. It only delays the bounded-distance lane:
/// the exact lanes read the projection in SQL and see a write as soon as it
/// commits, and ten seconds is well inside the cadence of the background work
/// that reads this lane.
pub const QUEUE_DRAIN_INTERVAL: Duration = Duration::from_secs(10);
/// Writer heap per batch. Tantivy's floor is 15 MB; a batch is small and
/// short-lived, so nothing is gained by giving it more.
const WRITER_HEAP_BYTES: usize = 15_000_000;
/// The widest distance tantivy's automaton can be asked for. Beyond it the
/// gram filter below carries the lane.
const MAX_AUTOMATON_DISTANCE: u8 = 2;
/// Leads every gram of a name's lenient form in the `grams` field.
///
/// That field already holds the grams of the match form, which the resolver's
/// gram lane reads. The lenient grams share the field rather than take one of
/// their own because a schema with another field no longer opens the directory
/// an installation already has. No name contains this character once it is
/// normalized and the gram tokenizer never emits it, so the two sets of grams
/// cannot meet: a resolver query never names a marked gram and a contains query
/// never names an unmarked one.
const LENIENT_GRAM_MARK: char = '\u{1}';
/// The longest gram of the lenient form that is indexed. A query at least this
/// long asks for every gram of this length it holds; a shorter one is itself a
/// gram.
const LENIENT_GRAM_MAX_CHARS: usize = 3;

/// The projection row as the index stores it. Every field here is already on
/// `title_search_terms`; the index adds no derivation of its own, which is
/// what keeps it rebuildable from the database alone.
#[derive(Clone, Debug)]
pub struct IndexedTerm {
    pub term_id: i64,
    pub title_id: String,
    pub facet: String,
    pub term_kind: String,
    pub weight: i64,
    pub literal_term: String,
    pub match_term: String,
    pub normalized_term: String,
    pub script: String,
    pub numbers_key: String,
    pub char_length: i64,
}

/// One queued title, with the queue row that claimed it.
#[derive(Clone, Debug)]
pub struct QueuedTitle {
    pub seq: i64,
    pub title_id: String,
}

/// What the projection stamp says the index should have been built from.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectionStamp {
    pub collation_version: String,
    pub projection_generation: i64,
}

/// The database side of the index, dialect-agnostic on purpose: SQLite and
/// PostgreSQL implement it over the same projection tables, so the index and
/// every behaviour built on it are identical on both.
#[async_trait]
pub trait TitleTermSource: Send + Sync {
    /// Projected terms with `term_id > after_term_id`, ordered by `term_id`,
    /// at most `limit` of them. Paging by key, not by offset, so a rebuild
    /// does not degrade quadratically on a large library.
    async fn page_terms(&self, after_term_id: i64, limit: i64) -> AppResult<Vec<IndexedTerm>>;
    /// Every projected term of these titles.
    async fn terms_for_titles(&self, title_ids: &[String]) -> AppResult<Vec<IndexedTerm>>;
    /// Oldest queued titles first.
    async fn queued_titles(&self, limit: i64) -> AppResult<Vec<QueuedTitle>>;
    /// Drop exactly these queue rows. Rows enqueued after the drain read them
    /// carry later sequence numbers and survive, so a write racing a drain is
    /// picked up by the next one rather than lost.
    async fn clear_queued(&self, seqs: &[i64]) -> AppResult<()>;
    async fn projection_stamp(&self) -> AppResult<ProjectionStamp>;
    /// Which SQL the projection's database speaks, for callers that hand the
    /// index's answer back to it.
    fn dialect(&self) -> TermSourceDialect;
    /// The contains lane's exact half. Of the rows `term_ids` names, those
    /// that are full names in `facets` and whose lenient form contains
    /// `lenient_query`, as one rank per title: the lowest of each row's lane
    /// rank plus its weight, where a row equal to the query is in the exact
    /// lane, one that starts with it in the prefix lane and any other in the
    /// contains lane. `term_ids` is ascending and bounded by the caller.
    async fn contains_ranks(
        &self,
        term_ids: &[i64],
        lenient_query: &str,
        facets: &[&str],
        lanes: ContainsLaneRanks,
    ) -> AppResult<Vec<(String, i64)>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermSourceDialect {
    Sqlite,
    Postgres,
}

/// The base rank of each direct lane.
#[derive(Clone, Copy, Debug)]
pub struct ContainsLaneRanks {
    pub exact: i64,
    pub prefix: i64,
    pub contains: i64,
}

/// How many candidate rows one statement of the contains lane tests.
const CONTAINS_VERIFY_BATCH: usize = 5_000;

#[derive(Clone, Copy)]
struct Fields {
    term_id: Field,
    title_id: Field,
    facet: Field,
    script: Field,
    numbers_key: Field,
    term_kind: Field,
    token_row: Field,
    weight: Field,
    char_length: Field,
    match_raw: Field,
    literal_raw: Field,
    lenient_raw: Field,
    grams: Field,
}

const TOKENIZER_RAW: &str = "scryer_raw";
const TOKENIZER_WORDS: &str = "scryer_words";
const TOKENIZER_GRAMS: &str = "scryer_grams";

fn raw_text_options(tokenizer: &str) -> TextOptions {
    TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(tokenizer)
            .set_index_option(IndexRecordOption::Basic),
    )
}

fn build_schema() -> (Schema, Fields) {
    let mut schema = Schema::builder();
    let fields = Fields {
        term_id: schema.add_i64_field("term_id", INDEXED | STORED | FAST),
        title_id: schema.add_text_field("title_id", raw_text_options(TOKENIZER_RAW) | STORED),
        facet: schema.add_text_field("facet", raw_text_options(TOKENIZER_RAW)),
        script: schema.add_text_field("script", raw_text_options(TOKENIZER_RAW)),
        numbers_key: schema.add_text_field("numbers_key", raw_text_options(TOKENIZER_RAW)),
        term_kind: schema.add_text_field("term_kind", raw_text_options(TOKENIZER_RAW)),
        token_row: schema.add_text_field("token_row", raw_text_options(TOKENIZER_RAW)),
        weight: schema.add_i64_field("weight", STORED | FAST),
        char_length: schema.add_i64_field("char_length", STORED | FAST),
        match_raw: schema.add_text_field("match_raw", raw_text_options(TOKENIZER_RAW)),
        literal_raw: schema.add_text_field("literal_raw", raw_text_options(TOKENIZER_RAW)),
        // Stored as well as indexed: the UI lane re-measures the edit
        // distance against the token it actually matched, so the precision
        // rules that used to be SQL predicates still apply.
        lenient_raw: schema.add_text_field("lenient_raw", raw_text_options(TOKENIZER_RAW) | STORED),
        grams: schema.add_text_field("grams", raw_text_options(TOKENIZER_GRAMS)),
    };
    (schema.build(), fields)
}

fn register_tokenizers(index: &Index) -> AppResult<()> {
    let manager = index.tokenizers();
    manager.register(
        TOKENIZER_RAW,
        TextAnalyzer::builder(RawTokenizer::default()).build(),
    );
    manager.register(
        TOKENIZER_WORDS,
        TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(LowerCaser)
            .build(),
    );
    // Both sizes, so one field serves two gram lanes: bigrams for scripts
    // that carry a word in two characters, trigrams for everything else.
    manager.register(
        TOKENIZER_GRAMS,
        TextAnalyzer::builder(
            NgramTokenizer::new(2, 3, false)
                .map_err(|error| AppError::Repository(error.to_string()))?,
        )
        .build(),
    );
    Ok(())
}

/// The in-process writer lock, in the form every blocking writer task keeps a
/// share of. See [`TitleFuzzyIndex::write_lock`].
type WriteGuard = Arc<tokio::sync::OwnedMutexGuard<()>>;

pub struct TitleFuzzyIndex {
    dir: PathBuf,
    index: Index,
    reader: IndexReader,
    fields: Fields,
    source: Arc<dyn TitleTermSource>,
    /// One writer at a time, always. Tantivy enforces this with a lock file;
    /// taking it in process turns a hard error into a wait.
    ///
    /// The guard is *owned* and shared into every blocking task that creates a
    /// writer, so it outlives the awaiting future rather than the caller's
    /// stack. A caller whose future is dropped mid-await — which is what a
    /// disconnected GraphQL client does — therefore cannot release this while
    /// a detached blocking task still holds tantivy's own lock file; without
    /// that, the next reader's `writer_with_num_threads` fails outright and a
    /// match, import or search fails with it.
    write_lock: Arc<tokio::sync::Mutex<()>>,
    /// What [`META_FILE`] says, kept in memory so a read does not do a
    /// blocking file read on the async runtime. The file remains the durable
    /// record: this is seeded from it at open and only changes in
    /// [`Self::write_meta`] and [`Self::remove_meta`], both under
    /// [`Self::write_lock`].
    meta: std::sync::RwLock<Option<ProjectionStamp>>,
    rebuild_complete: AtomicBool,
    /// When the last drain that emptied the queue started. Only read and
    /// written under [`Self::write_lock`].
    last_drain: std::sync::Mutex<Option<Instant>>,
    /// See [`QUEUE_DRAIN_INTERVAL`].
    drain_interval: Duration,
    // Keep directory ownership until the index and reader have been dropped.
    _ownership: std::fs::File,
}

/// How fresh a read needs the index to be with respect to the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Freshness {
    /// Drain whatever is queued.
    Current,
    /// Drain only if the last drain is older than the drain interval.
    Bounded,
    /// Drain what was queued when the read arrived, and no more than
    /// [`SEARCH_DRAIN_MAX_BATCHES`] of it: a read that kept draining while a
    /// scan kept writing titles would hold the index for as long as the scan
    /// ran. What it leaves is the next read's.
    Arrived,
}

/// A resolver-lane request: one bucket of the name index, the same
/// `(facet, script, numbers)` bucket the exact lanes use, and a distance.
#[derive(Clone, Debug)]
pub struct ResolverFuzzyQuery<'a> {
    pub facet: Option<&'a str>,
    pub script: &'a str,
    pub numbers_key: &'a str,
    pub match_term: &'a str,
    pub distance: u8,
    pub limit: usize,
}

/// A UI-lane hit: the same shape the SQL typo lane produced, so the ranking
/// arithmetic around it is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiFuzzyHit {
    pub title_id: String,
    /// The query token this hit answers.
    pub token_key: String,
    /// The projected token that matched, so the caller can re-measure the
    /// distance and apply the boundary rules the lane has always applied.
    pub matched_term: String,
    pub weight: i64,
}

impl TitleFuzzyIndex {
    /// Open the index, completing any required recovery and rebuild first.
    /// Failure to provide the complete index is an error, never an empty match.
    pub async fn open(data_dir: &Path, source: Arc<dyn TitleTermSource>) -> AppResult<Arc<Self>> {
        Self::open_with_drain_interval(data_dir, source, QUEUE_DRAIN_INTERVAL).await
    }

    /// [`Self::open`] with a different bound on how far reads may lag the
    /// queue. `Duration::ZERO` drains on every read.
    pub async fn open_with_drain_interval(
        data_dir: &Path,
        source: Arc<dyn TitleTermSource>,
        drain_interval: Duration,
    ) -> AppResult<Arc<Self>> {
        let dir = data_dir.join(FUZZY_INDEX_DIR);
        Self::open_inner(dir, source, drain_interval).await
    }

    async fn open_inner(
        dir: PathBuf,
        source: Arc<dyn TitleTermSource>,
        drain_interval: Duration,
    ) -> AppResult<Arc<Self>> {
        let (_, fields) = build_schema();
        let open_dir = dir.clone();
        let (ownership, index, reader) =
            tokio::task::spawn_blocking(move || open_owned_index(&open_dir))
                .await
                .map_err(|error| AppError::Repository(error.to_string()))??;

        let meta = read_meta_file(&dir.join(META_FILE));
        let handle = Arc::new(Self {
            dir,
            _ownership: ownership,
            index,
            reader,
            fields,
            source,
            write_lock: Arc::new(tokio::sync::Mutex::new(())),
            meta: std::sync::RwLock::new(meta),
            rebuild_complete: AtomicBool::new(false),
            last_drain: std::sync::Mutex::new(None),
            drain_interval,
        });

        let stamp = handle.source.projection_stamp().await?;
        if handle.read_meta().is_some_and(|meta| meta == stamp) {
            handle.rebuild_complete.store(true, Ordering::SeqCst);
        } else {
            handle.rebuild(stamp).await?;
        }
        handle.sync().await?;
        Ok(handle)
    }

    /// Drop every document and page the projection back in, one writer per
    /// batch. Readers wait for the complete replacement before proceeding.
    pub async fn rebuild(&self, stamp: ProjectionStamp) -> AppResult<()> {
        let guard = self.write_guard().await;
        self.rebuild_locked(&guard, stamp).await
    }

    /// Take the writer lock in a form a blocking task can keep alive on its
    /// own. Every path that may create a tantivy writer goes through here.
    async fn write_guard(&self) -> WriteGuard {
        Arc::new(self.write_lock.clone().lock_owned().await)
    }

    async fn rebuild_locked(&self, guard: &WriteGuard, stamp: ProjectionStamp) -> AppResult<()> {
        // Set before anything is destroyed: a rebuild whose future is dropped
        // halfway leaves the index not ready and without a stamp, which is the
        // state the next reader already knows how to repair.
        self.rebuild_complete.store(false, Ordering::SeqCst);
        self.remove_meta()?;

        {
            let index = self.index.clone();
            let guard = guard.clone();
            tokio::task::spawn_blocking(move || -> AppResult<()> {
                // Dropped after the writer below, so the in-process lock is
                // held for exactly as long as a tantivy writer exists.
                let _guard = guard;
                let mut writer = index
                    .writer_with_num_threads::<TantivyDocument>(1, WRITER_HEAP_BYTES)
                    .map_err(|error| AppError::Repository(error.to_string()))?;
                writer
                    .delete_all_documents()
                    .map_err(|error| AppError::Repository(error.to_string()))?;
                writer
                    .commit()
                    .map_err(|error| AppError::Repository(error.to_string()))?;
                Ok(())
            })
            .await
            .map_err(|error| AppError::Repository(error.to_string()))??;
        }

        let mut after = 0i64;
        loop {
            let page = self.source.page_terms(after, BATCH_TERMS as i64).await?;
            if page.is_empty() {
                break;
            }
            after = page.iter().map(|term| term.term_id).max().unwrap_or(after);
            self.write_batch(guard, page, Vec::new()).await?;
        }

        // Anything enqueued while the rebuild ran is already reflected by the
        // page that read it, or is still queued and will be drained. Either
        // way the stamp below describes the projection the pages came from.
        self.write_meta(&stamp)?;
        self.reload()?;
        self.rebuild_complete.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Apply everything `title_search_index_queue` holds and check that the
    /// projection generation has not changed while draining it.
    ///
    /// Unconditional, unlike the drain a read does: a caller that must see a
    /// write it has just committed calls this first.
    pub async fn sync(&self) -> AppResult<()> {
        let guard = self.write_guard().await;
        self.sync_locked(&guard, Freshness::Current, Instant::now())
            .await
    }

    /// The read path's drain: rebuild if the stamp says so, and drain the
    /// queue only when the last drain is older than the drain interval.
    ///
    /// Nothing waits here. A read inside the interval skips the queue and
    /// searches what the last drain committed; the writer lock is held only
    /// for the stamp check and, at most once per interval, for one drain.
    async fn sync_for_read(&self, guard: &WriteGuard) -> AppResult<()> {
        self.sync_locked(guard, Freshness::Bounded, Instant::now())
            .await
    }

    /// Whether a drain that started at or after `now - interval` emptied the
    /// queue. Read under the writer lock.
    fn drained_within_interval(&self, now: Instant) -> bool {
        let last = *self
            .last_drain
            .lock()
            .expect("title index drain clock is never held across a panic");
        last.is_some_and(|last| now.saturating_duration_since(last) < self.drain_interval)
    }

    fn record_drain(&self, started: Instant) {
        *self
            .last_drain
            .lock()
            .expect("title index drain clock is never held across a panic") = Some(started);
    }

    /// `now` is when this drain started. Recording the start rather than the
    /// end keeps the bound honest: a title enqueued after the drain read the
    /// queue is at most one interval behind the next read that drains.
    async fn sync_locked(
        &self,
        guard: &WriteGuard,
        freshness: Freshness,
        now: Instant,
    ) -> AppResult<()> {
        let mut drained_batches = 0;
        loop {
            let stamp = self.source.projection_stamp().await?;
            if !self.ready() || !self.stamp_matches(&stamp) {
                self.rebuild_locked(guard, stamp).await?;
                continue;
            }
            if freshness == Freshness::Bounded && self.drained_within_interval(now) {
                return Ok(());
            }
            if freshness == Freshness::Arrived && drained_batches == SEARCH_DRAIN_MAX_BATCHES {
                return Ok(());
            }
            let queued = self.source.queued_titles(QUEUE_DRAIN_BATCH).await?;
            if queued.is_empty() {
                if self.source.projection_stamp().await? == stamp {
                    self.record_drain(now);
                    return Ok(());
                }
                continue;
            }
            let title_ids = queued
                .iter()
                .map(|entry| entry.title_id.clone())
                .collect::<Vec<_>>();
            let terms = self.source.terms_for_titles(&title_ids).await?;
            self.write_batch(guard, terms, title_ids).await?;
            self.reload()?;
            let seqs = queued.iter().map(|entry| entry.seq).collect::<Vec<_>>();
            self.source.clear_queued(&seqs).await?;
            drained_batches += 1;
            // A short batch was the whole queue as this read found it.
            if freshness == Freshness::Arrived && (queued.len() as i64) < QUEUE_DRAIN_BATCH {
                self.record_drain(now);
                return Ok(());
            }
        }
    }

    /// One writer, one commit, dropped. `replace_title_ids` are deleted first,
    /// so a rename cannot leave the previous spelling matchable.
    async fn write_batch(
        &self,
        guard: &WriteGuard,
        terms: Vec<IndexedTerm>,
        replace_title_ids: Vec<String>,
    ) -> AppResult<()> {
        if terms.is_empty() && replace_title_ids.is_empty() {
            return Ok(());
        }
        let index = self.index.clone();
        let fields = self.fields;
        let guard = guard.clone();
        tokio::task::spawn_blocking(move || -> AppResult<()> {
            // Dropped after the writer, so a dropped caller future cannot let
            // a second writer start while this one is still committing.
            let _guard = guard;
            let mut writer = index
                .writer_with_num_threads::<TantivyDocument>(1, WRITER_HEAP_BYTES)
                .map_err(|error| AppError::Repository(error.to_string()))?;
            for title_id in &replace_title_ids {
                writer.delete_term(Term::from_field_text(fields.title_id, title_id));
            }
            for chunk in terms.chunks(BATCH_TERMS) {
                for term in chunk {
                    writer
                        .add_document(document_for(&fields, term))
                        .map_err(|error| AppError::Repository(error.to_string()))?;
                }
            }
            writer
                .commit()
                .map_err(|error| AppError::Repository(error.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|error| AppError::Repository(error.to_string()))??;
        Ok(())
    }

    fn reload(&self) -> AppResult<()> {
        self.reader
            .reload()
            .map_err(|error| AppError::Repository(error.to_string()))
    }

    fn meta_path(&self) -> PathBuf {
        self.dir.join(META_FILE)
    }

    /// Whether the directory on disk was built from this projection stamp.
    /// This is the invalidation rule in one call: a reopen that answers
    /// `false` rebuilds, and answers `false` for a missing, unreadable or
    /// older-schema meta file too.
    pub fn stamp_matches(&self, stamp: &ProjectionStamp) -> bool {
        self.read_meta().is_some_and(|meta| &meta == stamp)
    }

    /// The cached stamp. Deliberately not a file read: this is on the path of
    /// every fuzzy read, and the file only changes under the write lock.
    fn read_meta(&self) -> Option<ProjectionStamp> {
        self.meta
            .read()
            .expect("title index meta cache is never held across a panic")
            .clone()
    }

    fn store_meta(&self, stamp: Option<ProjectionStamp>) {
        *self
            .meta
            .write()
            .expect("title index meta cache is never held across a panic") = stamp;
    }

    fn write_meta(&self, stamp: &ProjectionStamp) -> AppResult<()> {
        // `fs::write` follows a symlink; never let the stamp land elsewhere.
        reject_symlink(&self.meta_path())?;
        let payload = serde_json::json!({
            "schema_version": FUZZY_SCHEMA_VERSION,
            "stamp": stamp,
        });
        std::fs::write(
            self.meta_path(),
            serde_json::to_vec(&payload)
                .map_err(|error| AppError::Repository(error.to_string()))?,
        )
        .map_err(|error| AppError::Repository(error.to_string()))?;
        self.store_meta(Some(stamp.clone()));
        Ok(())
    }

    fn remove_meta(&self) -> AppResult<()> {
        // A rebuild that dies halfway must not leave a stamp claiming the
        // segments are complete. The cache is cleared first, so a failure to
        // remove the file cannot leave the process trusting a stamp it just
        // decided was wrong.
        self.store_meta(None);
        match std::fs::remove_file(self.meta_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AppError::Repository(error.to_string())),
        }
    }

    fn ready(&self) -> bool {
        self.rebuild_complete.load(Ordering::SeqCst)
    }

    /// The resolver lane: term ids of projected names within `distance` edits
    /// of `match_term`, inside the same bucket the exact lanes use.
    ///
    /// Returns term ids rather than hydrated candidates so the caller reads
    /// the candidate columns from `title_search_terms` — one source of truth
    /// for what a candidate *is*, whichever lane found it.
    pub async fn resolver_candidates(&self, query: ResolverFuzzyQuery<'_>) -> AppResult<Vec<i64>> {
        Ok(self
            .resolver_candidates_batch(std::slice::from_ref(&query))
            .await?
            .pop()
            .unwrap_or_default())
    }

    /// [`Self::resolver_candidates`] for several buckets at once, answered in
    /// order.
    ///
    /// The queue drain and the stamp check run once for the whole batch, and
    /// every query searches the same pinned generation. One release's anchors
    /// used to pay that check — two stamp reads and a queue poll — per anchor
    /// per facet, which was most of the index's statement count.
    pub async fn resolver_candidates_batch(
        &self,
        queries: &[ResolverFuzzyQuery<'_>],
    ) -> AppResult<Vec<Vec<i64>>> {
        let guard = self.write_guard().await;
        self.sync_for_read(&guard).await?;
        // Pin a complete generation before allowing another rebuild to start.
        let searcher = self.reader.searcher();
        drop(guard);
        let mut results = Vec::with_capacity(queries.len());
        for query in queries {
            results.push(self.search_resolver(&searcher, query).await?);
        }
        Ok(results)
    }

    async fn search_resolver(
        &self,
        searcher: &tantivy::Searcher,
        query: &ResolverFuzzyQuery<'_>,
    ) -> AppResult<Vec<i64>> {
        if query.match_term.is_empty() {
            return Ok(Vec::new());
        }
        let fields = self.fields;
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![
            (Occur::Must, term_clause(fields.script, query.script)),
            (
                Occur::Must,
                term_clause(fields.numbers_key, query.numbers_key),
            ),
            (Occur::Must, term_clause(fields.token_row, "0")),
        ];
        if let Some(facet) = query.facet {
            clauses.push((Occur::Must, term_clause(fields.facet, facet)));
        }
        // The projection also holds sort-title and slug rows; the resolver
        // lane matches on names the catalog answers to, exactly as the SQL
        // bucket does with its `term_kind IN (...)`.
        clauses.push((
            Occur::Must,
            Box::new(BooleanQuery::new(
                ["name", "alias", "tagged_alias"]
                    .into_iter()
                    .map(|kind| (Occur::Should, term_clause(fields.term_kind, kind)))
                    .collect(),
            )),
        ));

        let limit = query.limit.max(1);
        let bucket = format!(
            "facet={} script={} numbers={}",
            query.facet.unwrap_or("*"),
            query.script,
            query.numbers_key
        );
        // One search per lane, unioned. The lanes score on different scales —
        // the automaton's hits all carry the same constant score, the gram
        // lane's carry BM25 — so a single ranked search lets gram noise evict
        // an exact hit that shares no n-gram with the query. Separate searches
        // make that impossible without having to reason about relative scores.
        let mut term_ids = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for lane in spelling_lanes(&fields, query.match_term, query.distance) {
            let mut lane_clauses = clauses
                .iter()
                .map(|(occur, clause)| (*occur, clause.box_clone()))
                .collect::<Vec<_>>();
            lane_clauses.push((Occur::Must, lane));
            let boolean = BooleanQuery::new(lane_clauses);
            let searcher = searcher.clone();
            let found =
                tokio::task::spawn_blocking(move || -> tantivy::Result<(usize, Vec<i64>)> {
                    let docs =
                        searcher.search(&boolean, &TopDocs::with_limit(limit).order_by_score())?;
                    let matched = docs.len();
                    let mut lane_ids = Vec::with_capacity(matched);
                    for (_score, address) in docs {
                        let doc = searcher.doc::<TantivyDocument>(address)?;
                        if let Some(value) = doc
                            .get_first(fields.term_id)
                            .and_then(|value| value.as_i64())
                        {
                            lane_ids.push(value);
                        }
                    }
                    Ok((matched, lane_ids))
                })
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?
                .map_err(|error| AppError::Repository(error.to_string()))?;
            let (matched, lane_ids) = found;
            // A lane that filled its limit was cut off at an arbitrary point
            // in a ranking that means nothing to the caller. Returning what
            // fits would be read as "these are all the competing names", which
            // is exactly the wrong answer to be silent about.
            if matched >= limit {
                return Err(AppError::Repository(format!(
                    "title index lane for {bucket} returned its full limit of {limit} \
                     names at distance {}: the bounded-distance answer is incomplete \
                     and must not be read as an absence of competing titles",
                    query.distance
                )));
            }
            for term_id in lane_ids {
                if seen.insert(term_id) {
                    term_ids.push(term_id);
                }
            }
        }
        Ok(term_ids)
    }

    /// The UI lane: per query token, the titles holding a projected token
    /// within the distance the token's length allows.
    pub async fn ui_candidates(
        &self,
        tokens: &[String],
        facets: &[&str],
        distance_for: impl Fn(usize) -> u8,
        limit_per_token: usize,
    ) -> AppResult<Vec<UiFuzzyHit>> {
        let guard = self.write_guard().await;
        self.sync_for_read(&guard).await?;
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        // All tokens must search the same complete generation.
        let searcher = self.reader.searcher();
        drop(guard);
        let fields = self.fields;
        let mut hits = Vec::new();
        for token in tokens {
            let distance = distance_for(token.chars().count());
            let mut clauses: Vec<(Occur, Box<dyn Query>)> =
                vec![(Occur::Must, term_clause(fields.token_row, "1"))];
            if facets.len() < 3 {
                clauses.push((
                    Occur::Must,
                    Box::new(BooleanQuery::new(
                        facets
                            .iter()
                            .map(|facet| (Occur::Should, term_clause(fields.facet, facet)))
                            .collect(),
                    )),
                ));
            }
            clauses.push((
                Occur::Must,
                Box::new(FuzzyTermQuery::new(
                    Term::from_field_text(fields.lenient_raw, token),
                    distance,
                    true,
                )),
            ));
            let boolean = BooleanQuery::new(clauses);
            let searcher = searcher.clone();
            let token_key = token.clone();
            let found = tokio::task::spawn_blocking(move || -> tantivy::Result<Vec<UiFuzzyHit>> {
                let docs = searcher.search(
                    &boolean,
                    &TopDocs::with_limit(limit_per_token.max(1)).order_by_score(),
                )?;
                let mut found = Vec::with_capacity(docs.len());
                for (_score, address) in docs {
                    let doc = searcher.doc::<TantivyDocument>(address)?;
                    let (Some(title_id), Some(weight), Some(matched_term)) = (
                        doc.get_first(fields.title_id).and_then(|v| v.as_str()),
                        doc.get_first(fields.weight).and_then(|v| v.as_i64()),
                        doc.get_first(fields.lenient_raw).and_then(|v| v.as_str()),
                    ) else {
                        continue;
                    };
                    found.push(UiFuzzyHit {
                        title_id: title_id.to_string(),
                        token_key: token_key.clone(),
                        matched_term: matched_term.to_string(),
                        weight,
                    });
                }
                Ok(found)
            })
            .await;
            hits.extend(
                found
                    .map_err(|error| AppError::Repository(error.to_string()))?
                    .map_err(|error| AppError::Repository(error.to_string()))?,
            );
        }
        Ok(hits)
    }

    pub fn dialect(&self) -> TermSourceDialect {
        self.source.dialect()
    }

    /// The direct lanes of the UI search, one rank per title whose full name
    /// contains `lenient_query` in its lenient form.
    ///
    /// The index names the rows that can match and the projection decides
    /// which do, by primary key, a bounded batch of ascending ids at a time.
    /// The answer is therefore the projection's own, whatever the index holds:
    /// a row the index names that no longer matches is dropped there.
    pub async fn ui_contains_ranks(
        &self,
        lenient_query: &str,
        facets: &[&str],
        lanes: ContainsLaneRanks,
    ) -> AppResult<Vec<(String, i64)>> {
        let term_ids = self.ui_contains_candidates(lenient_query, facets).await?;
        let mut best: HashMap<String, i64> = HashMap::new();
        for batch in term_ids.chunks(CONTAINS_VERIFY_BATCH) {
            for (title_id, rank) in self
                .source
                .contains_ranks(batch, lenient_query, facets, lanes)
                .await?
            {
                best.entry(title_id)
                    .and_modify(|current| *current = (*current).min(rank))
                    .or_insert(rank);
            }
        }
        let mut ranks = best.into_iter().collect::<Vec<_>>();
        ranks.sort();
        Ok(ranks)
    }

    /// Term ids, ascending, of the full names whose lenient form may contain
    /// `lenient_query`.
    ///
    /// A name that contains the query holds every gram the query does, so no
    /// containing name is left out; a name can hold them all without holding
    /// them in that order, so the caller still tests each returned row, by
    /// primary key and in bounded batches.
    ///
    /// What is queued when the search arrives is drained first, whatever the
    /// drain interval: this lane replaces a read of the projection itself,
    /// which saw a title as soon as its write committed. A human search finds
    /// the queue empty, and writes nothing, except while a scan or an import
    /// is writing titles; then it drains a bounded part and answers.
    async fn ui_contains_candidates(
        &self,
        lenient_query: &str,
        facets: &[&str],
    ) -> AppResult<Vec<i64>> {
        let grams = lenient_query_grams(lenient_query);
        if grams.is_empty() {
            return Ok(Vec::new());
        }
        let guard = self.write_guard().await;
        self.sync_locked(&guard, Freshness::Arrived, Instant::now())
            .await?;
        let searcher = self.reader.searcher();
        drop(guard);
        let fields = self.fields;
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = grams
            .iter()
            .map(|gram| (Occur::Must, term_clause(fields.grams, gram)))
            .collect();
        if facets.len() < 3 {
            clauses.push((
                Occur::Must,
                Box::new(BooleanQuery::new(
                    facets
                        .iter()
                        .map(|facet| (Occur::Should, term_clause(fields.facet, facet)))
                        .collect(),
                )),
            ));
        }
        let boolean = BooleanQuery::new(clauses);
        tokio::task::spawn_blocking(move || -> tantivy::Result<Vec<i64>> {
            let mut by_segment: HashMap<u32, Vec<u32>> = HashMap::new();
            for address in searcher.search(&boolean, &DocSetCollector)? {
                by_segment
                    .entry(address.segment_ord)
                    .or_default()
                    .push(address.doc_id);
            }
            let mut term_ids = Vec::new();
            for (segment_ord, doc_ids) in by_segment {
                let column = searcher
                    .segment_reader(segment_ord)
                    .fast_fields()
                    .i64("term_id")?;
                term_ids.extend(doc_ids.into_iter().filter_map(|doc| column.first(doc)));
            }
            term_ids.sort_unstable();
            Ok(term_ids)
        })
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
        .map_err(|error| AppError::Repository(error.to_string()))
    }
}

/// The durable record, read once at open. Everything after that reads the
/// cached copy: an unreadable, malformed or older-schema file is the same
/// answer as a missing one — no stamp, so rebuild.
fn read_meta_file(path: &Path) -> Option<ProjectionStamp> {
    #[derive(serde::Deserialize)]
    struct Meta {
        schema_version: u32,
        stamp: ProjectionStamp,
    }
    let bytes = std::fs::read(path).ok()?;
    let meta = serde_json::from_slice::<Meta>(&bytes).ok()?;
    (meta.schema_version == FUZZY_SCHEMA_VERSION).then_some(meta.stamp)
}

fn reject_symlink(path: &Path) -> AppResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::Repository(format!(
            "title index path must not be a symlink: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(error.to_string())),
    }
}

fn open_index_files(dir: &Path) -> tantivy::Result<(Index, IndexReader)> {
    std::fs::create_dir_all(dir)?;
    if dir.join(META_FILE).exists() && !dir.join("meta.json").exists() {
        return Err(tantivy::directory::error::OpenReadError::FileDoesNotExist(
            dir.join("meta.json"),
        )
        .into());
    }
    let directory = MmapDirectory::open(dir)?;
    let index = Index::open_or_create(directory, build_schema().0)?;
    register_tokenizers(&index)
        .map_err(|error| tantivy::TantivyError::InvalidArgument(error.to_string()))?;
    if !index.validate_checksum()?.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "title index checksum mismatch",
        )
        .into());
    }
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()?;
    Ok((index, reader))
}

fn recoverable_index_error(error: &tantivy::TantivyError) -> bool {
    use tantivy::TantivyError;
    use tantivy::directory::error::OpenReadError;
    match error {
        TantivyError::DataCorruption(_)
        | TantivyError::SchemaError(_)
        | TantivyError::IncompatibleIndex(_)
        | TantivyError::DeserializeError(_)
        | TantivyError::OpenReadError(OpenReadError::FileDoesNotExist(_))
        | TantivyError::OpenReadError(OpenReadError::IncompatibleIndex(_)) => true,
        TantivyError::IoError(error)
        | TantivyError::OpenReadError(OpenReadError::IoError {
            io_error: error, ..
        }) => error.kind() == std::io::ErrorKind::InvalidData,
        _ => false,
    }
}

fn open_owned_index(dir: &Path) -> AppResult<(std::fs::File, Index, IndexReader)> {
    let parent = dir
        .parent()
        .ok_or_else(|| AppError::Repository("title index has no parent".into()))?;
    std::fs::create_dir_all(parent).map_err(|error| AppError::Repository(error.to_string()))?;
    let lock_path = parent.join("title-fuzzy-index.lock");
    reject_symlink(&lock_path)?;
    let ownership = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    ownership.try_lock().map_err(|error| {
        AppError::Repository(format!(
            "title index is already owned or cannot be locked: {error}"
        ))
    })?;
    reject_symlink(dir)?;
    if dir.exists() && !dir.is_dir() {
        return Err(AppError::Repository(
            "title index path is not a directory".into(),
        ));
    }
    let (index, reader) = match open_index_files(dir) {
        Ok(opened) => opened,
        Err(error) if recoverable_index_error(&error) => {
            // Reserve a new sibling without replacing anything already there.
            // Keep the entire old directory, including files Tantivy does not own.
            let mut suffix = 0u64;
            let preserved = loop {
                let candidate = parent.join(format!("{FUZZY_INDEX_DIR}.recovery-{suffix}"));
                match std::fs::create_dir(&candidate) {
                    Ok(()) => break candidate,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => suffix += 1,
                    Err(error) => return Err(AppError::Repository(error.to_string())),
                }
            };
            std::fs::rename(dir, preserved.join(FUZZY_INDEX_DIR))
                .map_err(|error| AppError::Repository(error.to_string()))?;
            tracing::warn!(%error, path = %preserved.display(), "preserved unreadable title index; rebuilding before use");
            open_index_files(dir).map_err(|error| AppError::Repository(error.to_string()))?
        }
        Err(error) => return Err(AppError::Repository(error.to_string())),
    };
    Ok((ownership, index, reader))
}

fn term_clause(field: Field, value: &str) -> Box<dyn Query> {
    Box::new(TermQuery::new(
        Term::from_field_text(field, value),
        IndexRecordOption::Basic,
    ))
}

/// One spelling, up to two independent ways of being close to it.
///
/// Each returned query is a *lane*, searched on its own by
/// [`TitleFuzzyIndex::resolver_candidates`] and unioned with the others. They
/// are not `Should` clauses of one query on purpose: their scores are not
/// comparable, so combining them lets one lane's ranking discard the other's
/// hits.
///
/// * The **automaton lane** is exact but stops at [`MAX_AUTOMATON_DISTANCE`].
///   It counts *characters*, not bytes — `levenshtein_automata` builds its DFA
///   over `char`s — so it is meaningful in every script and runs for every
///   script. A three-character Han name one character away from another is a
///   distance of one here, which is the only lane that can see it: two such
///   names share no bigram at all.
/// * The **gram lane** supplements it with character n-grams and a shared-gram
///   floor, only for a distance above the automaton's ceiling (bigrams in a
///   wide script, trigrams otherwise). `k` edits destroy at most `n * k`
///   n-grams, so a candidate within `k` edits shares at least `G - n * k` of
///   the query's `G` grams. That is a floor, not a heuristic: nothing within
///   the distance can fall below it. At or below the ceiling the automaton is
///   already complete in every script, so the gram lane would add nothing but
///   rows to discard — and on a short wide-script name its floor collapses to
///   one shared bigram, which is the one realistic way to fill the lane limit.
///
/// Either way these lanes only have to *find* candidates. Every one of them is
/// then measured exactly by the caller's spelling comparison, so a loose gram
/// hit costs a comparison and can never become a match on its own.
fn spelling_lanes(fields: &Fields, match_term: &str, distance: u8) -> Vec<Box<dyn Query>> {
    let mut lanes: Vec<Box<dyn Query>> = Vec::new();

    let automaton_distance = distance.min(MAX_AUTOMATON_DISTANCE);
    lanes.push(Box::new(BooleanQuery::new(
        [fields.match_raw, fields.literal_raw]
            .into_iter()
            .map(|field| {
                let query: Box<dyn Query> = Box::new(FuzzyTermQuery::new(
                    Term::from_field_text(field, match_term),
                    automaton_distance,
                    true,
                ));
                (Occur::Should, query)
            })
            .collect(),
    )));

    if distance > MAX_AUTOMATON_DISTANCE {
        let gram_size = if is_wide_script(match_term) { 2 } else { 3 };
        let grams = character_ngrams(match_term, gram_size);
        if !grams.is_empty() {
            let destroyed = gram_size * distance as usize;
            let required = grams.len().saturating_sub(destroyed).max(1);
            lanes.push(Box::new(BooleanQuery::with_minimum_required_clauses(
                grams
                    .into_iter()
                    .map(|gram| (Occur::Should, term_clause(fields.grams, gram.as_str())))
                    .collect(),
                required,
            )));
        }
    }

    lanes
}

fn character_ngrams(value: &str, size: usize) -> Vec<String> {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() < size {
        return Vec::new();
    }
    chars
        .windows(size)
        .map(|window| window.iter().collect::<String>())
        .collect()
}

/// Characters outside the Basic Multilingual Plane's single-byte and
/// two-byte ranges: Han, Kana, Hangul and the CJK punctuation around them.
fn is_wide_script(value: &str) -> bool {
    value.chars().any(|character| {
        matches!(character as u32,
            0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
    })
}

fn document_for(fields: &Fields, term: &IndexedTerm) -> TantivyDocument {
    let mut doc = TantivyDocument::default();
    doc.add_i64(fields.term_id, term.term_id);
    doc.add_text(fields.title_id, &term.title_id);
    doc.add_text(fields.facet, &term.facet);
    doc.add_text(fields.script, &term.script);
    doc.add_text(fields.numbers_key, &term.numbers_key);
    doc.add_text(fields.term_kind, &term.term_kind);
    let is_token_row = term.term_kind.ends_with("_token");
    doc.add_text(fields.token_row, if is_token_row { "1" } else { "0" });
    doc.add_i64(fields.weight, term.weight);
    doc.add_i64(fields.char_length, term.char_length);
    doc.add_text(fields.match_raw, &term.match_term);
    doc.add_text(fields.literal_raw, &term.literal_term);
    doc.add_text(fields.lenient_raw, &term.normalized_term);
    doc.add_text(fields.grams, &term.match_term);
    // The contains lane matches full names only, as the SQL it narrows does.
    if !is_token_row {
        let tokens = (1..=LENIENT_GRAM_MAX_CHARS)
            .flat_map(|size| lenient_grams(&term.normalized_term, size))
            .enumerate()
            .map(|(position, text)| Token {
                offset_from: 0,
                offset_to: 0,
                position,
                text,
                position_length: 1,
            })
            .collect::<Vec<_>>();
        if !tokens.is_empty() {
            doc.add_pre_tokenized_text(
                fields.grams,
                PreTokenizedString {
                    text: String::new(),
                    tokens,
                },
            );
        }
    }
    doc
}

/// The distinct `size`-character grams of a lenient form, each marked.
fn lenient_grams(lenient: &str, size: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    character_ngrams(lenient, size)
        .into_iter()
        .filter(|gram| seen.insert(gram.clone()))
        .map(|gram| format!("{LENIENT_GRAM_MARK}{gram}"))
        .collect()
}

/// The grams every lenient form containing `query` must hold.
fn lenient_query_grams(query: &str) -> Vec<String> {
    let size = query.chars().count().min(LENIENT_GRAM_MAX_CHARS);
    if size == 0 {
        return Vec::new();
    }
    lenient_grams(query, size)
}

/// Terms grouped by title, for callers that rebuild a title's documents.
pub fn group_terms_by_title(terms: Vec<IndexedTerm>) -> HashMap<String, Vec<IndexedTerm>> {
    let mut grouped: HashMap<String, Vec<IndexedTerm>> = HashMap::new();
    for term in terms {
        grouped.entry(term.title_id.clone()).or_default().push(term);
    }
    grouped
}

#[cfg(test)]
mod remove_meta_tests {
    use super::*;

    struct EmptySource;

    #[async_trait]
    impl TitleTermSource for EmptySource {
        async fn page_terms(&self, _after: i64, _limit: i64) -> AppResult<Vec<IndexedTerm>> {
            Ok(Vec::new())
        }
        async fn terms_for_titles(&self, _title_ids: &[String]) -> AppResult<Vec<IndexedTerm>> {
            Ok(Vec::new())
        }
        async fn queued_titles(&self, _limit: i64) -> AppResult<Vec<QueuedTitle>> {
            Ok(Vec::new())
        }
        async fn clear_queued(&self, _seqs: &[i64]) -> AppResult<()> {
            Ok(())
        }
        async fn projection_stamp(&self) -> AppResult<ProjectionStamp> {
            Ok(stamp())
        }

        fn dialect(&self) -> TermSourceDialect {
            TermSourceDialect::Sqlite
        }
        async fn contains_ranks(
            &self,
            _term_ids: &[i64],
            _lenient_query: &str,
            _facets: &[&str],
            _lanes: ContainsLaneRanks,
        ) -> AppResult<Vec<(String, i64)>> {
            Ok(Vec::new())
        }
    }

    fn stamp() -> ProjectionStamp {
        ProjectionStamp {
            collation_version: "fixture-collation".into(),
            projection_generation: 7,
        }
    }

    async fn open_fixture(data_dir: &Path) -> Arc<TitleFuzzyIndex> {
        let index = TitleFuzzyIndex::open(data_dir, Arc::new(EmptySource))
            .await
            .expect("fixture index opens");
        assert!(index.meta_path().exists(), "open writes the stamp file");
        assert!(index.stamp_matches(&stamp()));
        index
    }

    #[tokio::test]
    async fn removes_only_the_stamp_file_and_tolerates_it_already_gone() {
        let data = tempfile::tempdir().unwrap();
        let index = open_fixture(data.path()).await;
        let index_sibling = index.dir.join("fixture-sibling.txt");
        let parent_sibling = data.path().join("fixture-parent-sibling.txt");
        std::fs::write(&index_sibling, b"keep").unwrap();
        std::fs::write(&parent_sibling, b"keep").unwrap();

        index.remove_meta().expect("stamp file removed");
        assert!(!index.meta_path().exists());
        assert!(!index.stamp_matches(&stamp()));
        assert!(index_sibling.exists(), "a file beside the stamp survives");
        assert!(parent_sibling.exists(), "a file in the data dir survives");
        assert!(
            index.dir.join("meta.json").exists(),
            "tantivy's own meta survives"
        );

        index
            .remove_meta()
            .expect("an already-missing stamp is not an error");
        assert!(index_sibling.exists() && parent_sibling.exists());
    }

    #[tokio::test]
    async fn a_failed_remove_still_clears_the_cached_stamp() {
        let data = tempfile::tempdir().unwrap();
        let index = open_fixture(data.path()).await;
        std::fs::remove_file(index.meta_path()).unwrap();
        // A directory in the stamp's place makes `remove_file` fail with
        // something other than NotFound.
        std::fs::create_dir(index.meta_path()).unwrap();
        assert!(index.stamp_matches(&stamp()), "cache still holds the stamp");

        assert!(index.remove_meta().is_err());
        assert!(!index.stamp_matches(&stamp()));
        assert!(
            index.meta_path().is_dir(),
            "the blocking directory is left alone"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_meta_refuses_a_symlinked_stamp_path() {
        let data = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("fixture-target.json");
        std::fs::write(&target, b"untouched").unwrap();
        let index = open_fixture(data.path()).await;
        std::fs::remove_file(index.meta_path()).unwrap();
        std::os::unix::fs::symlink(&target, index.meta_path()).unwrap();

        assert!(index.write_meta(&stamp()).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
    }
}

#[cfg(test)]
mod drain_cadence_tests {
    use super::*;
    use std::sync::Mutex;

    const INTERVAL: Duration = Duration::from_secs(10);

    /// A projection and queue held in memory, so a test controls exactly
    /// what is queued and when.
    #[derive(Default)]
    struct QueueSource {
        terms: Mutex<Vec<IndexedTerm>>,
        queue: Mutex<Vec<QueuedTitle>>,
        generation: Mutex<i64>,
        next_id: Mutex<i64>,
        /// Every batch of candidate ids the contains lane asked about.
        verified: Mutex<Vec<Vec<i64>>>,
    }

    impl QueueSource {
        /// What a projection write does: rows plus a queue claim.
        fn write_title(&self, title_id: &str, name: &str) {
            self.write_term(title_id, name, "name", "series");
        }

        fn write_term(&self, title_id: &str, name: &str, term_kind: &str, facet: &str) {
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            self.terms.lock().unwrap().push(IndexedTerm {
                term_id: *next,
                title_id: title_id.to_string(),
                facet: facet.into(),
                term_kind: term_kind.into(),
                weight: 0,
                literal_term: name.to_string(),
                match_term: name.to_string(),
                normalized_term: name.to_string(),
                script: "latin".into(),
                numbers_key: String::new(),
                char_length: name.chars().count() as i64,
            });
            self.queue.lock().unwrap().push(QueuedTitle {
                seq: *next,
                title_id: title_id.to_string(),
            });
        }

        fn queued(&self) -> usize {
            self.queue.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl TitleTermSource for QueueSource {
        async fn page_terms(&self, after: i64, limit: i64) -> AppResult<Vec<IndexedTerm>> {
            let mut page = self
                .terms
                .lock()
                .unwrap()
                .iter()
                .filter(|term| term.term_id > after)
                .cloned()
                .collect::<Vec<_>>();
            page.sort_by_key(|term| term.term_id);
            page.truncate(limit as usize);
            Ok(page)
        }
        async fn terms_for_titles(&self, title_ids: &[String]) -> AppResult<Vec<IndexedTerm>> {
            Ok(self
                .terms
                .lock()
                .unwrap()
                .iter()
                .filter(|term| title_ids.contains(&term.title_id))
                .cloned()
                .collect())
        }
        async fn queued_titles(&self, limit: i64) -> AppResult<Vec<QueuedTitle>> {
            Ok(self
                .queue
                .lock()
                .unwrap()
                .iter()
                .take(limit as usize)
                .cloned()
                .collect())
        }
        async fn clear_queued(&self, seqs: &[i64]) -> AppResult<()> {
            self.queue
                .lock()
                .unwrap()
                .retain(|entry| !seqs.contains(&entry.seq));
            Ok(())
        }
        async fn projection_stamp(&self) -> AppResult<ProjectionStamp> {
            Ok(ProjectionStamp {
                collation_version: "fixture-collation".into(),
                projection_generation: *self.generation.lock().unwrap(),
            })
        }

        fn dialect(&self) -> TermSourceDialect {
            TermSourceDialect::Sqlite
        }
        async fn contains_ranks(
            &self,
            term_ids: &[i64],
            lenient_query: &str,
            facets: &[&str],
            lanes: ContainsLaneRanks,
        ) -> AppResult<Vec<(String, i64)>> {
            assert!(term_ids.is_sorted(), "candidate ids arrive ascending");
            self.verified.lock().unwrap().push(term_ids.to_vec());
            let mut best: HashMap<String, i64> = HashMap::new();
            for term in self.terms.lock().unwrap().iter().filter(|term| {
                term_ids.contains(&term.term_id)
                    && !term.term_kind.ends_with("_token")
                    && facets.contains(&term.facet.as_str())
                    && term.normalized_term.contains(lenient_query)
            }) {
                let lane = if term.normalized_term == lenient_query {
                    lanes.exact
                } else if term.normalized_term.starts_with(lenient_query) {
                    lanes.prefix
                } else {
                    lanes.contains
                };
                let rank = lane + term.weight;
                best.entry(term.title_id.clone())
                    .and_modify(|current| *current = (*current).min(rank))
                    .or_insert(rank);
            }
            Ok(best.into_iter().collect())
        }
    }

    const LANES: ContainsLaneRanks = ContainsLaneRanks {
        exact: 0,
        prefix: 1_000,
        contains: 2_000,
    };
    const ALL_FACETS: [&str; 3] = ["movie", "series", "anime"];

    #[tokio::test]
    async fn the_contains_lane_ranks_names_by_how_they_hold_the_query() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        source.write_title("fixture-exact", "amber road");
        source.write_title("fixture-prefix", "amber road north");
        source.write_title("fixture-inside", "the amber road north");
        // Holds every gram of the query, in another order.
        source.write_title("fixture-shuffled", "road amber roa mber r");
        source.write_title("fixture-other", "copper lane");
        source.write_term("fixture-token", "amber road", "name_token", "series");
        source.write_term("fixture-movie", "amber road", "name", "movie");
        let (index, _opened) = open_fixture(data.path(), source.clone()).await;

        let ranks = index
            .ui_contains_ranks("amber road", &ALL_FACETS, LANES)
            .await
            .unwrap();
        assert_eq!(
            ranks,
            vec![
                ("fixture-exact".to_string(), 0),
                ("fixture-inside".to_string(), 2_000),
                ("fixture-movie".to_string(), 0),
                ("fixture-prefix".to_string(), 1_000),
            ]
        );
        let verified = source.verified.lock().unwrap().concat();
        assert!(
            !verified.contains(&5) && !verified.contains(&6),
            "an unrelated name and a token row are never candidates: {verified:?}"
        );

        let series_only = index
            .ui_contains_ranks("amber road", &["series"], LANES)
            .await
            .unwrap();
        assert!(
            series_only
                .iter()
                .all(|(title_id, _)| title_id != "fixture-movie"),
            "{series_only:?}"
        );
    }

    #[tokio::test]
    async fn the_contains_lane_answers_queries_shorter_than_a_trigram() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        source.write_title("fixture-short", "up");
        source.write_title("fixture-long", "pull up north");
        source.write_title("fixture-single", "x");
        source.write_title("fixture-none", "level");
        let (index, _opened) = open_fixture(data.path(), source).await;

        let titles = |ranks: Vec<(String, i64)>| {
            ranks
                .into_iter()
                .map(|(title_id, _)| title_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles(
                index
                    .ui_contains_ranks("up", &ALL_FACETS, LANES)
                    .await
                    .unwrap()
            ),
            ["fixture-long", "fixture-short"]
        );
        assert_eq!(
            titles(
                index
                    .ui_contains_ranks("u", &ALL_FACETS, LANES)
                    .await
                    .unwrap()
            ),
            ["fixture-long", "fixture-short"]
        );
        assert_eq!(
            titles(
                index
                    .ui_contains_ranks("x", &ALL_FACETS, LANES)
                    .await
                    .unwrap()
            ),
            ["fixture-single"]
        );
        assert_eq!(
            titles(
                index
                    .ui_contains_ranks("p n", &ALL_FACETS, LANES)
                    .await
                    .unwrap()
            ),
            ["fixture-long"]
        );
    }

    #[tokio::test]
    async fn the_contains_lane_sees_a_title_written_inside_the_drain_interval() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        let (index, _opened) = open_fixture(data.path(), source.clone()).await;

        source.write_title("fixture-late", "late harbor");
        let ranks = index
            .ui_contains_ranks("harbor", &ALL_FACETS, LANES)
            .await
            .unwrap();
        assert_eq!(ranks, vec![("fixture-late".to_string(), 2_000)]);
        assert_eq!(source.queued(), 0);
    }

    #[tokio::test]
    async fn the_contains_lane_drains_a_bounded_part_of_a_long_queue() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        let (index, _opened) = open_fixture(data.path(), source.clone()).await;

        let batch = QUEUE_DRAIN_BATCH as usize;
        let queued = batch * (SEARCH_DRAIN_MAX_BATCHES + 2);
        for number in 0..queued {
            source.write_title(&format!("fixture-{number}"), &format!("harbor {number}"));
        }
        let first = index
            .ui_contains_ranks("harbor", &ALL_FACETS, LANES)
            .await
            .unwrap();
        assert_eq!(first.len(), batch * SEARCH_DRAIN_MAX_BATCHES);
        assert_eq!(source.queued(), batch * 2);

        // The next search carries on from there, and an explicit sync ends it.
        let second = index
            .ui_contains_ranks("harbor", &ALL_FACETS, LANES)
            .await
            .unwrap();
        assert_eq!(second.len(), queued);
        assert_eq!(source.queued(), 0);
    }

    #[tokio::test]
    async fn the_contains_lane_tests_candidates_in_bounded_ascending_batches() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        let total = CONTAINS_VERIFY_BATCH * 2 + 7;
        for number in 0..total {
            source.write_term(
                &format!("fixture-{number}"),
                &format!("harbor {number}"),
                "name",
                "series",
            );
        }
        let (index, _opened) = open_fixture(data.path(), source.clone()).await;

        let ranks = index
            .ui_contains_ranks("harbor", &ALL_FACETS, LANES)
            .await
            .unwrap();
        assert_eq!(ranks.len(), total);
        let verified = source.verified.lock().unwrap();
        assert_eq!(
            verified.iter().map(Vec::len).collect::<Vec<_>>(),
            [CONTAINS_VERIFY_BATCH, CONTAINS_VERIFY_BATCH, 7]
        );
        assert!(verified.concat().is_sorted());
    }

    #[test]
    fn lenient_grams_never_meet_the_match_form_grams() {
        for gram in lenient_grams("amber road", 3)
            .into_iter()
            .chain(lenient_query_grams("am"))
            .chain(lenient_query_grams("a"))
        {
            assert!(gram.starts_with(LENIENT_GRAM_MARK), "{gram:?}");
        }
        assert!(lenient_query_grams("").is_empty());
    }

    async fn open_fixture(
        data_dir: &Path,
        source: Arc<QueueSource>,
    ) -> (Arc<TitleFuzzyIndex>, Instant) {
        let index = TitleFuzzyIndex::open_with_drain_interval(data_dir, source, INTERVAL)
            .await
            .expect("fixture index opens");
        // Open drains unconditionally; every instant below is measured from
        // that drain rather than from the wall clock.
        let opened = index
            .last_drain
            .lock()
            .unwrap()
            .expect("open records its drain");
        (index, opened)
    }

    /// A read as the read path does it, at a chosen instant.
    async fn read_at(index: &TitleFuzzyIndex, now: Instant) {
        let guard = index.write_guard().await;
        index
            .sync_locked(&guard, Freshness::Bounded, now)
            .await
            .expect("read-path drain succeeds");
    }

    fn committed_opstamp(index: &TitleFuzzyIndex) -> u64 {
        index.index.load_metas().expect("metas load").opstamp
    }

    fn visible_docs(index: &TitleFuzzyIndex) -> u64 {
        index.reader.searcher().num_docs()
    }

    #[tokio::test]
    async fn reads_inside_the_interval_share_one_commit() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        let (index, opened) = open_fixture(data.path(), source.clone()).await;
        let before = committed_opstamp(&index);

        source.write_title("fixture-title-a", "fixture alpha");
        for offset in [1, 3, 9] {
            read_at(&index, opened + Duration::from_secs(offset)).await;
        }
        assert_eq!(
            committed_opstamp(&index),
            before,
            "no read inside the interval commits"
        );
        assert_eq!(source.queued(), 1, "the queued title waits for its drain");
        assert_eq!(visible_docs(&index), 0);

        // The first read past the interval drains, and the reads after it
        // inside the next interval do not commit again.
        read_at(&index, opened + INTERVAL).await;
        let drained = committed_opstamp(&index);
        assert!(drained > before, "the read past the interval commits");
        assert_eq!(source.queued(), 0);
        assert_eq!(visible_docs(&index), 1);

        source.write_title("fixture-title-b", "fixture bravo");
        for offset in [11, 14, 19] {
            read_at(&index, opened + Duration::from_secs(offset)).await;
        }
        assert_eq!(committed_opstamp(&index), drained, "still one commit");
        assert_eq!(source.queued(), 1);
    }

    #[tokio::test]
    async fn a_waiting_queue_lands_once_the_interval_elapses() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        let (index, opened) = open_fixture(data.path(), source.clone()).await;

        read_at(&index, opened + INTERVAL).await;
        source.write_title("fixture-title-a", "fixture alpha");
        read_at(&index, opened + INTERVAL + Duration::from_secs(1)).await;
        assert_eq!(source.queued(), 1, "inside the interval of the last drain");

        read_at(&index, opened + INTERVAL * 2).await;
        assert_eq!(source.queued(), 0, "drained one interval after the last");
        assert_eq!(visible_docs(&index), 1);
    }

    #[tokio::test]
    async fn an_explicit_sync_drains_inside_the_interval() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        let (index, _opened) = open_fixture(data.path(), source.clone()).await;

        source.write_title("fixture-title-a", "fixture alpha");
        index.sync().await.expect("sync succeeds");
        assert_eq!(source.queued(), 0);
        assert_eq!(visible_docs(&index), 1);
    }

    #[tokio::test]
    async fn a_changed_stamp_rebuilds_even_inside_the_interval() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        let (index, opened) = open_fixture(data.path(), source.clone()).await;

        source.write_title("fixture-title-a", "fixture alpha");
        *source.generation.lock().unwrap() += 1;
        read_at(&index, opened + Duration::from_secs(1)).await;

        let stamp = source.projection_stamp().await.unwrap();
        assert!(index.stamp_matches(&stamp), "the read rebuilt to the stamp");
        assert_eq!(visible_docs(&index), 1, "the rebuild paged the title in");
    }

    #[tokio::test]
    async fn a_zero_interval_drains_on_every_read() {
        let data = tempfile::tempdir().unwrap();
        let source = Arc::new(QueueSource::default());
        let index =
            TitleFuzzyIndex::open_with_drain_interval(data.path(), source.clone(), Duration::ZERO)
                .await
                .expect("fixture index opens");
        let opened = index.last_drain.lock().unwrap().unwrap();

        source.write_title("fixture-title-a", "fixture alpha");
        read_at(&index, opened).await;
        assert_eq!(source.queued(), 0);
    }
}
