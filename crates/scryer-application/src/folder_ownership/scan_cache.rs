//! Ownership state one scan reuses between its folder claims: the recorded
//! folder owners, held while the catalog's folder-write counter shows that
//! nothing but this scan's own claims changed them, and filesystem evidence
//! for the link checks. Without a counter, or after any other folder write,
//! the owners are read anew for the claim, as outside a scan.
//!
//! Held owners are indexed by comparison key and by the directories their
//! link checks walk, so a claim visits only the owners the linear visit would
//! compare or probe, in the same order, with the same probes.
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ops::Bound::{Excluded, Included, Unbounded},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use super::*;

tokio::task_local! {
    static OWNERSHIP_CONTEXT: Arc<ScanOwnershipContext>;
}

pub(crate) async fn with_scan_ownership<F: std::future::Future>(
    context: Option<Arc<ScanOwnershipContext>>,
    future: F,
) -> F::Output {
    match context {
        Some(context) => OWNERSHIP_CONTEXT.scope(context, future).await,
        None => future.await,
    }
}

/// An owner list estimated above this many bytes is not held between claims;
/// each claim reads the owners anew instead.
const HELD_OWNERS_BYTE_BUDGET: usize = 64 * 1024 * 1024;

pub(crate) struct ScanOwnershipContext {
    admission: Arc<tokio::sync::Semaphore>,
    cache: Mutex<PathEvidence>,
    /// Read and changed only while holding the admission permit.
    owners: Mutex<HeldOwners>,
    /// How reservations settled, queued by their guards without the permit
    /// and applied by the next claim that holds it.
    settlements: Mutex<Vec<Settlement>>,
    next_reservation: AtomicU64,
    #[cfg(test)]
    oracle: ClaimOracle,
}

/// Test controls: answer held claims by the linear visit, or check every
/// indexed claim against the linear visit run on a copy of the evidence.
#[cfg(test)]
#[derive(Default)]
struct ClaimOracle {
    linear: std::sync::atomic::AtomicBool,
    compare: std::sync::atomic::AtomicBool,
    compared: AtomicU64,
    rests: AtomicU64,
    mismatches: Mutex<Vec<String>>,
    /// Replaces [`HELD_OWNERS_BYTE_BUDGET`] when set.
    byte_budget: std::sync::atomic::AtomicUsize,
}

impl ScanOwnershipContext {
    fn held_owners_byte_budget(&self) -> usize {
        #[cfg(test)]
        {
            let budget = self.oracle.byte_budget.load(Ordering::SeqCst);
            if budget != 0 {
                return budget;
            }
        }
        HELD_OWNERS_BYTE_BUDGET
    }
}

/// One recorded folder owner and its comparison keys for the roots in
/// [`HeldList::prepared_for`].
struct HeldOwner {
    title_id: String,
    folder: String,
    prefilter: Option<OwnerOverlapPrefilter>,
}

impl HeldOwner {
    fn new(title_id: String, folder: String) -> Self {
        Self {
            title_id,
            folder,
            prefilter: None,
        }
    }

    fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.title_id.capacity()
            + self.folder.capacity()
            + self
                .prefilter
                .as_ref()
                .map_or(0, OwnerOverlapPrefilter::estimated_bytes)
    }
}

/// An estimate made before any key is prepared: the keys and the path add
/// roughly three more copies of the folder, and the index its keys, its
/// chain directories and copies of the id.
fn estimated_unprepared_bytes(owners: &[(String, String)]) -> usize {
    owners
        .iter()
        .map(|(id, folder)| {
            std::mem::size_of::<HeldOwner>()
                + std::mem::size_of::<OwnerOverlapPrefilter>()
                + 9 * std::mem::size_of::<String>()
                + 6 * id.len()
                + 7 * folder.len()
        })
        .sum()
}

struct HeldList {
    /// The counter value the owners are known to be current at.
    generation: u64,
    /// Counter values produced by this scan's own claims after `generation`,
    /// already applied to `owners`.
    own: BTreeSet<u64>,
    /// By title id, the order of the sorted catalog read.
    owners: BTreeMap<String, HeldOwner>,
    /// The roots every owner's prefilter was prepared for, and the index of
    /// the owners under them.
    prepared: Option<(Vec<(String, Option<String>)>, OwnerIndex)>,
}

impl HeldList {
    /// Whether the owners are still what a fresh read would return, given the
    /// counter's `current` value: every advance since they were known to be
    /// current came from one of this scan's own claims.
    fn holds(&mut self, current: Option<u64>) -> bool {
        let Some(current) = current else {
            return false;
        };
        if current < self.generation {
            return false;
        }
        let own = self.own.range(self.generation + 1..=current).count() as u64;
        if own != current - self.generation {
            return false;
        }
        self.generation = current;
        self.own.clear();
        true
    }

    /// Apply this scan's own recorded claim, the catalog recording `folder`
    /// for `title_id` at counter value `generation`.
    fn apply_own_write(&mut self, title_id: &str, folder: Option<String>, generation: u64) {
        if generation <= self.generation {
            // The owners were read after this write: they already show it, or
            // a later write that replaced it.
            return;
        }
        self.own.insert(generation);
        if let Some(previous) = self.owners.remove(title_id)
            && let Some((_, index)) = self.prepared.as_mut()
        {
            index.remove(&previous);
        }
        if let Some(folder) = folder.filter(|folder| !folder.is_empty()) {
            let mut owner = HeldOwner::new(title_id.to_owned(), folder);
            if let Some((roots, index)) = self.prepared.as_mut() {
                owner.prefilter = Some(OwnerOverlapPrefilter::prepare(&owner.folder, roots));
                index.insert(&owner);
            }
            self.owners.insert(title_id.to_owned(), owner);
        }
    }

    /// Prepare every owner and index them for `roots`, unless that is done.
    fn prepare(&mut self, roots: &[(String, Option<String>)], evidence: &PathEvidence) {
        if self
            .prepared
            .as_ref()
            .is_some_and(|(prepared, _)| prepared == roots)
        {
            return;
        }
        for owner in self.owners.values_mut() {
            owner.prefilter = Some(OwnerOverlapPrefilter::prepare(&owner.folder, roots));
        }
        let index = OwnerIndex::build(self.owners.values(), evidence);
        self.prepared = Some((roots.to_vec(), index));
    }

    fn estimated_bytes(&self) -> usize {
        self.owners
            .values()
            .map(|owner| {
                owner.estimated_bytes() + std::mem::size_of::<String>() + owner.title_id.len()
            })
            .sum::<usize>()
            + self
                .prepared
                .as_ref()
                .map_or(0, |(_, index)| index.estimated_bytes())
    }
}

/// The held owners by comparison key and by the directories their link
/// checks walk. Every held owner is in it, prepared for the same roots.
#[derive(Default)]
struct OwnerIndex {
    /// Each owner key, its own and its key under a resolved root, to the
    /// owners holding it.
    keys: BTreeMap<String, BTreeSet<String>>,
    /// Owners every claim compares or walks: their prefilter is
    /// [`OwnerOverlapPrefilter::Exact`], or a directory they walk has no
    /// containing directory.
    always: BTreeSet<String>,
    slots: HashMap<PathBuf, usize>,
    groups: Vec<Option<ChainGroup>>,
    free: Vec<usize>,
}

/// The directories owners walk for links that share one containing
/// directory. The evidence checks each of them against that directory's
/// stamp, so one probe of `parent` answers for all of them while it holds.
struct ChainGroup {
    parent: PathBuf,
    /// Components of `parent`: of one owner's groups, the deeper is walked
    /// first.
    depth: usize,
    /// Each walked directory to the owners that walk it.
    children: HashMap<PathBuf, BTreeSet<String>>,
    /// Every owner walking one of `children`; each walks at most one.
    owners: BTreeSet<String>,
    record: Option<GroupRecord>,
}

/// What the evidence held for a group: every child outside `unverified` is
/// recorded as a directory under `stamp`, and nothing was removed from the
/// evidence since its `invalidations` read `epoch`.
struct GroupRecord {
    stamp: Stamp,
    epoch: u64,
    unverified: HashSet<PathBuf>,
}

/// The directories an owner's link check walks, deepest first.
fn chain_dirs(prefilter: &OwnerOverlapPrefilter) -> impl Iterator<Item = &Path> {
    match prefilter {
        OwnerOverlapPrefilter::Keys { path, chain, .. } => Some(path.ancestors().take(*chain)),
        OwnerOverlapPrefilter::Exact => None,
    }
    .into_iter()
    .flatten()
}

impl ChainGroup {
    /// The record the evidence supports now, with every child either under
    /// `stamp` (the first one found when `None`) or absent.
    fn record_from(&self, evidence: &PathEvidence, stamp: Option<Stamp>) -> Option<GroupRecord> {
        let mut stamp = stamp;
        let mut unverified = HashSet::new();
        for child in self.children.keys() {
            match (evidence.directories.get(child), stamp) {
                (None, _) => {
                    unverified.insert(child.clone());
                }
                (Some(recorded), None) => stamp = Some(*recorded),
                (Some(recorded), Some(stamp)) if *recorded == stamp => {}
                // A later check of that child would invalidate its siblings.
                (Some(_), Some(_)) => return None,
            }
        }
        Some(GroupRecord {
            stamp: stamp?,
            epoch: evidence.invalidations,
            unverified,
        })
    }

    /// Bring the record up to the evidence after a claim that found the
    /// containing directory at `stamp`.
    fn refresh(&mut self, evidence: &PathEvidence, stamp: Stamp) {
        if let Some(record) = self.record.as_mut()
            && record.stamp == stamp
            && record.epoch == evidence.invalidations
        {
            let mut stale = false;
            record
                .unverified
                .retain(|child| match evidence.directories.get(child) {
                    Some(recorded) if *recorded == stamp => false,
                    Some(_) => {
                        stale = true;
                        true
                    }
                    None => true,
                });
            if stale {
                self.record = None;
            }
            return;
        }
        self.record = self.record_from(evidence, Some(stamp));
    }

    fn estimated_bytes(&self) -> usize {
        let ids = |ids: &BTreeSet<String>| {
            ids.iter()
                .map(|id| id.capacity() + std::mem::size_of::<String>())
                .sum::<usize>()
        };
        std::mem::size_of::<Self>()
            + self.parent.as_os_str().len()
            + self
                .children
                .iter()
                .map(|(child, owners)| {
                    std::mem::size_of::<PathBuf>() + child.as_os_str().len() + ids(owners)
                })
                .sum::<usize>()
            + ids(&self.owners)
            + self.record.as_ref().map_or(0, |record| {
                record
                    .unverified
                    .iter()
                    .map(|child| std::mem::size_of::<PathBuf>() + child.as_os_str().len())
                    .sum()
            })
    }
}

impl OwnerIndex {
    fn build<'a>(owners: impl Iterator<Item = &'a HeldOwner>, evidence: &PathEvidence) -> Self {
        let mut index = Self::default();
        for owner in owners {
            index.insert(owner);
        }
        for group in index.groups.iter_mut().flatten() {
            group.record = group.record_from(evidence, None);
        }
        index
    }

    fn insert(&mut self, owner: &HeldOwner) {
        let id = &owner.title_id;
        let prefilter = owner
            .prefilter
            .as_ref()
            .expect("owners are prepared before they are indexed");
        let OwnerOverlapPrefilter::Keys { keys, .. } = prefilter else {
            self.always.insert(id.clone());
            return;
        };
        for key in keys {
            self.keys.entry(key.clone()).or_default().insert(id.clone());
        }
        for dir in chain_dirs(prefilter) {
            let Some(parent) = dir.parent() else {
                self.always.insert(id.clone());
                continue;
            };
            let slot = match self.slots.get(parent) {
                Some(slot) => *slot,
                None => {
                    let group = ChainGroup {
                        parent: parent.to_path_buf(),
                        depth: parent.components().count(),
                        children: HashMap::new(),
                        owners: BTreeSet::new(),
                        record: None,
                    };
                    let slot = match self.free.pop() {
                        Some(slot) => {
                            self.groups[slot] = Some(group);
                            slot
                        }
                        None => {
                            self.groups.push(Some(group));
                            self.groups.len() - 1
                        }
                    };
                    self.slots.insert(parent.to_path_buf(), slot);
                    slot
                }
            };
            let group = self.groups[slot]
                .as_mut()
                .expect("an indexed slot holds its group");
            group.owners.insert(id.clone());
            let walkers = group.children.entry(dir.to_path_buf()).or_default();
            if walkers.is_empty()
                && let Some(record) = group.record.as_mut()
            {
                record.unverified.insert(dir.to_path_buf());
            }
            walkers.insert(id.clone());
        }
    }

    fn remove(&mut self, owner: &HeldOwner) {
        let id = owner.title_id.as_str();
        self.always.remove(id);
        let Some(prefilter) = owner.prefilter.as_ref() else {
            return;
        };
        if let OwnerOverlapPrefilter::Keys { keys, .. } = prefilter {
            for key in keys {
                if let Some(holders) = self.keys.get_mut(key) {
                    holders.remove(id);
                    if holders.is_empty() {
                        self.keys.remove(key);
                    }
                }
            }
        }
        for dir in chain_dirs(prefilter) {
            let Some(parent) = dir.parent() else {
                continue;
            };
            let Some(&slot) = self.slots.get(parent) else {
                continue;
            };
            let group = self.groups[slot]
                .as_mut()
                .expect("an indexed slot holds its group");
            group.owners.remove(id);
            if let Some(walkers) = group.children.get_mut(dir) {
                walkers.remove(id);
                if walkers.is_empty() {
                    group.children.remove(dir);
                    if let Some(record) = group.record.as_mut() {
                        record.unverified.remove(dir);
                    }
                }
            }
            if group.owners.is_empty() {
                self.slots.remove(parent);
                self.groups[slot] = None;
                self.free.push(slot);
            }
        }
    }

    /// Every owner with a key [`overlap_keys_related`] to `candidate`: the
    /// keys `candidate` lies within are its prefixes that end at or just
    /// after a separator, or all of it; the keys within `candidate` start with
    /// it followed by a separator, or with it when it ends in one.
    fn related(&self, candidate: &str, mut found: impl FnMut(&str)) {
        let mut ends = vec![candidate.len()];
        for (at, _) in candidate.match_indices('/') {
            ends.push(at);
            ends.push(at + 1);
        }
        for end in ends {
            let key = &candidate[..end];
            if let Some(holders) = self.keys.get(key)
                && overlap_keys_related(key, candidate)
            {
                holders.iter().for_each(|id| found(id));
            }
        }
        let prefix = if candidate.ends_with('/') {
            std::borrow::Cow::Borrowed(candidate)
        } else {
            std::borrow::Cow::Owned(format!("{candidate}/"))
        };
        for (key, holders) in self
            .keys
            .range::<str, _>((Included(prefix.as_ref()), Unbounded))
            .take_while(|(key, _)| key.starts_with(&*prefix))
        {
            if overlap_keys_related(key, candidate) {
                holders.iter().for_each(|id| found(id));
            }
        }
    }

    /// After a claim: bring each group whose containing directory the claim
    /// probed up to the evidence.
    fn refresh_records(
        &mut self,
        evidence: &PathEvidence,
        parents: &HashMap<PathBuf, Option<Stamp>>,
    ) {
        for (parent, stamp) in parents {
            let Some(&slot) = self.slots.get(parent) else {
                continue;
            };
            let group = self.groups[slot]
                .as_mut()
                .expect("an indexed slot holds its group");
            match stamp {
                Some(stamp) => group.refresh(evidence, *stamp),
                None => group.record = None,
            }
        }
    }

    fn estimated_bytes(&self) -> usize {
        let ids = |ids: &BTreeSet<String>| {
            ids.iter()
                .map(|id| id.capacity() + std::mem::size_of::<String>())
                .sum::<usize>()
        };
        self.keys
            .iter()
            .map(|(key, holders)| key.capacity() + std::mem::size_of::<String>() + ids(holders))
            .sum::<usize>()
            + ids(&self.always)
            + self
                .slots
                .keys()
                .map(|parent| parent.as_os_str().len() + std::mem::size_of::<(PathBuf, usize)>())
                .sum::<usize>()
            + self
                .groups
                .iter()
                .flatten()
                .map(ChainGroup::estimated_bytes)
                .sum::<usize>()
    }
}

/// Where a claim's indexed visit is, in the linear visit's order: the
/// groups of the owner at an id come before the owner, the deeper first, and
/// a reserved folder after the recorded owner with the same id.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    Group { depth: Reverse<usize>, slot: usize },
    Recorded,
    Reserved(usize),
}

/// One claim's walk over the owners: the decision the linear visit makes for
/// each owner it reaches.
struct OwnerVisit<'a> {
    candidate: &'a str,
    roots: &'a [String],
    claimant: &'a str,
    keys: FolderOverlapKeys,
    cache: &'a mut PathEvidence,
    parents: HashMap<PathBuf, Option<Stamp>>,
    visited: usize,
    /// Claims that fell back to the linear visit part way.
    rest_visits: usize,
    #[cfg(test)]
    trail: Vec<String>,
}

impl OwnerVisit<'_> {
    fn visit(&mut self, owner: &HeldOwner) -> AppResult<()> {
        self.visited += 1;
        #[cfg(test)]
        self.trail.push(owner.title_id.clone());
        let (id, path) = (&owner.title_id, &owner.folder);
        let prefilter = owner
            .prefilter
            .as_ref()
            .expect("owners are prepared before the visit");
        let (candidate, roots) = (self.candidate, self.roots);
        let Self {
            keys,
            cache,
            parents,
            ..
        } = self;
        let compare = keys.needs_exact_comparison_prepared(prefilter, path, &mut |path| {
            cache.verify(path, parents)
        });
        cache.probes += keys.link_probes;
        keys.link_probes = 0;
        if !compare? {
            return Ok(());
        }
        cache.fallbacks += 1;
        let overlaps = title_folders_overlap(candidate, path).map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::folder_ownership::CANNOT_COMPARE_FOLDER_FOLDER_OWNED_TITLE,
                format!(
                    "cannot compare folder {candidate} with folder {path} owned by title {id}: {error}"
                ),
            )
        })?;
        let spans_root = matches!(
            crate::title_folder_rules::title_folder_root_violation(path, roots, roots),
            Some(
                crate::title_folder_rules::TitleFolderRootViolation::EqualsRoot
                    | crate::title_folder_rules::TitleFolderRootViolation::ContainsRoot
            )
        );
        if overlaps && !spans_root {
            return Err(AppError::validation_refused(
                crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE,
                format!("folder {candidate} overlaps the folder owned by title {id}"),
            ));
        }
        Ok(())
    }

    /// Recorded owners in id order, with reserved folders (sorted by title
    /// id) merged in as they will appear once recorded.
    fn visit_linear<'o>(
        &mut self,
        recorded: impl Iterator<Item = &'o HeldOwner>,
        reserved: &[HeldOwner],
    ) -> AppResult<()> {
        let mut reserved = reserved.iter().peekable();
        let mut recorded = recorded.peekable();
        while let Some(owner) = match (recorded.peek(), reserved.peek()) {
            (Some(left), Some(right)) if right.title_id < left.title_id => reserved.next(),
            (Some(_), _) => recorded.next(),
            (None, _) => reserved.next(),
        } {
            if owner.title_id == self.claimant {
                continue;
            }
            self.visit(owner)?;
        }
        Ok(())
    }

    /// [`Self::visit_linear`] through the index. An owner the linear visit
    /// would only find unrelated by key, with every walked directory proven
    /// by evidence that holds, is skipped; every other owner is visited in
    /// the same order with the same comparison. Each group's containing
    /// directory is probed when the first owner that walks it unflagged is
    /// reached, which is when the linear visit probes it.
    fn visit_indexed(
        &mut self,
        owners: &BTreeMap<String, HeldOwner>,
        index: &OwnerIndex,
        reserved: &[HeldOwner],
    ) -> AppResult<()> {
        let epoch = self.cache.invalidations;
        let claimant = self.claimant;
        let mut queue: BTreeSet<(String, Step)> = BTreeSet::new();
        let mut flagged: HashSet<String> = HashSet::new();
        let flag =
            |id: &str, queue: &mut BTreeSet<(String, Step)>, flagged: &mut HashSet<String>| {
                if id != claimant && flagged.insert(id.to_owned()) {
                    queue.insert((id.to_owned(), Step::Recorded));
                }
            };
        for candidate in &self.keys.candidate {
            index.related(candidate, |id| flag(id, &mut queue, &mut flagged));
        }
        for id in &index.always {
            flag(id, &mut queue, &mut flagged);
        }
        for (at, owner) in reserved.iter().enumerate() {
            if owner.title_id != claimant {
                queue.insert((owner.title_id.clone(), Step::Reserved(at)));
            }
        }
        for (slot, group) in index.groups.iter().enumerate() {
            if let Some(group) = group
                && let Some(first) = group.owners.first()
            {
                queue.insert((
                    first.clone(),
                    Step::Group {
                        depth: Reverse(group.depth),
                        slot,
                    },
                ));
            }
        }
        let mut held = Vec::new();
        while let Some((id, step)) = queue.pop_first() {
            match step {
                Step::Group { slot, .. } => {
                    let group = index.groups[slot]
                        .as_ref()
                        .expect("an indexed slot holds its group");
                    if id == claimant || flagged.contains(&id) {
                        // That owner is visited or skipped as a whole; the
                        // group waits for the next owner that walks it.
                        if let Some(next) = group
                            .owners
                            .range::<str, _>((Excluded(id.as_str()), Unbounded))
                            .find(|next| {
                                next.as_str() != claimant && !flagged.contains(next.as_str())
                            })
                        {
                            queue.insert((next.clone(), step));
                        }
                        continue;
                    }
                    let stamp = if cfg!(unix) {
                        self.cache.parent_stamp(&group.parent, &mut self.parents)
                    } else {
                        None
                    };
                    match (&group.record, stamp) {
                        (Some(record), Some(stamp))
                            if record.stamp == stamp && record.epoch == epoch =>
                        {
                            held.push(slot);
                            for child in &record.unverified {
                                for owner in group.children[child]
                                    .range::<str, _>((Included(id.as_str()), Unbounded))
                                {
                                    flag(owner, &mut queue, &mut flagged);
                                }
                            }
                        }
                        _ => {
                            for owner in group
                                .owners
                                .range::<str, _>((Included(id.as_str()), Unbounded))
                            {
                                flag(owner, &mut queue, &mut flagged);
                            }
                        }
                    }
                }
                Step::Recorded => {
                    let owner = owners.get(&id).expect("an indexed owner is held");
                    self.visit(owner)?;
                    if self.cache.invalidations != epoch {
                        return self
                            .visit_rest(owners, index, reserved, &held, &flagged, &id, None);
                    }
                }
                Step::Reserved(at) => {
                    self.visit(&reserved[at])?;
                    if self.cache.invalidations != epoch {
                        return self.visit_rest(
                            owners,
                            index,
                            reserved,
                            &held,
                            &flagged,
                            &id,
                            Some(at),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Evidence was removed during the claim, so records no longer prove
    /// what the linear visit would find: visit every owner after the one at
    /// `id` (the reserved folder at `reserved_at`, or the recorded owner) as
    /// the linear visit does, with the link answers it holds by now for the
    /// directories skipped owners walked.
    #[allow(clippy::too_many_arguments)]
    fn visit_rest(
        &mut self,
        owners: &BTreeMap<String, HeldOwner>,
        index: &OwnerIndex,
        reserved: &[HeldOwner],
        held: &[usize],
        flagged: &HashSet<String>,
        id: &str,
        reserved_at: Option<usize>,
    ) -> AppResult<()> {
        let walked_before = |owner: &str| match reserved_at {
            Some(_) => owner <= id,
            None => owner < id,
        };
        for &slot in held {
            let group = index.groups[slot]
                .as_ref()
                .expect("an indexed slot holds its group");
            let Some(record) = group.record.as_ref() else {
                continue;
            };
            for (child, walkers) in &group.children {
                if record.unverified.contains(child) {
                    continue;
                }
                if walkers
                    .iter()
                    .take_while(|owner| walked_before(owner))
                    .any(|owner| {
                        owner.as_str() != self.claimant && !flagged.contains(owner.as_str())
                    })
                {
                    self.keys.links.entry(child.clone()).or_insert(false);
                }
            }
        }
        let recorded = owners
            .range::<str, _>((Excluded(id), Unbounded))
            .map(|(_, owner)| owner);
        let rest = match reserved_at {
            Some(at) => &reserved[at + 1..],
            None => &reserved[reserved.partition_point(|owner| owner.title_id.as_str() < id)..],
        };
        self.rest_visits += 1;
        self.visit_linear(recorded, rest)
    }
}

#[derive(Default)]
struct HeldOwners {
    list: Option<HeldList>,
    /// Folders accepted for a claim whose write has not settled, by
    /// reservation: `(title id, folder)`.
    reservations: BTreeMap<u64, (String, String)>,
    reloads: u64,
    reuses: u64,
    unheld_reads: u64,
    /// Claims answered through the index, and the owners they visited.
    indexed_claims: u64,
    indexed_visits: u64,
}

impl HeldOwners {
    fn settle(&mut self, settlements: Vec<Settlement>) {
        for settlement in settlements {
            match settlement {
                Settlement::Recorded {
                    reservation,
                    title_id,
                    generation,
                    folder,
                } => {
                    self.reservations.remove(&reservation);
                    match (generation, self.list.as_mut()) {
                        (Some(generation), Some(list)) => {
                            list.apply_own_write(&title_id, folder, generation)
                        }
                        (None, _) => self.list = None,
                        (Some(_), None) => {}
                    }
                }
                Settlement::Released {
                    reservation,
                    write_started,
                } => {
                    self.reservations.remove(&reservation);
                    if write_started {
                        // The write may have committed: its effect is unknown.
                        self.list = None;
                    }
                }
            }
        }
    }
}

enum Settlement {
    Recorded {
        reservation: u64,
        title_id: String,
        generation: Option<u64>,
        folder: Option<String>,
    },
    Released {
        reservation: u64,
        write_started: bool,
    },
}

/// A folder accepted for a scan claim, held as recorded for the claim's title
/// until the claim's write settles. Dropping it unsettled releases it; once
/// its write has started, the scan reads the owners anew.
pub(crate) struct ScanClaimReservation {
    context: Arc<ScanOwnershipContext>,
    reservation: u64,
    title_id: String,
    write_started: bool,
    generation: Option<u64>,
    settled: bool,
}

impl ScanClaimReservation {
    pub(super) fn begin_write(&mut self) {
        self.write_started = true;
    }

    /// The counter value the claim's own write produced, if known.
    pub(super) fn wrote(&mut self, generation: Option<u64>) {
        self.generation = generation;
    }

    /// The write succeeded and the catalog now records `folder` for the
    /// claim's title.
    pub(super) fn recorded(mut self, folder: Option<String>) {
        self.settled = true;
        let settlement = Settlement::Recorded {
            reservation: self.reservation,
            title_id: std::mem::take(&mut self.title_id),
            generation: if self.write_started {
                self.generation
            } else {
                None
            },
            folder,
        };
        self.context.queue_settlement(settlement);
    }
}

impl Drop for ScanClaimReservation {
    fn drop(&mut self) {
        if !self.settled {
            self.context.queue_settlement(Settlement::Released {
                reservation: self.reservation,
                write_started: self.write_started,
            });
        }
    }
}

impl ScanOwnershipContext {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            admission: Arc::new(tokio::sync::Semaphore::new(1)),
            cache: Mutex::new(PathEvidence::default()),
            owners: Mutex::new(HeldOwners::default()),
            settlements: Mutex::new(Vec::new()),
            next_reservation: AtomicU64::new(1),
            #[cfg(test)]
            oracle: ClaimOracle::default(),
        })
    }

    pub(crate) fn current() -> Option<Arc<Self>> {
        OWNERSHIP_CONTEXT.try_with(Arc::clone).ok()
    }

    fn queue_settlement(&self, settlement: Settlement) {
        // A poisoned queue only loses settlements; the next claim then finds
        // the queue unusable and reads the owners anew.
        match self.settlements.lock() {
            Ok(mut queue) => queue.push(settlement),
            Err(_) => {
                if let Ok(mut owners) = self.owners.lock() {
                    owners.list = None;
                }
            }
        }
    }

    pub(super) async fn check(
        self: Arc<Self>,
        app: &AppUseCase,
        title: &Title,
        candidate: String,
    ) -> AppResult<()> {
        self.check_claim(app, title, candidate, None).await
    }

    /// [`Self::check`], and when it passes, the folder stays reserved for
    /// `title` until the returned reservation settles.
    pub(super) async fn check_and_reserve(
        self: Arc<Self>,
        app: &AppUseCase,
        title: &Title,
        candidate: String,
    ) -> AppResult<ScanClaimReservation> {
        let reservation = ScanClaimReservation {
            context: self.clone(),
            reservation: self.next_reservation.fetch_add(1, Ordering::SeqCst),
            title_id: title.id.clone(),
            write_started: false,
            generation: None,
            settled: false,
        };
        // On refusal or cancellation the guard drops and queues its release,
        // which the next claim applies after any insert this one made.
        self.check_claim(app, title, candidate, Some(reservation.reservation))
            .await?;
        Ok(reservation)
    }

    async fn check_claim(
        self: Arc<Self>,
        app: &AppUseCase,
        title: &Title,
        candidate: String,
        reserve: Option<u64>,
    ) -> AppResult<()> {
        // Admission precedes reads and dispatch. A cancelled waiter cannot leave
        // an unbounded queue of blocking tasks or reuse an old ownership result.
        let check_started = Instant::now();
        let permit = self
            .admission
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| {
                AppError::Repository(format!("ownership admission failed: {error}"))
            })?;
        let admission_wait_ms = elapsed_ms(check_started);
        let stage = Instant::now();
        let titles = &app.services.catalog.titles;
        // Taken before any read below, so a write that ends after it shows as
        // an advance at the next claim.
        let current = titles.title_folder_write_generation();
        let reused = {
            let settlements =
                std::mem::take(&mut *self.settlements.lock().map_err(|_| {
                    AppError::Repository("ownership settlement lock poisoned".into())
                })?);
            let mut held = self
                .owners
                .lock()
                .map_err(|_| AppError::Repository("ownership owners lock poisoned".into()))?;
            held.settle(settlements);
            let reused = held.list.as_mut().is_some_and(|list| list.holds(current));
            if reused {
                held.reuses += 1;
            } else {
                held.list = None;
            }
            reused
        };
        let mut unheld = None;
        if !reused {
            let mut owners = titles.list_title_folder_owners().await?;
            // Visited by title id whichever collation the catalog sorts with;
            // a no-op for the byte order SQLite returns.
            owners.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            let estimate = estimated_unprepared_bytes(&owners);
            let mut held = self
                .owners
                .lock()
                .map_err(|_| AppError::Repository("ownership owners lock poisoned".into()))?;
            let owners_iter = owners
                .into_iter()
                .map(|(title_id, folder)| HeldOwner::new(title_id, folder));
            match current {
                Some(generation) if estimate <= self.held_owners_byte_budget() => {
                    held.reloads += 1;
                    held.list = Some(HeldList {
                        generation,
                        own: Default::default(),
                        owners: owners_iter
                            .map(|owner| (owner.title_id.clone(), owner))
                            .collect(),
                        prepared: None,
                    });
                }
                _ => {
                    held.unheld_reads += 1;
                    unheld = Some(owners_iter.collect::<Vec<_>>());
                }
            }
        }
        let owners_read_ms = elapsed_ms(stage);
        // Roots come from the scan's snapshot when one is in scope, the same
        // view the root and library-boundary checks use for this claim.
        let stage = Instant::now();
        let roots: Vec<_> = match ScanFolderRoots::current_all() {
            Some(roots) => roots,
            None => app
                .all_library_root_folders()
                .await?
                .into_iter()
                .map(|root| root.path)
                .collect(),
        };
        let roots_ms = elapsed_ms(stage);
        let title_id = title.id.clone();
        let dispatched = Instant::now();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let blocking_queue_ms = elapsed_ms(dispatched);
            let started = Instant::now();
            let mut cache = self.cache.lock().map_err(|_| AppError::Repository("ownership evidence lock poisoned".into()))?;
            let mut held = self.owners.lock().map_err(|_| AppError::Repository("ownership owners lock poisoned".into()))?;
            let lock_wait_ms = elapsed_ms(started);
            let before = (cache.probes, cache.hits, cache.invalidations, cache.fallbacks);
            let mut owner_count = 0usize;
            let mut indexed = false;
            let result = (|| {
                let keys = FolderOverlapKeys::from_roots(&candidate, &roots)?;
                cache.set_roots(keys.root_keys());
                let root_keys = keys.root_keys().to_vec();
                let HeldOwners { list, reservations, .. } = &mut *held;
                let mut reserved: Vec<HeldOwner> = reservations
                    .values()
                    .map(|(id, folder)| {
                        let mut owner = HeldOwner::new(id.clone(), folder.clone());
                        owner.prefilter = Some(OwnerOverlapPrefilter::prepare(folder, &root_keys));
                        owner
                    })
                    .collect();
                reserved.sort_by(|left, right| left.title_id.cmp(&right.title_id));
                if let (None, Some(list)) = (unheld.as_ref(), list.as_mut()) {
                    list.prepare(&root_keys, &cache);
                }
                #[cfg(test)]
                let expected = match (unheld.as_ref(), list.as_ref()) {
                    (None, Some(list)) if self.oracle.compare.load(Ordering::SeqCst) => {
                        let mut shadow = (*cache).clone();
                        let (outcome, parents) = {
                            let mut oracle = OwnerVisit {
                                candidate: &candidate,
                                roots: &roots,
                                claimant: &title_id,
                                keys: FolderOverlapKeys::from_roots(&candidate, &roots)?,
                                cache: &mut shadow,
                                parents: HashMap::new(),
                                visited: 0,
                                rest_visits: 0,
                                #[cfg(test)]
                                trail: Vec::new(),
                            };
                            let outcome = claim_outcome(&oracle.visit_linear(list.owners.values(), &reserved));
                            (outcome, (oracle.parents, oracle.trail))
                        };
                        Some((outcome, shadow, parents))
                    }
                    _ => None,
                };
                #[cfg(test)]
                {
                    cache.replay = expected.as_ref().map(|(_, _, (parents, _))| parents.clone());
                }
                let mut visit = OwnerVisit {
                    candidate: &candidate,
                    roots: &roots,
                    claimant: &title_id,
                    keys,
                    cache: &mut cache,
                    parents: HashMap::new(),
                    visited: 0,
                    rest_visits: 0,
                    #[cfg(test)]
                    trail: Vec::new(),
                };
                let outcome = match (unheld.as_mut(), list.as_mut()) {
                    (Some(unheld), _) => {
                        for owner in unheld.iter_mut() {
                            owner.prefilter = Some(OwnerOverlapPrefilter::prepare(&owner.folder, &root_keys));
                        }
                        visit.visit_linear(unheld.iter(), &reserved)
                    }
                    (None, Some(list)) => {
                        let HeldList { owners, prepared, .. } = list;
                        let (_, index) = prepared.as_mut().expect("held owners are prepared above");
                        #[cfg(test)]
                        let linear = self.oracle.linear.load(Ordering::SeqCst);
                        #[cfg(not(test))]
                        let linear = false;
                        let outcome = if linear {
                            visit.visit_linear(owners.values(), &reserved)
                        } else {
                            indexed = true;
                            visit.visit_indexed(owners, index, &reserved)
                        };
                        index.refresh_records(visit.cache, &visit.parents);
                        outcome
                    }
                    (None, None) => {
                        return Err(AppError::Repository("ownership owners were not loaded".into()));
                    }
                };
                owner_count = visit.visited;
                #[cfg(test)]
                {
                    visit.cache.replay = None;
                }
                #[cfg(test)]
                self.oracle.rests.fetch_add(visit.rest_visits as u64, Ordering::SeqCst);
                #[cfg(test)]
                if let Some((expected, shadow, (parents, trail))) = expected {
                    self.oracle.compared.fetch_add(1, Ordering::SeqCst);
                    let actual = claim_outcome(&outcome);
                    let evidence = &*visit.cache;
                    if expected != actual
                        || shadow.probes != evidence.probes
                        || shadow.invalidations != evidence.invalidations
                        || shadow.directories != evidence.directories
                        || parents != visit.parents
                    {
                        self.oracle.mismatches.lock().unwrap().push(format!(
                            "{candidate}: linear {expected:?} probes {} invalidations {}; indexed {actual:?} probes {} invalidations {}",
                            shadow.probes, shadow.invalidations, evidence.probes, evidence.invalidations,
                        ) + &format!(" linear trail {trail:?} indexed trail {:?} claim {}", visit.trail, self.oracle.compared.load(Ordering::SeqCst)
                        ));
                    }
                }
                outcome?;
                if let Some(reservation) = reserve {
                    // Still under the permit: the next claim compares with it.
                    reservations.insert(reservation, (title_id.clone(), candidate.clone()));
                }
                Ok(())
            })();
            cache.checks += 1;
            cache.elapsed += started.elapsed();
            if indexed {
                held.indexed_claims += 1;
                held.indexed_visits += owner_count as u64;
            }
            tracing::debug!(
                folder = %candidate,
                admission_wait_ms,
                owners_read_ms,
                owners_reused = reused,
                owners_indexed = indexed,
                owners = owner_count,
                roots_ms,
                blocking_queue_ms,
                lock_wait_ms,
                compare_ms = elapsed_ms(started),
                probes = cache.probes - before.0,
                hits = cache.hits - before.1,
                invalidations = cache.invalidations - before.2,
                fallbacks = cache.fallbacks - before.3,
                ok = result.is_ok(),
                total_ms = elapsed_ms(check_started),
                "scan ownership check timing"
            );
            result
        }).await.map_err(|error| AppError::Repository(format!("ownership verification task failed to join: {error}")))?
    }
}

impl Drop for ScanOwnershipContext {
    fn drop(&mut self) {
        if let (Ok(cache), Ok(held)) = (self.cache.lock(), self.owners.lock()) {
            tracing::debug!(
                checks = cache.checks,
                hits = cache.hits,
                invalidations = cache.invalidations,
                fallback_checks = cache.fallbacks,
                evidence_probes = cache.probes,
                elapsed_ms = cache.elapsed.as_millis(),
                entries = cache.directories.len(),
                estimated_cache_bytes = cache.estimated_bytes(),
                owner_reloads = held.reloads,
                owner_reuses = held.reuses,
                owner_unheld_reads = held.unheld_reads,
                owner_indexed_claims = held.indexed_claims,
                owner_indexed_visits = held.indexed_visits,
                held_owners = held.list.as_ref().map_or(0, |list| list.owners.len()),
                estimated_held_owner_bytes =
                    held.list.as_ref().map_or(0, HeldList::estimated_bytes),
                "scan ownership evidence summary"
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

fn stamp(metadata: &std::fs::Metadata) -> Option<Stamp> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Absent identity/change markers and coarse timestamps cannot prove
        // freshness between rapid claims. Keep those filesystems on fallback.
        (metadata.is_dir()
            && metadata.ino() != 0
            && metadata.ctime() != 0
            && metadata.ctime_nsec() != 0)
            .then(|| Stamp {
                device: metadata.dev(),
                inode: metadata.ino(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            })
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

#[cfg(test)]
fn claim_outcome(outcome: &AppResult<()>) -> Result<(), (Option<&'static str>, String)> {
    outcome
        .as_ref()
        .map(|_| ())
        .map_err(|error| (error.validation_reason(), error.to_string()))
}

#[derive(Default)]
#[cfg_attr(test, derive(Clone))]
struct PathEvidence {
    // Each proven non-link directory is bound to its containing directory's
    // identity/change stamp. Never store missing paths or linked entries.
    directories: HashMap<PathBuf, Stamp>,
    roots: Vec<(String, Option<String>)>,
    probes: u64,
    hits: u64,
    invalidations: u64,
    fallbacks: u64,
    checks: u64,
    elapsed: std::time::Duration,
    /// The oracle's parent stamps, so the compared visit sees the same moment
    /// of a shared directory (the system temporary directory) that other
    /// processes change between the two visits. Each use still counts a probe.
    #[cfg(test)]
    replay: Option<HashMap<PathBuf, Option<Stamp>>>,
}

impl PathEvidence {
    fn estimated_bytes(&self) -> usize {
        self.directories.capacity()
            * (std::mem::size_of::<PathBuf>() + std::mem::size_of::<Stamp>() + 1)
            + self
                .directories
                .keys()
                .map(|path| path.as_os_str().len())
                .sum::<usize>()
    }
    fn set_roots(&mut self, roots: &[(String, Option<String>)]) {
        if self.roots != roots {
            self.invalidations += self.directories.len() as u64;
            self.directories.clear();
            self.roots = roots.to_vec();
        }
    }

    /// A containing directory's stamp, probed once per claim.
    fn parent_stamp(
        &mut self,
        parent: &Path,
        parents: &mut HashMap<PathBuf, Option<Stamp>>,
    ) -> Option<Stamp> {
        if let Some(current) = parents.get(parent) {
            return *current;
        }
        self.probes += 1;
        #[cfg(test)]
        if let Some(current) = self.replay.as_ref().and_then(|replay| replay.get(parent)) {
            parents.insert(parent.to_path_buf(), *current);
            return *current;
        }
        let current = std::fs::symlink_metadata(parent)
            .ok()
            .as_ref()
            .and_then(stamp);
        parents.insert(parent.to_path_buf(), current);
        current
    }

    fn verify(&mut self, path: &Path, parents: &mut HashMap<PathBuf, Option<Stamp>>) -> bool {
        if !cfg!(unix) {
            return false;
        }
        let Some(parent) = path.parent() else {
            return false;
        };
        let current = self.parent_stamp(parent, parents);
        if let Some(previous) = self.directories.get(path) {
            if Some(*previous) == current {
                self.hits += 1;
                return true;
            }
            // The ancestor walk also verifies every intermediate component.
            // Invalidate the containing group once, rather than rescanning the
            // cache separately for every sibling after a directory change.
            let before = self.directories.len();
            self.directories
                .retain(|entry, _| !entry.starts_with(parent));
            self.invalidations += (before - self.directories.len()) as u64;
        }
        let Some(current) = current else {
            return false;
        };
        self.probes += 1;
        let verified = std::fs::symlink_metadata(path)
            .ok()
            .is_some_and(|metadata| metadata.is_dir());
        if verified && self.directories.len() < 65_536 {
            self.directories.insert(path.to_path_buf(), current);
        }
        verified
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::TitleRepository as _;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn compare(cache: &mut PathEvidence, root: &Path, candidate: &Path, owner: &Path) -> bool {
        let roots = vec![path_to_stored_string(root)];
        let candidate = path_to_stored_string(candidate);
        let owner = path_to_stored_string(owner);
        let mut cached = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
        cache.set_roots(&cached.roots);
        let actual = cached.needs_exact_comparison_with_evidence(&owner, &mut |path| {
            cache.verify(path, &mut HashMap::new())
        });
        let mut original = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
        let expected = original.needs_exact_comparison(&owner);
        match (actual, expected) {
            (Ok(actual), Ok(expected)) => {
                assert_eq!(actual, expected);
                actual
            }
            (Err(actual), Err(expected)) => {
                assert_eq!(actual.validation_reason(), expected.validation_reason());
                true
            }
            _ => panic!("cached and original decisions differ"),
        }
    }

    #[test]
    fn scan_ownership_evidence_tracks_replacements_links_and_roots() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let candidate = root.join("candidate");
        let owner = root.join("owner");
        std::fs::create_dir_all(&candidate).unwrap();
        std::fs::create_dir(&owner).unwrap();
        let sentinel = candidate.join("preserved.mkv");
        std::fs::write(&sentinel, b"untouched").unwrap();
        let mut cache = PathEvidence::default();
        assert!(!compare(&mut cache, &root, &candidate, &owner));
        assert!(!compare(&mut cache, &root, &candidate, &owner));
        let reusable = stamp(&std::fs::symlink_metadata(&root).unwrap()).is_some();
        assert_eq!(cache.hits > 0, reusable);
        assert!(compare(
            &mut cache,
            &root,
            &candidate,
            &candidate.join("child")
        ));
        assert!(compare(&mut cache, &root, &candidate, &root));
        std::fs::rename(&owner, root.join("saved-owner")).unwrap();
        symlink(&candidate, &owner).unwrap();
        assert!(compare(&mut cache, &root, &candidate, &owner));
        std::fs::remove_file(&owner).unwrap();
        symlink(root.join("saved-owner"), &owner).unwrap();
        assert!(compare(&mut cache, &root, &candidate, &owner));
        assert_eq!(cache.invalidations > 0, reusable);
        assert!(!compare(
            &mut cache,
            &root,
            &candidate,
            &root.join("missing")
        ));
        let alias = temp.path().join("alias");
        symlink(&root, &alias).unwrap();
        assert!(compare(
            &mut cache,
            &alias,
            &candidate,
            &alias.join("candidate")
        ));
        assert_eq!(std::fs::read(sentinel).unwrap(), b"untouched");
    }

    #[test]
    fn scan_ownership_evidence_rechecks_replaced_ancestors_and_inaccessible_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let candidate = root.join("candidate");
        let group = root.join("group");
        let owner = group.join("owner");
        std::fs::create_dir(&candidate).unwrap();
        std::fs::create_dir_all(&owner).unwrap();
        let mut cache = PathEvidence::default();
        assert!(!compare(&mut cache, root, &candidate, &owner));
        std::fs::rename(&group, root.join("saved-group")).unwrap();
        symlink(&candidate, &group).unwrap();
        assert!(compare(&mut cache, root, &candidate, &owner));
        let inaccessible = root.join("restricted");
        std::fs::create_dir(&inaccessible).unwrap();
        std::fs::set_permissions(&inaccessible, std::fs::Permissions::from_mode(0o000)).unwrap();
        compare(&mut cache, root, &candidate, &inaccessible.join("child"));
        std::fs::set_permissions(&inaccessible, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn scan_ownership_evidence_rechecks_replaced_and_retargeted_roots() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let candidate = root.join("candidate");
        let owner = root.join("owner");
        std::fs::create_dir_all(&candidate).unwrap();
        std::fs::create_dir(&owner).unwrap();
        let mut cache = PathEvidence::default();
        assert!(!compare(&mut cache, &root, &candidate, &owner));
        std::fs::rename(&root, temp.path().join("old-root")).unwrap();
        std::fs::create_dir_all(&candidate).unwrap();
        symlink(&candidate, &owner).unwrap();
        assert!(compare(&mut cache, &root, &candidate, &owner));
        let alias = temp.path().join("alias");
        symlink(temp.path().join("old-root"), &alias).unwrap();
        assert!(!compare(
            &mut cache,
            &alias,
            &candidate,
            &alias.join("owner")
        ));
        std::fs::remove_file(&alias).unwrap();
        symlink(&root, &alias).unwrap();
        assert!(compare(
            &mut cache,
            &alias,
            &candidate,
            &alias.join("owner")
        ));
    }

    #[tokio::test]
    async fn scan_ownership_cancelled_admission_does_not_dispatch_or_touch_files() {
        let temp = tempfile::tempdir().unwrap();
        let sentinel = temp.path().join("preserved.mkv");
        std::fs::write(&sentinel, b"preserved").unwrap();
        let (app, actor) = crate::lib_tests::bootstrap();
        let title = app
            .create_title_without_hydration_in_library(
                &actor,
                crate::NewTitle {
                    name: "Cancellation Fixture".into(),
                    facet: scryer_domain::MediaFacet::Movie,
                    ..Default::default()
                },
                scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie),
            )
            .await
            .unwrap()
            .title;
        let context = ScanOwnershipContext::new();
        let permit = context.admission.clone().acquire_owned().await.unwrap();
        {
            let check = context
                .clone()
                .check(&app, &title, path_to_stored_string(temp.path()));
            tokio::pin!(check);
            tokio::select! {
                biased;
                _ = &mut check => panic!("ownership work must wait for admission"),
                _ = std::future::ready(()) => {}
            }
        }
        drop(permit);
        assert_eq!(context.admission.available_permits(), 1);
        assert_eq!(context.cache.lock().unwrap().checks, 0);
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserved");
    }

    async fn set_movie_roots(app: &AppUseCase, roots: &[&Path]) -> String {
        let library_id =
            scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie);
        let library = app
            .services
            .catalog
            .libraries
            .get_by_id(&library_id)
            .await
            .unwrap()
            .unwrap();
        app.services
            .catalog
            .libraries
            .update(
                &library_id,
                library.name,
                library.slug,
                roots
                    .iter()
                    .enumerate()
                    .map(|(index, root)| crate::LibraryRootDraft {
                        id: None,
                        path: path_to_stored_string(root),
                        is_default: index == 0,
                    })
                    .collect(),
            )
            .await
            .unwrap();
        library_id
    }

    #[tokio::test]
    async fn scan_ownership_claims_use_the_scan_root_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let moved_root = temp.path().join("moved-library");
        std::fs::create_dir_all(&moved_root).unwrap();
        let folders: Vec<_> = (0..3)
            .map(|index| root.join(format!("Synthetic Fixture {index} (2001)")))
            .collect();
        for folder in &folders {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join("preserved.mkv"), b"preserved").unwrap();
        }
        let (app, actor) = crate::lib_tests::bootstrap();
        let library_id = set_movie_roots(&app, &[&root]).await;
        let mut titles = Vec::new();
        for index in 0..folders.len() {
            titles.push(
                app.create_title_without_hydration_in_library(
                    &actor,
                    crate::NewTitle {
                        name: format!("Synthetic Fixture {index}"),
                        facet: scryer_domain::MediaFacet::Movie,
                        ..Default::default()
                    },
                    library_id.clone(),
                )
                .await
                .unwrap()
                .title,
            );
        }
        let snapshot = ScanFolderRoots::load(&app).await.unwrap();
        let snapshot_keys =
            FolderOverlapKeys::from_roots(&path_to_stored_string(&folders[0]), &snapshot.all)
                .unwrap()
                .roots;
        // The catalog moves the library elsewhere mid-scan. Every claim in this
        // scan still answers from the snapshot, so none of them re-reads the
        // roots: a fresh read would place each folder outside its library.
        set_movie_roots(&app, &[&moved_root]).await;
        let context = ScanOwnershipContext::new();
        SCAN_FOLDER_ROOTS
            .scope(
                snapshot,
                with_scan_ownership(Some(context.clone()), async {
                    for (title, folder) in titles.iter_mut().zip(&folders) {
                        let candidate = path_to_stored_string(folder);
                        assert!(
                            !title_folder_is_outside_its_library(&app, title, &candidate)
                                .await
                                .unwrap()
                        );
                        assert!(!folder_spans_a_library_root(&app, &candidate).await.unwrap());
                        claim_title_folder_for_scan_if_missing(&app, title, folder)
                            .await
                            .unwrap();
                        assert_eq!(title.folder_path.as_deref(), Some(candidate.as_str()));
                    }
                }),
            )
            .await;
        {
            let cache = context.cache.lock().unwrap();
            assert_eq!(cache.roots, snapshot_keys);
            assert!(cache.checks >= folders.len() as u64);
        }
        // Outside the scan the same check reads the catalog's current roots.
        assert!(
            title_folder_is_outside_its_library(
                &app,
                &titles[0],
                &path_to_stored_string(&folders[0])
            )
            .await
            .unwrap()
        );
        for folder in &folders {
            assert_eq!(
                std::fs::read(folder.join("preserved.mkv")).unwrap(),
                b"preserved"
            );
        }
    }

    #[test]
    fn scan_ownership_flat_roots_probe_shared_parent_once_per_claim() {
        for count in [100, 10_000] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let roots = vec![path_to_stored_string(root)];
            let candidate = root.join("candidate");
            std::fs::create_dir(&candidate).unwrap();
            let candidate = path_to_stored_string(&candidate);
            let owners: Vec<_> = (0..count)
                .map(|index| {
                    let path = root.join(format!("title-{index}"));
                    std::fs::create_dir(&path).unwrap();
                    path_to_stored_string(&path)
                })
                .collect();
            let claims = 32;
            let reusable = stamp(&std::fs::symlink_metadata(root).unwrap()).is_some();
            let start = Instant::now();
            let mut original_probes = 0;
            for _ in 0..claims {
                let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
                for owner in &owners {
                    assert!(!keys.needs_exact_comparison(owner).unwrap());
                }
                original_probes += keys.link_probes;
            }
            let original_elapsed = start.elapsed();
            let start = Instant::now();
            let mut cache = PathEvidence::default();
            for claim in 0..claims {
                let before = cache.probes;
                let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
                cache.set_roots(&keys.roots);
                let mut parents = HashMap::new();
                for owner in &owners {
                    assert!(
                        !keys
                            .needs_exact_comparison_with_evidence(owner, &mut |path| cache
                                .verify(path, &mut parents))
                            .unwrap()
                    );
                }
                assert_eq!(keys.link_probes, if reusable { 0 } else { count });
                cache.probes += keys.link_probes;
                assert_eq!(
                    cache.probes - before,
                    if claim == 0 || !reusable {
                        count + 1
                    } else {
                        1
                    }
                );
            }
            assert_eq!(original_probes, count * claims);
            eprintln!(
                "ownership prefilter benchmark: titles={count} claims={claims} original_probes={original_probes} cached_probes={} original_ms={} cached_ms={} estimated_cache_bytes={}",
                cache.probes,
                original_elapsed.as_millis(),
                start.elapsed().as_millis(),
                cache.estimated_bytes()
            );
        }
    }

    async fn synthetic_movie(
        app: &AppUseCase,
        actor: &scryer_domain::User,
        library_id: &str,
        name: &str,
    ) -> Title {
        app.create_title_without_hydration_in_library(
            actor,
            crate::NewTitle {
                name: name.into(),
                facet: scryer_domain::MediaFacet::Movie,
                ..Default::default()
            },
            library_id.to_owned(),
        )
        .await
        .unwrap()
        .title
    }

    fn reason(result: AppResult<()>) -> Result<(), Option<&'static str>> {
        result.map_err(|error| error.validation_reason())
    }

    /// Every candidate gets the same answer from the scan's held owners as
    /// from the check outside a scan, which reads the catalog anew.
    async fn held_matches_fresh(
        app: &AppUseCase,
        context: &Arc<ScanOwnershipContext>,
        title: &Title,
        candidates: &[PathBuf],
    ) -> Vec<Result<(), Option<&'static str>>> {
        let mut decisions = Vec::new();
        for candidate in candidates {
            let held = with_scan_ownership(
                Some(context.clone()),
                ensure_folder_move_available_to_title(app, title, candidate),
            )
            .await;
            let fresh = ensure_folder_move_available_to_title(app, title, candidate).await;
            let (held, fresh) = (reason(held), reason(fresh));
            assert_eq!(held, fresh, "candidate {}", candidate.display());
            decisions.push(held);
        }
        decisions
    }

    fn owner_stats(context: &ScanOwnershipContext) -> (u64, u64) {
        let held = context.owners.lock().unwrap();
        (held.reloads, held.reuses)
    }

    #[tokio::test]
    async fn scan_ownership_held_owners_answer_as_a_fresh_read() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let second_root = temp.path().join("second-library");
        for folder in ["Emberfall", "Cinderwake", "Pinecrest", "Gloamridge"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        std::fs::create_dir_all(&second_root).unwrap();
        let sentinel = root.join("Pinecrest").join("preserved.mkv");
        std::fs::write(&sentinel, b"preserved").unwrap();
        let (app, actor, titles) = crate::lib_tests::bootstrap_with_title_repo();
        titles.counts_folder_writes.store(true, Ordering::SeqCst);
        let library_id = set_movie_roots(&app, &[&root]).await;
        let mut emberfall = synthetic_movie(&app, &actor, &library_id, "Emberfall").await;
        let mut cinderwake = synthetic_movie(&app, &actor, &library_id, "Cinderwake").await;
        let mut ashgrove = synthetic_movie(&app, &actor, &library_id, "Ashgrove").await;
        let mut duskmere = synthetic_movie(&app, &actor, &library_id, "Duskmere").await;
        let probe = synthetic_movie(&app, &actor, &library_id, "Hollowmere").await;
        claim_title_folder_if_missing(&app, &mut emberfall, &root.join("Emberfall"))
            .await
            .unwrap();
        claim_title_folder_if_missing(&app, &mut cinderwake, &root.join("Cinderwake"))
            .await
            .unwrap();
        let candidates: Vec<PathBuf> = vec![
            root.join("Emberfall"),
            root.join("Emberfall").join("Extras"),
            root.join("Cinderwake"),
            root.join("Cinderwake Moved"),
            root.join("Ashgrove").join("Featurettes"),
            root.join("Duskmere"),
            root.join("Gloamridge"),
            root.join("Pinecrest"),
            root.join("Thornvale"),
            second_root.join("Thornvale"),
        ];
        let context = ScanOwnershipContext::new();
        let overlap = Err(Some(
            crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE,
        ));

        let decisions = held_matches_fresh(&app, &context, &probe, &candidates).await;
        assert_eq!(decisions[0], overlap);
        assert_eq!(decisions[1], overlap);
        assert_eq!(decisions[5], Ok(()));

        // The scan's own claim changes nothing but what it records itself.
        let (reloads, _) = owner_stats(&context);
        with_scan_ownership(
            Some(context.clone()),
            claim_title_folder_for_scan_if_missing(&app, &mut ashgrove, &root.join("Ashgrove")),
        )
        .await
        .unwrap();
        let decisions = held_matches_fresh(&app, &context, &probe, &candidates).await;
        assert_eq!(decisions[4], overlap);
        assert_eq!(
            owner_stats(&context).0,
            reloads,
            "an own claim must not force a read"
        );

        // Another writer adds a title that records a folder.
        let mut gloamridge = emberfall.clone();
        gloamridge.id = "synthetic-gloamridge".into();
        gloamridge.name = "Gloamridge".into();
        gloamridge.folder_path = Some(path_to_stored_string(&root.join("Gloamridge")));
        titles.create(gloamridge).await.unwrap();
        let decisions = held_matches_fresh(&app, &context, &probe, &candidates).await;
        assert_eq!(decisions[6], overlap);
        assert_eq!(owner_stats(&context).0, reloads + 1);

        // Another writer moves a title's folder.
        std::fs::rename(root.join("Cinderwake"), root.join("Cinderwake Moved")).unwrap();
        titles
            .assign_title_folders(&[crate::TitleFolderWrite::assign(
                &cinderwake.id,
                path_to_stored_string(&root.join("Cinderwake Moved")),
            )
            .expecting(cinderwake.folder_path.clone())])
            .await
            .unwrap();
        let decisions = held_matches_fresh(&app, &context, &probe, &candidates).await;
        assert_eq!(decisions[2], Ok(()));
        assert_eq!(decisions[3], overlap);

        // An own claim after outside writes.
        with_scan_ownership(
            Some(context.clone()),
            claim_title_folder_for_scan_if_missing(&app, &mut duskmere, &root.join("Duskmere")),
        )
        .await
        .unwrap();
        let decisions = held_matches_fresh(&app, &context, &probe, &candidates).await;
        assert_eq!(decisions[5], overlap);

        // Another writer deletes a title.
        titles.delete(&emberfall.id).await.unwrap();
        let decisions = held_matches_fresh(&app, &context, &probe, &candidates).await;
        assert_eq!(decisions[0], Ok(()));
        assert_eq!(decisions[1], Ok(()));

        // An owner's directory is swapped for a link into an unowned folder.
        std::fs::rename(root.join("Cinderwake Moved"), temp.path().join("saved")).unwrap();
        symlink(root.join("Pinecrest"), root.join("Cinderwake Moved")).unwrap();
        let (reloads, _) = owner_stats(&context);
        let decisions = held_matches_fresh(&app, &context, &probe, &candidates).await;
        assert_eq!(decisions[7], overlap);
        assert_eq!(
            owner_stats(&context).0,
            reloads,
            "a link swap is filesystem evidence"
        );

        // The library's roots change; the held keys are prepared again.
        set_movie_roots(&app, &[&second_root]).await;
        held_matches_fresh(&app, &context, &probe, &candidates).await;
        let third_root = temp.path().join("third-library");
        std::fs::create_dir_all(&third_root).unwrap();
        set_movie_roots(&app, &[&third_root]).await;
        held_matches_fresh(&app, &context, &probe, &candidates).await;

        let (reloads, reuses) = owner_stats(&context);
        assert!(reuses > reloads, "reloads={reloads} reuses={reuses}");
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserved");
    }

    #[tokio::test]
    async fn scan_ownership_without_a_counter_reads_every_claim() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        std::fs::create_dir_all(root.join("Emberfall")).unwrap();
        let (app, actor, _) = crate::lib_tests::bootstrap_with_title_repo();
        let library_id = set_movie_roots(&app, &[&root]).await;
        let mut emberfall = synthetic_movie(&app, &actor, &library_id, "Emberfall").await;
        let probe = synthetic_movie(&app, &actor, &library_id, "Hollowmere").await;
        claim_title_folder_if_missing(&app, &mut emberfall, &root.join("Emberfall"))
            .await
            .unwrap();
        let context = ScanOwnershipContext::new();
        held_matches_fresh(
            &app,
            &context,
            &probe,
            &[root.join("Emberfall"), root.join("Cinderwake")],
        )
        .await;
        let held = context.owners.lock().unwrap();
        assert_eq!((held.reloads, held.reuses, held.unheld_reads), (0, 0, 2));
        assert!(held.list.is_none());
    }

    #[tokio::test]
    async fn scan_ownership_reserves_an_accepted_claim_until_its_write_settles() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        std::fs::create_dir_all(&root).unwrap();
        let (app, actor, titles) = crate::lib_tests::bootstrap_with_title_repo();
        titles.counts_folder_writes.store(true, Ordering::SeqCst);
        let library_id = set_movie_roots(&app, &[&root]).await;
        let mut emberfall = synthetic_movie(&app, &actor, &library_id, "Emberfall").await;
        let mut cinderwake = synthetic_movie(&app, &actor, &library_id, "Cinderwake").await;
        let parent = root.join("Emberfall");
        let nested = parent.join("Cinderwake");
        let overlap =
            Some(crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE);
        let context = ScanOwnershipContext::new();

        // Two nested claims admitted back to back, before either writes.
        let mut first = context
            .clone()
            .check_and_reserve(&app, &emberfall, path_to_stored_string(&parent))
            .await
            .unwrap();
        let second = context
            .clone()
            .check_and_reserve(&app, &cinderwake, path_to_stored_string(&nested))
            .await;
        assert_eq!(second.err().unwrap().validation_reason(), overlap);
        // The catalog does not record it yet; only the scan holds it back.
        ensure_folder_move_available_to_title(&app, &cinderwake, &nested)
            .await
            .unwrap();
        // A reservation never refuses its own title.
        context
            .clone()
            .check(&app, &emberfall, path_to_stored_string(&parent))
            .await
            .unwrap();

        // The first claim's write fails: its reservation goes with it.
        first.begin_write();
        first.wrote(None);
        drop(first);
        context
            .clone()
            .check(&app, &cinderwake, path_to_stored_string(&nested))
            .await
            .unwrap();
        assert!(context.owners.lock().unwrap().reservations.is_empty());

        // A reservation released before any write.
        let reserved = context
            .clone()
            .check_and_reserve(&app, &cinderwake, path_to_stored_string(&nested))
            .await
            .unwrap();
        drop(reserved);
        context
            .clone()
            .check(&app, &emberfall, path_to_stored_string(&parent))
            .await
            .unwrap();
        assert!(context.owners.lock().unwrap().reservations.is_empty());

        // Through the claim path: a failed write releases the reservation.
        titles
            .fail_folder_path_writes_for(&emberfall.id, "synthetic write failure")
            .await;
        with_scan_ownership(
            Some(context.clone()),
            claim_title_folder_for_scan_if_missing(&app, &mut emberfall, &parent),
        )
        .await
        .unwrap_err();
        assert!(emberfall.folder_path.is_none());
        with_scan_ownership(
            Some(context.clone()),
            claim_title_folder_for_scan_if_missing(&app, &mut cinderwake, &nested),
        )
        .await
        .unwrap();
        // The failed claim's reservation is gone; the recorded claim's own
        // settles at the next claim.
        assert!(
            context
                .owners
                .lock()
                .unwrap()
                .reservations
                .values()
                .all(|(title_id, _)| *title_id == cinderwake.id)
        );

        // Two real nested claims racing in one scan: exactly one records.
        let mut ashgrove = synthetic_movie(&app, &actor, &library_id, "Ashgrove").await;
        let mut duskmere = synthetic_movie(&app, &actor, &library_id, "Duskmere").await;
        let outer = root.join("Ashgrove");
        let inner = outer.join("Duskmere");
        let (left, right) = tokio::join!(
            with_scan_ownership(
                Some(context.clone()),
                claim_title_folder_for_scan_if_missing(&app, &mut ashgrove, &outer),
            ),
            with_scan_ownership(
                Some(context.clone()),
                claim_title_folder_for_scan_if_missing(&app, &mut duskmere, &inner),
            ),
        );
        let refused: Vec<_> = [left, right]
            .into_iter()
            .filter_map(Result::err)
            .map(|error| error.validation_reason())
            .collect();
        assert_eq!(refused, vec![overlap]);
        assert_eq!(
            [&ashgrove, &duskmere]
                .iter()
                .filter(|title| title.folder_path.is_some())
                .count(),
            1
        );
        assert!(context.owners.lock().unwrap().reservations.len() <= 1);
    }

    #[tokio::test]
    async fn scan_ownership_held_owners_claim_cost() {
        for count in [100usize, 10_000, 100_000] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("library");
            std::fs::create_dir_all(&root).unwrap();
            let owners: Vec<String> = (0..count)
                .map(|index| {
                    let path = root.join(format!("Emberfall {index}"));
                    std::fs::create_dir(&path).unwrap();
                    path_to_stored_string(&path)
                })
                .collect();
            let candidate = path_to_stored_string(&root.join("Cinderwake"));
            let roots = vec![path_to_stored_string(&root)];
            let claims = if count > 10_000 { 4u32 } else { 16 };

            // The comparison alone: keys built per owner per claim, as before,
            // against keys prepared once and reused.
            let mut cache = PathEvidence::default();
            let start = Instant::now();
            for _ in 0..claims {
                let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
                cache.set_roots(keys.root_keys());
                let mut parents = HashMap::new();
                for owner in &owners {
                    assert!(
                        !keys
                            .needs_exact_comparison_with_evidence(owner, &mut |path| cache
                                .verify(path, &mut parents))
                            .unwrap()
                    );
                }
            }
            let rebuilt = start.elapsed() / claims;
            let mut cache = PathEvidence::default();
            let start = Instant::now();
            let mut prepared: Option<Vec<OwnerOverlapPrefilter>> = None;
            for _ in 0..claims {
                let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
                cache.set_roots(keys.root_keys());
                let prepared = prepared.get_or_insert_with(|| {
                    owners
                        .iter()
                        .map(|owner| OwnerOverlapPrefilter::prepare(owner, keys.root_keys()))
                        .collect()
                });
                let mut parents = HashMap::new();
                for (owner, prefilter) in owners.iter().zip(prepared.iter()) {
                    assert!(
                        !keys
                            .needs_exact_comparison_prepared(prefilter, owner, &mut |path| cache
                                .verify(path, &mut parents))
                            .unwrap()
                    );
                }
            }
            let reused = start.elapsed() / claims;

            // Whole claims through the scan context: the catalog read anew per
            // claim (no counter), and the owners held and visited linearly or
            // through the index, each after one claim that loads them.
            let (app, actor, titles) = crate::lib_tests::bootstrap_with_title_repo();
            let library_id = set_movie_roots(&app, &[&root]).await;
            let template = synthetic_movie(&app, &actor, &library_id, "Cinderwake").await;
            titles
                .store
                .lock()
                .await
                .extend(owners.iter().enumerate().map(|(index, owner)| {
                    let mut title = template.clone();
                    title.id = format!("synthetic-owner-{index:06}");
                    title.name = format!("Emberfall {index}");
                    title.folder_path = Some(owner.clone());
                    title
                }));
            let mut per_claim = Vec::new();
            for mode in ["read", "linear", "indexed"] {
                let held_owners = mode != "read";
                titles
                    .counts_folder_writes
                    .store(held_owners, Ordering::SeqCst);
                let context = ScanOwnershipContext::new();
                context
                    .oracle
                    .linear
                    .store(mode == "linear", Ordering::SeqCst);
                // Measured held at every size, whatever the default budget.
                context
                    .oracle
                    .byte_budget
                    .store(usize::MAX, Ordering::SeqCst);
                if held_owners {
                    context
                        .clone()
                        .check(&app, &template, candidate.clone())
                        .await
                        .unwrap();
                }
                let probes = context.cache.lock().unwrap().probes;
                let visits = context.owners.lock().unwrap().indexed_visits;
                let start = Instant::now();
                for _ in 0..claims {
                    context
                        .clone()
                        .check(&app, &template, candidate.clone())
                        .await
                        .unwrap();
                }
                let elapsed = start.elapsed() / claims;
                let probes = (context.cache.lock().unwrap().probes - probes) / u64::from(claims);
                let held = context.owners.lock().unwrap();
                let visits = (held.indexed_visits - visits) / u64::from(claims);
                if held_owners {
                    assert_eq!((held.reloads, held.reuses), (1, u64::from(claims)));
                }
                if mode == "indexed" {
                    let read: Vec<(String, String)> = owners
                        .iter()
                        .enumerate()
                        .map(|(index, owner)| {
                            (format!("synthetic-owner-{index:06}"), owner.clone())
                        })
                        .collect();
                    eprintln!(
                        "held owner bytes: owners={count} estimated_before_read={} estimated_held={} within_default_budget={}",
                        estimated_unprepared_bytes(&read),
                        held.list.as_ref().map_or(0, HeldList::estimated_bytes),
                        estimated_unprepared_bytes(&read) <= HELD_OWNERS_BYTE_BUDGET
                    );
                    assert_eq!(held.indexed_claims, u64::from(claims) + 1);
                    // An unrelated candidate under one flat root: once the
                    // root's evidence holds, only owners whose folder the
                    // evidence has no room to record are visited, as the
                    // linear visit probes exactly those.
                    assert_eq!(visits, count.saturating_sub(65_536) as u64);
                }
                per_claim.push((elapsed, probes));
            }
            assert_eq!(
                per_claim[1].1, per_claim[2].1,
                "the index probes what the linear visit probes"
            );
            eprintln!(
                "scan ownership claim cost (debug build): owners={count} claims={claims} compare_rebuilt_us={} compare_prepared_us={} check_read_per_claim_us={} check_held_linear_us={} check_held_indexed_us={} evidence_probes_per_claim_read={} linear={} indexed={}",
                rebuilt.as_micros(),
                reused.as_micros(),
                per_claim[0].0.as_micros(),
                per_claim[1].0.as_micros(),
                per_claim[2].0.as_micros(),
                per_claim[0].1,
                per_claim[1].1,
                per_claim[2].1
            );
        }
    }

    /// The index finds exactly the owners whose keys the linear visit finds
    /// related, across separator boundaries, trailing separators, filesystem
    /// roots and look-alike prefixes.
    #[test]
    fn scan_ownership_index_finds_every_related_key() {
        let keys = [
            "/",
            "/a",
            "/a/",
            "/a/b",
            "/a/b/",
            "/a/b/c",
            "/a/bc",
            "/ab",
            "/a-b",
            "/a b",
            "/a/b c",
            "/a//b",
            "a",
            "a/b",
            "ab",
            "c:/",
            "c:/movies",
            "c:/movies/x",
            "c:/moviesx",
            "//srv",
            "//srv/share",
            "//srv/share/t",
            "/é",
            "/é/x",
            "/éx",
            "/movies/emberfall",
            "/movies/emberfall/extras",
            "/movies/emberfall two",
            "/movies/emberfall.two",
        ];
        let mut index = OwnerIndex::default();
        for (at, key) in keys.iter().enumerate() {
            index
                .keys
                .entry((*key).to_owned())
                .or_default()
                .insert(format!("owner-{at:02}"));
        }
        let candidates = keys.iter().copied().chain([
            "",
            "/a/b/c/d",
            "/a/b/c/",
            "/x",
            "/movies",
            "/movies/",
            "/movies/emberfall/",
            "c:",
            "c:/movies/",
            "//",
            "//srv/",
            "/é/",
            "/a/b c/d",
            "/a//",
            "/a//b/c",
        ]);
        for candidate in candidates {
            let mut found = BTreeSet::new();
            index.related(candidate, |id| {
                found.insert(id.to_owned());
            });
            let expected: BTreeSet<String> = keys
                .iter()
                .enumerate()
                .filter(|(_, key)| overlap_keys_related(key, candidate))
                .map(|(at, _)| format!("owner-{at:02}"))
                .collect();
            assert_eq!(found, expected, "candidate {candidate:?}");
        }
    }

    async fn sweep(
        app: &AppUseCase,
        context: &Arc<ScanOwnershipContext>,
        claimants: &[Title],
        candidates: &[PathBuf],
    ) -> Vec<Result<(), Option<&'static str>>> {
        let mut outcomes = Vec::new();
        for claimant in claimants {
            for candidate in candidates {
                outcomes.push(reason(
                    context
                        .clone()
                        .check(app, claimant, path_to_stored_string(candidate))
                        .await,
                ));
            }
        }
        outcomes
    }

    /// Every held claim is answered through the index and, on a copy of the
    /// evidence, by the linear visit: the same decision, reason code and
    /// reported owner, the same probes, and the same evidence afterwards.
    #[tokio::test]
    async fn scan_ownership_index_answers_as_the_linear_visit() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real-library");
        let alias = temp.path().join("alias-library");
        let missing = temp.path().join("missing-library");
        let outside = temp.path().join("outside");
        let second = temp.path().join("second-library");
        for dir in [
            real.join("Emberfall").join("Extras"),
            real.join("Emberfall Two"),
            real.join("Emberfall Three"),
            real.join("cinderwake"),
            real.join("Pinecrest"),
            real.join("G").join("Gloamridge"),
            real.join("G").join("Glimmer"),
            real.join("Fernhollow"),
            real.join("Ivyreach"),
            real.join("Fresh"),
            outside.join("Ashgrove Real"),
            outside.join("G Real").join("Gloamridge"),
            second.clone(),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for filler in 0..24 {
            std::fs::create_dir_all(real.join(format!("Filler {filler:02}"))).unwrap();
        }
        let sentinel = real.join("Fresh").join("preserved.mkv");
        std::fs::write(&sentinel, b"preserved").unwrap();
        symlink(&real, &alias).unwrap();
        symlink(outside.join("Ashgrove Real"), real.join("Ashgrove")).unwrap();
        let (app, actor, titles) = crate::lib_tests::bootstrap_with_title_repo();
        let library_id = set_movie_roots(&app, &[&alias, &missing]).await;
        let template = synthetic_movie(&app, &actor, &library_id, "Hollowmere").await;
        let seed = |id: &str, folder: String| {
            let mut title = template.clone();
            title.id = id.into();
            title.name = format!("Synthetic {id}");
            title.folder_path = Some(folder);
            title
        };
        let stored = |path: &Path| path_to_stored_string(path);
        let mut seeded = vec![
            // Nested, sibling and look-alike owners under the stored root.
            seed("owner-01", stored(&alias.join("Emberfall"))),
            seed("owner-02", stored(&alias.join("Emberfall").join("Extras"))),
            seed("owner-03", stored(&alias.join("Emberfall Two"))),
            // Recorded under the resolved root, one in another case.
            seed("owner-04", stored(&real.join("cinderwake"))),
            seed("owner-05", stored(&real.join("Pinecrest"))),
            // Under an organizing folder, walked two directories deep.
            seed("owner-06", stored(&alias.join("G").join("Gloamridge"))),
            seed("owner-07", stored(&alias.join("G").join("Glimmer"))),
            // A link out of the library.
            seed("owner-08", stored(&alias.join("Ashgrove"))),
            // Compared whatever the candidate: relative, and under a root
            // that does not resolve.
            seed("owner-zz-relative", "relative/Thornvale".into()),
            seed("owner-zz-unresolved", stored(&missing.join("Duskmere"))),
        ];
        for filler in 0..24 {
            seeded.push(seed(
                &format!("owner-1{filler:02}"),
                stored(&alias.join(format!("Filler {filler:02}"))),
            ));
        }
        titles.store.lock().await.extend(seeded.iter().cloned());
        titles.counts_folder_writes.store(true, Ordering::SeqCst);
        let context = ScanOwnershipContext::new();
        context.oracle.compare.store(true, Ordering::SeqCst);
        let claimants = [template.clone(), seeded[4].clone(), seeded[1].clone()];
        let candidates: Vec<PathBuf> = vec![
            alias.join("Emberfall"),
            alias.join("Emberfall").join("Extras").join("Deep"),
            real.join("Emberfall"),
            alias.join("CINDERWAKE"),
            alias.join("Emberfall Two").join("Bonus"),
            alias.join("G"),
            alias.join("G").join("Gloamridge"),
            alias.join("Fresh"),
            alias.join("Pinecrest"),
            alias.clone(),
            missing.join("Duskmere"),
            alias.join("Ashgrove"),
            outside.join("Ashgrove Real"),
            outside.join("G Real"),
            alias.join("Filler 07"),
            alias.join("Filler 07").join("Sub"),
            alias.join("Fernhollow"),
            alias.join("Ivyreach"),
        ];
        let mut outcomes = sweep(&app, &context, &claimants, &candidates).await;

        // A reserved folder joins the visit by title id.
        let reserved = context
            .clone()
            .check_and_reserve(&app, &template, stored(&alias.join("Fernhollow")))
            .await;
        outcomes.extend(sweep(&app, &context, &claimants[1..], &candidates).await);
        drop(reserved);

        // Another writer deletes the owners every claim compares.
        titles.delete("owner-zz-relative").await.unwrap();
        titles.delete("owner-zz-unresolved").await.unwrap();
        outcomes.extend(sweep(&app, &context, &claimants, &candidates).await);

        // The scan's own claim, then claims against it.
        let mut fernhollow = titles
            .create({
                let mut title = template.clone();
                title.id = "owner-30".into();
                title.name = "Fernhollow".into();
                title
            })
            .await
            .unwrap();
        let (reloads, _) = owner_stats(&context);
        with_scan_ownership(
            Some(context.clone()),
            claim_title_folder_for_scan_if_missing(
                &app,
                &mut fernhollow,
                &alias.join("Fernhollow"),
            ),
        )
        .await
        .unwrap();
        outcomes.extend(sweep(&app, &context, &claimants, &candidates).await);
        assert_eq!(
            owner_stats(&context).0,
            reloads,
            "an own claim is applied in place"
        );

        // Another writer adds an owner and moves one.
        titles
            .create(seed("owner-12", stored(&alias.join("Ivyreach"))))
            .await
            .unwrap();
        titles
            .assign_title_folders(&[crate::TitleFolderWrite::assign(
                "owner-03",
                stored(&alias.join("Emberfall Three")),
            )
            .expecting(seeded[2].folder_path.clone())])
            .await
            .unwrap();
        outcomes.extend(sweep(&app, &context, &claimants, &candidates).await);

        // Mid-scan, an organizing folder and an owner's folder become links.
        std::fs::rename(real.join("G"), outside.join("G Saved")).unwrap();
        symlink(outside.join("G Real"), real.join("G")).unwrap();
        outcomes.extend(sweep(&app, &context, &claimants, &candidates).await);
        std::fs::rename(real.join("Pinecrest"), outside.join("Pinecrest Saved")).unwrap();
        symlink(outside.join("Ashgrove Real"), real.join("Pinecrest")).unwrap();
        outcomes.extend(sweep(&app, &context, &claimants, &candidates).await);

        // The library's roots change.
        set_movie_roots(&app, &[&second]).await;
        outcomes.extend(sweep(&app, &context, &claimants, &candidates).await);

        let mismatches = context.oracle.mismatches.lock().unwrap().clone();
        assert!(mismatches.is_empty(), "{mismatches:#?}");
        let held = context.owners.lock().unwrap();
        let compared = context.oracle.compared.load(Ordering::SeqCst);
        assert_eq!(held.indexed_claims, compared);
        assert_eq!(compared, outcomes.len() as u64 + 2);
        assert!(
            context.oracle.rests.load(Ordering::SeqCst) > 0,
            "a link swap removes evidence mid-claim"
        );
        assert!(outcomes.iter().any(Result::is_ok));
        assert!(outcomes.contains(&Err(Some(
            crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE
        ))));
        assert!(
            held.indexed_visits < compared * seeded.len() as u64 / 2,
            "visits={} claims={compared}",
            held.indexed_visits
        );
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserved");
    }
}
