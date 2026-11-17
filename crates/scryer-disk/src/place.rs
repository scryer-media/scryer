//! Placing one file at a destination.
//!
//! The caller says what it wants: the method, whether an existing destination
//! may be replaced, and whether a copy is an acceptable way to get there. The
//! owner picks how: which rename the filesystem offers, the staging name a
//! copy is written under, the proof a copy must pass before its source goes,
//! and what is put back when a step fails. The answer says what was done
//! against what was asked, or why nothing was.
//!
//! `rename(2)` replaces its destination silently, so a move that does not
//! claim the destination can destroy a file nobody asked it to touch. Every
//! path here that may not replace either refuses in the same call that would
//! have written, or reserves the name first.

use std::cell::{Cell, RefCell};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::copy::{
    Cause, Copied, CopyPermit, Evidence, Landing, Opening, Phase, Step, Watch, WriteFail, Writing,
    resumable_staging_path, staged_name_is_free, write_copy,
};
use super::mark::{Mark, UNKNOWN_LANE};
use super::sample::{ContentSample, sample_reader};
use super::{
    Disk, DiskError, DiskVerb, EntryKind, Link, LinkOutcome, Offloaded, Open, Parents, PathReading,
    Removal, Remove, Replace, SyncOutcome, offload, reason, staging_path_for, stored_paths,
};

/// How long to wait before looking again at a copy a network filesystem may
/// not have settled yet.
const COPY_VERIFY_RETRY_DELAY: Duration = Duration::from_secs(3);

/// How the caller wants the file to reach its destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// A second name for the same file. The source keeps its name.
    HardLink,
    /// A second file with the same contents. The source is left alone.
    Copy,
    /// The file leaves its source name.
    Move,
}

/// What may be done when the method itself cannot be honoured: a link the
/// destination cannot hold, or a move a rename cannot make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// Refuse and change nothing.
    None,
    /// Copy instead. A move then removes its source once the copy is in
    /// place.
    Copy,
}

/// The proof a copy must pass before it counts, and before a move removes its
/// source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verify {
    /// The caller proves the copy for itself. A copy the filesystem could
    /// not sync is still proven here, and so is the copy a move falls back
    /// to: its source goes next.
    None,
    /// The copy's size and sampled contents must match the source's, before
    /// it takes the destination's name. One that does not match is removed.
    Quick,
    /// The copy takes its place and is then proven, and the answer carries
    /// what it streamed and what it proved to be. A copy that does not prove
    /// stays as written. A recorded copy never removes a source: a move that
    /// needs one is refused, for the caller to copy and decide on the proof.
    Recorded(Evidence),
}

/// One placement's policy. The application's resolver builds it; there is no
/// default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub method: Method,
    pub fallback: Fallback,
    /// Whether a file already at the destination may be replaced. Replacing
    /// is a deliberate act, never a fallback.
    pub replace: Replace,
    pub verify: Verify,
    /// Whether the destination's missing folders are created first. A hard
    /// link never creates them.
    pub parents: Parents,
}

/// What was done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    HardLinked,
    Copied,
    Renamed,
    CopiedThenRemovedSource,
    /// The source was a symbolic link on another volume: a link to the same
    /// target was made at the destination and the source link removed.
    RelinkedThenRemovedSource,
}

/// A finished placement: what was asked, and what was done to honour it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub asked: Method,
    pub done: Done,
    /// What a recorded copy streamed and proved. Absent when no bytes were
    /// copied, or the policy records nothing.
    pub copied: Option<Copied>,
}

impl Placed {
    /// Whether the fallback was taken instead of the method asked for: a
    /// copy, or a symbolic link made again on the other volume.
    pub fn fell_back(&self) -> bool {
        !matches!(
            (self.asked, self.done),
            (Method::HardLink, Done::HardLinked)
                | (Method::Copy, Done::Copied)
                | (Method::Move, Done::Renamed)
        )
    }
}

/// What a move onto a free name did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeNameMove {
    Moved,
    /// Nothing was moved: the volume has no rename that refuses an occupied
    /// name, or on Windows it refused for a reason that is neither a
    /// collision nor a move to another volume. The caller takes the path it
    /// took before the rename existed.
    NotOffered,
}

/// Why the destination cannot hold a hard link to the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CannotLink {
    /// The two paths are on different volumes.
    DifferentVolume,
    /// The filesystem has no hard links, or no room for another.
    NotSupported,
    /// The link was made, but its new name did not prove to be the file the
    /// caller saw, and the link was removed again. Only a placement that
    /// asks for the proof answers this.
    Unverifiable,
}

/// How far a new hard link is proven before it counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkProof {
    /// The new name is read once, itself, and must be the file the caller
    /// saw.
    Name,
    /// And what the new name resolves to is read as well, and must have the
    /// length the caller saw.
    NameAndLength,
}

/// Why a hard link that was made did not prove.
#[derive(Debug)]
pub enum Doubt {
    /// The new name could not be read.
    Unread(DiskError),
    /// The new name read as something other than the file the caller saw.
    /// Filesystems that number files by path report this for a true link.
    OtherFile { linked: Mark },
    /// What the new name resolves to has another length, or could not be
    /// read; the reading says which.
    OtherLength(PathReading),
}

/// What became of the name an unproven hard link was made under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkAfter {
    /// The placement removed the name it had created.
    Removed,
    /// The removal found nothing at the name: something else took it away.
    AlreadyGone,
    /// The removal failed, and that failure is the refusal. The name may
    /// still be there.
    NotRemoved,
    /// The name was not touched. It read as something other than a regular
    /// file, even if this call made it: a source swapped for a symlink
    /// before the link call makes exactly that. Or it read as a regular file
    /// that is not the link and whose length or modification time is not the
    /// source's, which is another writer's file.
    LeftAlone,
}

/// A hard link this placement made and could not prove. The placement then
/// went to remove the name it had just created, and nothing else; a name
/// that does not read as a regular file, or reads as a regular file with
/// another length or modification time than the source, is left alone.
#[derive(Debug)]
pub struct Unproven {
    pub doubt: Doubt,
    pub after: LinkAfter,
}

/// A file a failed placement left away from where it started: under a name
/// it had reserved, where it may be the source moved by a rename that
/// reported failure, or the staged file of a move. Each variant names where
/// the file is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Leftover {
    /// Put back under the source's name, which is `at`.
    PutBack { at: PathBuf },
    /// Left at the reserved or staged name `at`, because nothing read says
    /// it may be put back or removed: the source, or the name itself, could
    /// not be read, or the file is a copy that was never proven.
    AtReservedName { at: PathBuf },
    /// Putting it back under the source's name failed, so it is still at the
    /// reserved or staged name `at`.
    PutBackFailed { at: PathBuf },
}

/// Which path a refusal is about, where the disk's answer says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Source,
    Destination,
    /// The call named both paths and its failure does not say which.
    Either,
}

/// Why nothing was placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The destination holds something, and the policy does not replace it.
    DestinationExists,
    CannotLink(CannotLink),
    /// A rename cannot make the move, and the policy allows no copy.
    NeedsCopy,
    /// The copy did not match its source. The source was kept.
    NotVerified,
    /// The name a staged recorded copy writes under is held by something.
    /// Nothing was copied.
    StagingOccupied,
    /// Whoever watches the copy stopped it.
    Cancelled,
    NoSpace,
    /// Permission was refused, on the side the failed call named.
    Denied(Side),
    /// The source is not there. A rename names both paths in one call and
    /// does not say which is missing, so from a rename this is also the
    /// answer when the destination's folder is not there.
    SourceMissing,
    /// The destination's folder is not there and the policy creates none; or
    /// a destination the policy says the caller holds is not there; or the
    /// name of a hard link that did not prove was already gone when the
    /// placement went to remove it.
    DestinationMissing,
    Unreachable(Side),
    /// The name is a link the placement does not go through: a name the
    /// caller holds is written only when it is the file itself.
    LinkNotFollowed,
    /// Any other failure.
    Failed,
}

impl Refused {
    /// The stable [`reason`] code for this refusal.
    pub fn reason(self) -> &'static str {
        match self {
            Self::DestinationExists => reason::DISK_DESTINATION_EXISTS,
            Self::CannotLink(CannotLink::DifferentVolume) => reason::DISK_LINK_DIFFERENT_VOLUME,
            Self::CannotLink(CannotLink::NotSupported) => reason::DISK_LINK_NOT_SUPPORTED,
            Self::CannotLink(CannotLink::Unverifiable) => reason::DISK_LINK_UNVERIFIABLE,
            Self::NeedsCopy => reason::DISK_MOVE_NEEDS_COPY,
            Self::NotVerified => reason::DISK_COPY_NOT_VERIFIED,
            Self::StagingOccupied => reason::DISK_COPY_STAGING_OCCUPIED,
            Self::Cancelled => reason::DISK_COPY_CANCELLED,
            Self::NoSpace => reason::DISK_NO_SPACE,
            Self::Denied(_) => reason::DISK_PERMISSION_DENIED,
            Self::SourceMissing | Self::DestinationMissing => reason::DISK_PATH_MISSING,
            Self::Unreachable(_) => reason::DISK_PATH_UNREACHABLE,
            Self::LinkNotFollowed => reason::DISK_LINK_NOT_FOLLOWED,
            Self::Failed => reason::DISK_OPERATION_FAILED,
        }
    }
}

/// A placement that was refused: what was asked and why it was not done. It
/// displays the system's own words for the failure, for a person to read.
#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub struct PlaceRefusal {
    pub asked: Method,
    pub refused: Refused,
    /// The link this placement made, could not prove and went to remove,
    /// and whether the name is gone.
    pub unproven: Option<Unproven>,
    /// Whether this placement changed what is under the destination name
    /// itself before it failed: a copy that lands directly, stopped after it
    /// emptied the file the caller holds there or created one. `false` for
    /// every failure before that (the source not opened, a turn not given,
    /// the destination not opened or not emptied), for a copy that lands
    /// through a staged name, and for a placement whose thread ended without
    /// answering, where nothing says what it did.
    pub destination_written: bool,
    /// Each file this placement left away from where it started, and where.
    /// Empty when it left nothing so.
    pub left: Vec<Leftover>,
    detail: io::Error,
}

/// A failed step, classified where it failed.
pub(crate) struct Fail {
    pub(crate) refused: Refused,
    pub(crate) detail: io::Error,
}

impl Fail {
    pub(crate) fn of(error: DiskError, side: Side) -> Self {
        let refused = match error.reason() {
            reason::DISK_DESTINATION_EXISTS | reason::DISK_DIRECTORY_NOT_EMPTY => {
                Refused::DestinationExists
            }
            reason::DISK_NO_SPACE => Refused::NoSpace,
            reason::DISK_PERMISSION_DENIED => Refused::Denied(side),
            reason::DISK_PATH_MISSING if side == Side::Destination => Refused::DestinationMissing,
            reason::DISK_PATH_MISSING => Refused::SourceMissing,
            reason::DISK_PATH_UNREACHABLE | reason::DISK_TIMED_OUT => Refused::Unreachable(side),
            reason::DISK_LINK_NOT_FOLLOWED => Refused::LinkNotFollowed,
            _ => Refused::Failed,
        };
        Self {
            refused,
            detail: error.into(),
        }
    }

    /// A failure of reading or writing through an open handle.
    fn io(error: io::Error, path: &Path, side: Side) -> Self {
        Self::of(DiskError::new(DiskVerb::Write, path, error), side)
    }

    /// A path whose reading gave no answer, refused by what stopped the
    /// reading: only a path that timed out or could not be reached is
    /// unreachable.
    pub(crate) fn unread(reading: PathReading, side: Side, detail: String) -> Self {
        let refused = match reading.unreachable_cause() {
            Some(reason::DISK_PERMISSION_DENIED) => Refused::Denied(side),
            Some(reason::DISK_PATH_UNREACHABLE | reason::DISK_TIMED_OUT) => {
                Refused::Unreachable(side)
            }
            _ => Refused::Failed,
        };
        Self::new(refused, io::ErrorKind::Other, detail)
    }

    pub(crate) fn new(refused: Refused, kind: io::ErrorKind, detail: String) -> Self {
        Self {
            refused,
            detail: io::Error::new(kind, detail),
        }
    }

    /// The same refusal, saying also what the failed step left behind.
    fn noting(self, left: &str) -> Self {
        Self {
            refused: self.refused,
            detail: io::Error::new(self.detail.kind(), format!("{}; {left}", self.detail)),
        }
    }
}

/// What a refusal says of an empty reserved name that was not released.
fn left_as_the_source(name: &Path, source: &Path, source_reads: PathReading) -> String {
    format!(
        "the empty file at {} may be the source and is left in place ({}: {})",
        name.display(),
        source.display(),
        source_reads.describe_absence()
    )
}

/// What a placement's source and destination folder read as, within their
/// roots' bounds, before it took a thread.
#[derive(Clone)]
struct Answered {
    source: PathReading,
    /// The destination's folder, links followed, read as a folder.
    folder_is_dir: bool,
    /// The copy lane of the volume that folder is on, where its read named
    /// one.
    folder_lane: Option<String>,
}

/// A placement's first reads, of the source's name and the destination's
/// folder, taken once for placements that share both. See
/// [`Disk::read_before_placing`].
#[derive(Clone)]
pub struct ReadBefore {
    answered: Answered,
    /// The source whose name was read.
    source: PathBuf,
    folder: Option<PathBuf>,
}

impl ReadBefore {
    /// The read of a caller that has just read `source`'s name within its
    /// root's bound, as `source_read`, and made `dest`'s folder itself.
    pub fn made_folder(source: &Path, source_read: PathReading, dest: &Path) -> Self {
        Self {
            answered: Answered {
                source: source_read,
                folder_is_dir: dest
                    .parent()
                    .is_some_and(|folder| !folder.as_os_str().is_empty()),
                folder_lane: None,
            },
            source: source.to_path_buf(),
            folder: dest.parent().map(Path::to_path_buf),
        }
    }
}

/// A placement refused before its job ran, so with nothing written.
fn refused_before_it_ran(policy: Placement, fail: Fail) -> PlaceRefusal {
    PlaceRefusal {
        asked: policy.method,
        refused: fail.refused,
        unproven: None,
        destination_written: false,
        left: Vec::new(),
        detail: fail.detail,
    }
}

fn needs_copy(error: &DiskError) -> bool {
    error.reason() == reason::DISK_MOVE_NEEDS_COPY
}

/// What the staging name holds when a move through it fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StagedHolds {
    /// Only the empty claim: nothing of the source reached it.
    EmptyClaim,
    /// A copy of a source that may still exist, `proven` once it was written
    /// and verified in full.
    Copy { proven: bool },
    /// The source itself, renamed there.
    OnlyCopy,
}

/// Whether a staging or intermediate name still holds only the empty file
/// claimed for it.
fn holds_only_the_claim(reading: PathReading) -> bool {
    reading.is(EntryKind::File) && reading.size() == Some(0)
}

/// True when two paths differ only in case or Unicode form, which is how a
/// case-only rename looks on a case-insensitive volume.
///
/// Deliberately compares paths rather than file identities: CIFS without
/// `serverino`, mergerfs and rclone all synthesize them, and a collision
/// there would be misread as a case-only rename and overwrite the
/// destination. Sonarr compares paths for the same reason.
pub(crate) fn paths_are_same_file(source: &Path, dest: &Path) -> bool {
    let source_key = stored_paths::path_to_stored_string(source);
    let dest_key = stored_paths::path_to_stored_string(dest);
    stored_paths::paths_match_ignoring_case(&source_key, &dest_key)
}

impl Disk {
    /// Place `source` at `dest` as `policy` says.
    ///
    /// A move renames where it can: exclusively where the filesystem offers
    /// that, by reserving a name beside the destination otherwise. A move a
    /// rename cannot make copies through that reserved name, proves the copy,
    /// puts it under the real name without replacing, and only then removes
    /// the source. A symlink is re-created rather than having the file it
    /// points at copied. When a step fails, the file is put back or left
    /// where it is; a stray file is always preferred to a lost one.
    ///
    /// Never call it on a runtime worker thread; see
    /// `Disk::place_watched_blocking`.
    pub fn place_blocking(
        &self,
        source: &Path,
        dest: &Path,
        policy: Placement,
    ) -> Result<Placed, PlaceRefusal> {
        self.place_watched_blocking(source, dest, policy, &Watch::nobody())
    }

    /// [`Self::place_blocking`], reporting a copy's progress to `watch` and
    /// stopping it when `watch` says so. A cancelled copy removes only a
    /// staged file this call created. A recorded copy waits on this thread
    /// for its turn on the destination's volume, and that wait does not
    /// observe `watch`'s cancel. Never call it on a runtime worker thread:
    /// it would hold the worker for the wait and the copy.
    pub(crate) fn place_watched_blocking(
        &self,
        source: &Path,
        dest: &Path,
        policy: Placement,
        watch: &Watch,
    ) -> Result<Placed, PlaceRefusal> {
        self.place_admitted(source, dest, policy, watch, None, false, None, None)
    }

    /// [`Self::place_blocking`] for a hard link that must prove itself.
    ///
    /// Once the link is made its new name is read, itself, and must be the
    /// file `seen` was read as before the link: the same device and file
    /// number where the platform gives them, a zero number compared as
    /// given, and the same length and modification time where it gives
    /// none. [`LinkProof::NameAndLength`] reads what the name resolves to as
    /// well. `seen` must come from a read made for this call: a stored mark
    /// or one that names a volume only never proves.
    ///
    /// A link that does not prove is removed: only the name this call
    /// created, and only when the link call itself succeeded. A name that
    /// was there before the call is refused as a destination that exists and
    /// is never removed. Neither is a new name that reads as anything but a
    /// regular file, nor one that reads as a regular file that is not the
    /// link and has another length or modification time than `seen`: another
    /// writer replaced the link, the name is left as it is with
    /// [`LinkAfter::LeftAlone`], and the answer is a destination that exists,
    /// with no fallback copy over it. A removal that fails, or finds the
    /// name already gone, is the refusal, and [`PlaceRefusal::unproven`]
    /// says what did not prove and what became of the name. Once the name is
    /// removed the policy's fallback copy runs, or the answer is
    /// [`CannotLink::Unverifiable`].
    ///
    /// What remains: a regular file another writer put at the name in that
    /// instant, with the source's length and modification time, is removed
    /// as the unproven link would be, on any filesystem. Where file numbers
    /// are made up it cannot even be told from the link. So is any regular
    /// file there when `seen` names a volume only, which carries no length
    /// or time to tell it by.
    ///
    /// Nothing is remembered about the volume. A policy that asks for
    /// another method is placed as [`Self::place_blocking`] places it.
    pub fn place_proven_blocking(
        &self,
        source: &Path,
        dest: &Path,
        policy: Placement,
        seen: &Mark,
        proof: LinkProof,
    ) -> Result<Placed, PlaceRefusal> {
        let prove = Some((seen, proof));
        self.place_admitted(
            source,
            dest,
            policy,
            &Watch::nobody(),
            None,
            false,
            prove,
            None,
        )
    }

    /// [`Self::place_proven_blocking`] off the async runtime. Once started
    /// it runs to its end even if the caller stops waiting.
    pub async fn place_proven(
        &self,
        source: impl Into<PathBuf>,
        dest: impl Into<PathBuf>,
        policy: Placement,
        seen: Mark,
        proof: LinkProof,
    ) -> Result<Placed, PlaceRefusal> {
        let (disk, source, dest) = (self.clone(), source.into(), dest.into());
        let answered = match self.roots_answer(&source, &dest).await {
            Ok(answered) => Some(answered),
            Err(fail) => return Err(refused_before_it_ran(policy, fail)),
        };
        let job = move || {
            let prove = Some((&seen, proof));
            let nobody = Watch::nobody();
            disk.place_admitted(
                &source, &dest, policy, &nobody, None, false, prove, answered,
            )
        };
        match offload(None, job).await {
            Offloaded::Done(result) => result,
            Offloaded::TimedOut | Offloaded::Failed(_) => Err(PlaceRefusal {
                asked: policy.method,
                refused: Refused::Failed,
                unproven: None,
                destination_written: false,
                left: Vec::new(),
                detail: io::Error::other("the placement did not run to its end"),
            }),
        }
    }

    /// Reads the source's name and the destination's folder, each within its
    /// root's bound, before a placement takes a thread: a root that does not
    /// answer refuses the placement here instead of holding the thread. What
    /// either holds is the placement's to judge; only a read that did not
    /// come back refuses. Returns what both read as, for the placement to
    /// use in place of its own looks at the same things.
    async fn roots_answer(&self, source: &Path, dest: &Path) -> Result<Answered, Fail> {
        let unanswered = |path: &Path, side, reading: PathReading| {
            let why = reading.describe_absence();
            Fail::unread(reading, side, format!("{}: {why}", path.display()))
        };
        let name = |disk: &Self, path: &Path| Ok(disk.uncached_blocking(path));
        let source_read = self
            .probe_with(source.to_path_buf(), name, Err)
            .await
            .map_err(|reading| unanswered(source, Side::Source, reading))?;
        let (mut folder_is_dir, mut folder_lane) = (false, None);
        if let Some(folder) = dest
            .parent()
            .filter(|folder| !folder.as_os_str().is_empty())
        {
            let resolved = |disk: &Self, path: &Path| Ok(disk.fingerprint_followed_blocking(path));
            let folder_read = self
                .probe_with(folder.to_path_buf(), resolved, Err)
                .await
                .map_err(|reading| unanswered(folder, Side::Destination, reading))?;
            if let Ok(folder) = folder_read
                && folder.kind() == Some(EntryKind::Directory)
            {
                folder_is_dir = true;
                folder_lane = folder.names_volume().then(|| folder.volume().lane());
            }
        }
        Ok(Answered {
            source: source_read,
            folder_is_dir,
            folder_lane,
        })
    }

    fn place_admitted(
        &self,
        source: &Path,
        dest: &Path,
        policy: Placement,
        watch: &Watch,
        permit: Option<CopyPermit>,
        staged_free: bool,
        prove: Option<(&Mark, LinkProof)>,
        answered: Option<Answered>,
    ) -> Result<Placed, PlaceRefusal> {
        let job = Job {
            disk: self,
            source,
            dest,
            policy,
            watch,
            permit: RefCell::new(permit),
            staged_free,
            copied: RefCell::new(None),
            prove,
            unproven: RefCell::new(None),
            left: RefCell::new(Vec::new()),
            source_read: Cell::new(answered.as_ref().map(|answered| answered.source)),
            folder_is_dir: answered.is_some_and(|answered| answered.folder_is_dir),
            wrote_dest: Cell::new(false),
        };
        match job.run() {
            Ok(done) => Ok(Placed {
                asked: policy.method,
                done,
                copied: job.copied.take(),
            }),
            Err(fail) => Err(PlaceRefusal {
                asked: policy.method,
                refused: fail.refused,
                unproven: job.unproven.take(),
                destination_written: job.wrote_dest.get(),
                left: job.left.take(),
                detail: fail.detail,
            }),
        }
    }

    /// [`Self::place_blocking`] off the async runtime. Once started it runs
    /// to its end even if the caller stops waiting, so no step is abandoned
    /// half way.
    pub async fn place(
        &self,
        source: impl Into<PathBuf>,
        dest: impl Into<PathBuf>,
        policy: Placement,
    ) -> Result<Placed, PlaceRefusal> {
        self.place_watched(source, dest, policy, Watch::nobody())
            .await
    }

    /// `Disk::place_watched_blocking` off the async runtime.
    ///
    /// The source's name and the destination's folder are first read within
    /// their roots' bounds, and a placement whose root does not answer is
    /// refused before it takes a thread. The copy that lands directly is the
    /// exception, below.
    ///
    /// A recorded copy that lands [`Landing::Staged`] first reads its staged
    /// name and its destination's volume within the root's bound, and is
    /// refused before it takes a thread when either does not answer. It then
    /// waits for its turn on that volume and only then takes a thread.
    ///
    /// A recorded copy that lands [`Landing::Direct`] is never refused
    /// before it runs: its destination can be a name the caller holds, and a
    /// caller that cleans up a refused copy would remove a file nothing
    /// wrote to. Its volume is read off the runtime with no bound, and a
    /// volume that never answers there leaves the copy waiting. It then
    /// waits for its turn without holding a thread, and takes one only for
    /// the copy.
    ///
    /// A link that falls back to a recorded copy is already on its thread
    /// and waits there. Every way the turn is given back before the copy is
    /// proven. `watch`'s cancel is not observed while the volume is read or
    /// the turn is waited for.
    pub async fn place_watched(
        &self,
        source: impl Into<PathBuf>,
        dest: impl Into<PathBuf>,
        policy: Placement,
        watch: Watch,
    ) -> Result<Placed, PlaceRefusal> {
        self.place_answered(source.into(), dest.into(), policy, watch, None)
            .await
    }

    /// Reads `source`'s name and `dest`'s folder within their roots' bounds,
    /// as [`Self::place`] does first, for a caller that places from one
    /// source into one folder more than once: see
    /// [`Self::place_after_reading`]. A root that does not answer is the
    /// refusal a placement would give.
    pub async fn read_before_placing(
        &self,
        source: &Path,
        dest: &Path,
        policy: Placement,
    ) -> Result<ReadBefore, PlaceRefusal> {
        let answered = self.roots_answer(source, dest).await;
        let answered = answered.map_err(|fail| refused_before_it_ran(policy, fail))?;
        Ok(ReadBefore {
            answered,
            source: source.to_path_buf(),
            folder: dest.parent().map(Path::to_path_buf),
        })
    }

    /// [`Self::place`] with the source's name and the destination's folder
    /// taken from `read` instead of read again. A `source` other than the one
    /// `read` was taken for, or a `dest` in another folder, is read as
    /// [`Self::place`] reads it.
    pub async fn place_after_reading(
        &self,
        source: impl Into<PathBuf>,
        dest: impl Into<PathBuf>,
        policy: Placement,
        read: &ReadBefore,
    ) -> Result<Placed, PlaceRefusal> {
        let (source, dest) = (source.into(), dest.into());
        let same = read.source == source && read.folder.as_deref() == dest.parent();
        let answered = same.then(|| read.answered.clone());
        let watch = Watch::nobody();
        self.place_answered(source, dest, policy, watch, answered)
            .await
    }

    async fn place_answered(
        &self,
        source: PathBuf,
        dest: PathBuf,
        policy: Placement,
        watch: Watch,
        read_already: Option<Answered>,
    ) -> Result<Placed, PlaceRefusal> {
        let disk = self.clone();
        let recorded = matches!(policy.verify, Verify::Recorded(_));
        let mut staged_free = false;
        let refuse = |fail: Fail| refused_before_it_ran(policy, fail);
        let lands_directly = matches!(
            policy,
            Placement {
                method: Method::Copy,
                verify: Verify::Recorded(Evidence {
                    landing: Landing::Direct,
                    ..
                }),
                ..
            }
        );
        // Every placement but the copy that lands directly is refused here
        // when its source's root or its destination's does not answer.
        let answered = match (lands_directly, read_already) {
            (true, _) => None,
            (false, Some(answered)) => Some(answered),
            (false, None) => Some(self.roots_answer(&source, &dest).await.map_err(refuse)?),
        };
        // The folder's volume, where its read named one, is the volume the
        // copy lands on: no second read of it for the turn.
        let landing = answered
            .as_ref()
            .filter(|answered| answered.folder_is_dir)
            .and_then(|answered| answered.folder_lane.clone());
        let permit = match policy {
            Placement {
                method: Method::Copy,
                verify:
                    Verify::Recorded(Evidence {
                        landing: Landing::Staged,
                        ..
                    }),
                ..
            } => {
                // The staged name is read within the root's bound, before a
                // turn is waited for and before a thread is taken.
                let staged = resumable_staging_path(&dest);
                let reading = self.uncached(staged.clone()).await;
                staged_name_is_free(&staged, reading).map_err(refuse)?;
                staged_free = true;
                watch.progress.report(Phase::Waiting, 0);
                match landing {
                    Some(lane) => Some(self.copy_admission().acquire_keyed(&lane).await),
                    None => Some(self.copy_turn(&dest).await.map_err(refuse)?),
                }
            }
            Placement {
                method: Method::Copy,
                verify:
                    Verify::Recorded(Evidence {
                        landing: Landing::Direct,
                        ..
                    }),
                ..
            } => {
                watch.progress.report(Phase::Waiting, 0);
                let (reader, landing) = (self.clone(), dest.clone());
                let lane = match offload(None, move || reader.copy_lane(&landing)).await {
                    Offloaded::Done(lane) => lane,
                    // A read that did not come back names no volume: the
                    // copy shares the unknown lane, and still waits here.
                    Offloaded::TimedOut | Offloaded::Failed(_) => UNKNOWN_LANE.to_owned(),
                };
                Some(self.copy_admission().acquire_keyed(&lane).await)
            }
            _ => None,
        };
        let job = move || {
            let (free, read) = (staged_free, answered);
            disk.place_admitted(&source, &dest, policy, &watch, permit, free, None, read)
        };
        match offload(None, job).await {
            Offloaded::Done(result) => result,
            Offloaded::TimedOut => Err(PlaceRefusal {
                asked: policy.method,
                refused: Refused::Failed,
                unproven: None,
                destination_written: false,
                left: Vec::new(),
                detail: io::Error::other("the placement did not run to its end"),
            }),
            Offloaded::Failed(said) => Err(PlaceRefusal {
                asked: policy.method,
                refused: Refused::Failed,
                unproven: None,
                destination_written: false,
                left: Vec::new(),
                detail: io::Error::other(match recorded {
                    true => format!("verified copy task failed to join: {said}"),
                    false => "the placement did not run to its end".to_string(),
                }),
            }),
        }
    }

    /// Whether `dest` is available to receive `source`: nothing reads as
    /// present there, or it is the source under another spelling of its name,
    /// which a move renames rather than refuses.
    pub async fn destination_is_free_for(&self, source: &Path, dest: &Path) -> bool {
        self.uncached(dest).await.exists() != Some(true) || paths_are_same_file(source, dest)
    }

    /// The exclusive rename, or `None` where the caller should reserve the
    /// destination instead: the filesystem has none, or on Windows it refused
    /// for a reason that is neither a collision nor a move to another volume,
    /// where the reservation worked before Windows had this rename.
    pub(crate) fn exclusive_rename_blocking(
        &self,
        source: &Path,
        dest: &Path,
    ) -> Option<Result<(), DiskError>> {
        match self.rename_no_replace_blocking(source, dest) {
            Ok(true) => Some(Ok(())),
            Ok(false) => None,
            Err(error)
                if cfg!(windows)
                    && !matches!(
                        error.reason(),
                        reason::DISK_DESTINATION_EXISTS
                            | reason::DISK_DIRECTORY_NOT_EMPTY
                            | reason::DISK_MOVE_NEEDS_COPY
                    ) =>
            {
                None
            }
            Err(error) => Some(Err(error)),
        }
    }

    /// Moves a staged or reserved file or folder onto a name nothing holds,
    /// in one step that never replaces. A refusal changed nothing; its
    /// reason says whether the name was taken or the move needs a copy.
    pub async fn move_onto_free_name(
        &self,
        source: impl Into<PathBuf>,
        dest: impl Into<PathBuf>,
    ) -> Result<FreeNameMove, DiskError> {
        let dest = dest.into();
        self.run(
            DiskVerb::Move,
            source.into(),
            None,
            move |disk, source| match disk.exclusive_rename_blocking(source, &dest) {
                Some(result) => result.map(|()| FreeNameMove::Moved),
                None => Ok(FreeNameMove::NotOffered),
            },
        )
        .await
    }

    /// Move a finished file from its staging name onto its destination,
    /// never replacing anything there.
    ///
    /// The destination was never reserved, so this is the moment another
    /// writer could have taken it; the exclusive rename is tried first for
    /// that reason. Where there is none the staged file is linked into
    /// place, which refuses an existing destination in the same call, and
    /// only a volume with neither reserves the name and renames over the
    /// reservation.
    pub(crate) fn promote_staged_blocking(
        &self,
        staged: &Path,
        dest: &Path,
    ) -> Result<(), DiskError> {
        if let Some(result) = self.exclusive_rename_blocking(staged, dest) {
            return result;
        }
        match self.link_blocking(staged, dest, Link::Hard) {
            Ok(LinkOutcome::Linked) => {
                // The destination now holds the file. Failing to unlink the
                // staging name leaves a stray link, not a lost file.
                if !matches!(
                    self.remove_blocking(staged, Remove::File),
                    Ok(Removal::Removed)
                ) {
                    tracing::warn!(
                        staged = %staged.display(),
                        dest = %dest.display(),
                        "moved the file but could not remove its staging link"
                    );
                }
                return Ok(());
            }
            Err(error) if error.reason() == reason::DISK_DESTINATION_EXISTS => return Err(error),
            Ok(LinkOutcome::NotPossible) | Err(_) => {}
        }
        // The empty reservation exists only for the length of a rename.
        self.move_claimed_blocking(staged, dest).map(|_| ())
    }
}

pub(crate) struct Job<'a> {
    pub(crate) disk: &'a Disk,
    pub(crate) source: &'a Path,
    pub(crate) dest: &'a Path,
    pub(crate) policy: Placement,
    pub(crate) watch: &'a Watch,
    /// The copy's turn on the destination's volume, when it waited for one.
    pub(crate) permit: RefCell<Option<CopyPermit>>,
    /// Whether the staged name of a recorded copy was already read as free.
    pub(crate) staged_free: bool,
    pub(crate) copied: RefCell<Option<Copied>>,
    /// What a hard link must prove to be once it is made; `None` for a link
    /// the caller proves, or does not.
    pub(crate) prove: Option<(&'a Mark, LinkProof)>,
    /// The link this job made and could not prove, when that is why it
    /// failed.
    pub(crate) unproven: RefCell<Option<Unproven>>,
    /// What [`PlaceRefusal::left`] reports.
    pub(crate) left: RefCell<Vec<Leftover>>,
    /// The source's name as it was read, within its root's bound, before
    /// this job took its thread; `None` for a job that started on one.
    pub(crate) source_read: Cell<Option<PathReading>>,
    /// Whether the destination's folder read as a folder, within its root's
    /// bound, before this job took its thread; it is then not created.
    pub(crate) folder_is_dir: bool,
    /// Whether a copy that lands directly on the destination name emptied
    /// or wrote it before failing.
    pub(crate) wrote_dest: Cell<bool>,
}

impl Job<'_> {
    fn run(&self) -> Result<Done, Fail> {
        let creates =
            self.policy.parents == Parents::Create && self.policy.method != Method::HardLink;
        let parent = self
            .dest
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        let make_folders = |parent: &Path| {
            self.disk
                .create_dir_blocking(parent, Parents::Create)
                .map_err(|error| Fail::of(error, Side::Destination))
        };
        if creates
            && !self.folder_is_dir
            && let Some(parent) = parent
        {
            make_folders(parent)?;
        }
        let placed = self.place_once();
        // The folder read as there before this job took its thread, so it
        // was not created. A name missing on the way may be that folder,
        // gone since: when it now reads as gone, nothing of this placement
        // can be in it, so the folders are made and the placement is tried
        // once more. One read, on this failure alone.
        match placed {
            Err(fail)
                if creates
                    && self.folder_is_dir
                    && matches!(
                        fail.refused,
                        Refused::DestinationMissing | Refused::SourceMissing
                    )
                    && let Some(parent) = parent
                    && self.disk.follow_blocking(parent).exists() == Some(false) =>
            {
                make_folders(parent)?;
                self.place_once()
            }
            placed => placed,
        }
    }

    fn place_once(&self) -> Result<Done, Fail> {
        match self.policy.method {
            Method::Move => self.relocate(),
            Method::HardLink => self.hard_link(),
            Method::Copy => self.copy_keeping_source(),
        }
    }

    fn replaces(&self) -> bool {
        self.policy.replace == Replace::Allowed
    }

    fn relocate(&self) -> Result<Done, Fail> {
        let (disk, source, dest) = (self.disk, self.source, self.dest);
        // On a case-insensitive volume the destination name resolves to the
        // source file itself, so this is a rename to perform, not a collision
        // to reject.
        if source != dest && paths_are_same_file(source, dest) {
            return self.move_via_intermediate_name();
        }

        if self.replaces() {
            return match disk.move_replacing_blocking(source, dest) {
                Ok(_) => Ok(Done::Renamed),
                Err(error) if needs_copy(&error) => self.across_volumes(error),
                Err(error) => Err(Fail::of(error, Side::Either)),
            };
        }

        match disk.exclusive_rename_blocking(source, dest) {
            Some(Ok(())) => return Ok(Done::Renamed),
            Some(Err(error)) if needs_copy(&error) => return self.across_volumes(error),
            Some(Err(error)) => return Err(Fail::of(error, Side::Either)),
            // No exclusive rename here; reserve a name instead.
            None => {}
        }

        // Reserving fails closed: if no name can be reserved, the move does
        // not happen. A plain rename here would bring back the silent replace
        // on exactly the network mounts that refuse the reservation.
        //
        // The name reserved is beside the destination rather than the
        // destination itself, so a media scanner never sees an empty file
        // under the real name.
        let staged = staging_path_for(dest);
        self.claim(&staged)?;
        let (outcome, held) = match disk.move_replacing_blocking(source, &staged) {
            Ok(_) => (
                self.promote(&staged, dest).map(|()| Done::Renamed),
                StagedHolds::OnlyCopy,
            ),
            Err(error) if needs_copy(&error) && self.may_copy_instead() => match self.source_kind()
            {
                // A symlinked source is made again on the other volume, as
                // a move without the claim makes it. A rename refused across
                // volumes moved nothing, so the claim is still the empty file
                // this job made, and is released before the link is made.
                Ok(EntryKind::Symlink) => {
                    let _ = disk.remove_blocking(&staged, Remove::File);
                    return self.relink_across_volumes();
                }
                Ok(_) => return self.copy_through_staging(&staged),
                Err(fail) => (Err(fail), StagedHolds::EmptyClaim),
            },
            Err(error) if needs_copy(&error) => {
                (self.across_volumes(error), StagedHolds::EmptyClaim)
            }
            Err(error) => (Err(Fail::of(error, Side::Either)), StagedHolds::EmptyClaim),
        };
        outcome.map_err(|fail| match self.roll_back(&staged, held) {
            Some(left) => fail.noting(&left),
            None => fail,
        })
    }

    /// Whether a move a rename cannot make may be copied. A move never takes
    /// the recorded copy, whose hashes and proof it could not hand back.
    fn may_copy_instead(&self) -> bool {
        self.policy.fallback == Fallback::Copy && !matches!(self.policy.verify, Verify::Recorded(_))
    }

    /// A move a rename cannot make: copied when the policy allows it.
    pub(crate) fn across_volumes(&self, refusal: DiskError) -> Result<Done, Fail> {
        if !self.may_copy_instead() {
            return Err(Fail {
                refused: Refused::NeedsCopy,
                detail: refusal.into(),
            });
        }
        let (disk, dest) = (self.disk, self.dest);
        // A symlinked media file is a pointer, not the payload: copying it
        // would silently duplicate the whole file and drop the link.
        if self.source_kind()? == EntryKind::Symlink {
            return self.relink_across_volumes();
        }

        if !self.replaces() {
            let staged = staging_path_for(dest);
            self.claim(&staged)?;
            return self.copy_through_staging(&staged);
        }
        if let Err(fail) = self.copy_into(dest) {
            let _ = disk.remove_blocking(dest, Remove::File);
            return Err(fail);
        }
        self.unlink_moved_source(Done::CopiedThenRemovedSource)
    }

    /// What the source is, for a move a rename cannot make. The look made
    /// before this job took its thread stands for this one when it found the
    /// source; anything else is read now.
    fn source_kind(&self) -> Result<EntryKind, Fail> {
        let (disk, source) = (self.disk, self.source);
        let before = self.source_read.take().filter(|read| read.kind().is_some());
        let reading = before.unwrap_or_else(|| disk.uncached_blocking(source));
        reading.kind().ok_or_else(|| {
            let absence = format!("{}: {}", source.display(), reading.describe_absence());
            match reading.exists() {
                Some(false) => Fail::new(Refused::SourceMissing, io::ErrorKind::NotFound, absence),
                _ => Fail::unread(reading, Side::Source, absence),
            }
        })
    }

    /// Moves a symlinked source to another volume: a link to the same
    /// target is made at the destination, and the source link removed.
    fn relink_across_volumes(&self) -> Result<Done, Fail> {
        let (disk, source, dest) = (self.disk, self.source, self.dest);
        let link_target = disk
            .read_link_blocking(source)
            .map_err(|error| Fail::of(error, Side::Source))?;
        if self.replaces() {
            let _ = disk.remove_blocking(dest, Remove::File);
        }
        let linked = disk
            .link_blocking(&link_target, dest, Link::Symbolic)
            .map_err(|error| Fail::of(error, Side::Destination))?;
        if linked == LinkOutcome::NotPossible {
            return Err(Fail::new(
                Refused::Failed,
                io::ErrorKind::Unsupported,
                format!("{} cannot hold a symbolic link", dest.display()),
            ));
        }
        self.unlink_moved_source(Done::RelinkedThenRemovedSource)
    }

    /// Moves the source to another volume through `staged`, which this job
    /// reserved: copy into it and prove the copy, promote it onto the
    /// destination without replacing, and only then unlink the source.
    /// Nothing is under the real name while the copy is unproven, and a
    /// failure before the unlink leaves the source as it was.
    fn copy_through_staging(&self, staged: &Path) -> Result<Done, Fail> {
        let (fail, proven) = match self.copy_into(staged) {
            Ok(()) => match self.promote(staged, self.dest) {
                Ok(()) => return self.unlink_moved_source(Done::CopiedThenRemovedSource),
                Err(fail) => (fail, true),
            },
            Err(fail) => (fail, false),
        };
        // A copy is never the empty claim the note speaks of, so there is no
        // note to add; what became of the copy is logged by the rollback.
        let _ = self.roll_back(staged, StagedHolds::Copy { proven });
        Err(fail)
    }

    /// A copy that leaves its source alone. It is written and proven under a
    /// name reserved beside the destination, so an interruption never leaves
    /// a part-written file under the real name.
    fn copy_keeping_source(&self) -> Result<Done, Fail> {
        if let Verify::Recorded(evidence) = self.policy.verify {
            return self.recorded_copy(evidence).map(|()| Done::Copied);
        }
        let staged = staging_path_for(self.dest);
        self.claim(&staged)?;
        let placed = self
            .copy_into(&staged)
            .and_then(|()| match self.replaces() {
                true => self
                    .disk
                    .move_replacing_blocking(&staged, self.dest)
                    .map(|_| ())
                    .map_err(|error| Fail::of(error, Side::Destination)),
                false => self.promote(&staged, self.dest),
            });
        if let Err(fail) = placed {
            // The source was never touched, so the staged copy this job
            // created, and gave to nobody, is worth nothing.
            let _ = self.disk.remove_blocking(&staged, Remove::File);
            return Err(fail);
        }
        Ok(Done::Copied)
    }

    fn hard_link(&self) -> Result<Done, Fail> {
        let refusal = match self.disk.hard_link_unclassified(self.source, self.dest) {
            Ok(()) => {
                return match self.prove {
                    Some((seen, proof)) => self.prove_link(seen, proof),
                    None => Ok(Done::HardLinked),
                };
            }
            Err(error) => error,
        };
        let cannot = if super::is_cross_device_error(&refusal) {
            Some(CannotLink::DifferentVolume)
        } else if super::link_not_possible(&refusal) {
            Some(CannotLink::NotSupported)
        } else {
            None
        };
        let fail = Fail::of(
            DiskError::new(DiskVerb::Link, self.dest, refusal),
            Side::Either,
        );
        if fail.refused == Refused::DestinationExists {
            return Err(fail);
        }
        // Only a link the destination cannot hold, or one that failed for a
        // reason the disk does not name, is copied instead. A source that is
        // missing or unreachable, a refusal of access and a full disk are
        // answers in themselves.
        if self.policy.fallback == Fallback::Copy
            && (cannot.is_some() || fail.refused == Refused::Failed)
        {
            return self.copy_keeping_source();
        }
        Err(match cannot {
            Some(cannot) => Fail {
                refused: Refused::CannotLink(cannot),
                detail: fail.detail,
            },
            None => fail,
        })
    }

    /// Proves the link this job has just made at the destination, and
    /// removes it when it does not prove. Only a successful link call leads
    /// here, and a regular file that is not the link is removed only when it
    /// does not read with another length or modification time than `seen`,
    /// so the name removed is one this job created.
    fn prove_link(&self, seen: &Mark, proof: LinkProof) -> Result<Done, Fail> {
        let (disk, dest) = (self.disk, self.dest);
        let read = disk.call("lstat", || disk.inner.backend.lstat(dest));
        let doubt = match read.map(|stat| Mark::of_stat(&stat)) {
            Err(error) => Doubt::Unread(DiskError::new(DiskVerb::Read, dest, error)),
            // Not a regular file, so not removed as one, whoever made it. A
            // regular file that is not the link and has another length or
            // modification time than the source is another writer's.
            Ok(linked)
                if linked.kind() != Some(EntryKind::File)
                    || (!linked.is_link_of(seen) && linked.differs_in_content_facts(seen)) =>
            {
                let doubt = Doubt::OtherFile { linked };
                let after = LinkAfter::LeftAlone;
                self.unproven.replace(Some(Unproven { doubt, after }));
                return Err(Fail::new(
                    Refused::DestinationExists,
                    io::ErrorKind::AlreadyExists,
                    format!("{} is no longer the link made there", dest.display()),
                ));
            }
            Ok(linked) if !linked.is_link_of(seen) => Doubt::OtherFile { linked },
            Ok(_) if proof == LinkProof::Name => return Ok(Done::HardLinked),
            Ok(_) => {
                let resolved = disk.follow_blocking(dest);
                if resolved.size().is_some() && resolved.size() == seen.len() {
                    return Ok(Done::HardLinked);
                }
                Doubt::OtherLength(resolved)
            }
        };
        // Strict: an unproven link left behind would be taken for the file.
        let (after, fail) = match disk.remove_blocking(dest, Remove::File) {
            Ok(Removal::Removed) => (LinkAfter::Removed, None),
            Ok(Removal::AlreadyAbsent) => {
                let gone = format!("{} is already gone", dest.display());
                let fail = Fail::new(Refused::DestinationMissing, io::ErrorKind::NotFound, gone);
                (LinkAfter::AlreadyGone, Some(fail))
            }
            Err(error) => (
                LinkAfter::NotRemoved,
                Some(Fail::of(error, Side::Destination)),
            ),
        };
        // A fallback copy that is then refused still says what did not prove.
        self.unproven.replace(Some(Unproven { doubt, after }));
        if let Some(fail) = fail {
            return Err(fail);
        }
        if self.policy.fallback == Fallback::Copy {
            return self.copy_keeping_source();
        }
        let detail = format!("the hard link at {} could not be proven", dest.display());
        Err(Fail::new(
            Refused::CannotLink(CannotLink::Unverifiable),
            io::ErrorKind::Other,
            detail,
        ))
    }

    /// Reserves `path` so no other writer can take it, failing if it is
    /// already held.
    fn claim(&self, path: &Path) -> Result<(), Fail> {
        self.disk
            .open_blocking(path, Open::create_new())
            .map(drop)
            .map_err(|error| Fail::of(error, Side::Destination))
    }

    fn promote(&self, staged: &Path, dest: &Path) -> Result<(), Fail> {
        self.disk
            .promote_staged_blocking(staged, dest)
            .map_err(|error| {
                self.note_left(&error);
                Fail::of(error, Side::Destination)
            })
    }

    /// Records what a failed disk call left at a name it reserved.
    pub(crate) fn note_left(&self, error: &DiskError) {
        if let Some(left) = error.left() {
            self.left.borrow_mut().push(left.clone());
        }
    }

    /// Where the file is after putting it back from `from` under the source
    /// name reported failure. A put-back never replaces anything, so one
    /// that found the source name taken left the file at `from`, unread. A
    /// rename can otherwise apply and lose its reply, so one look at the
    /// source, on this failure alone, says which: present means the file
    /// reached it. Anything else leaves the file reported at `from`, the
    /// last name it is known to hold, with whatever the failed call itself
    /// left at the source name. A name that is gone says nothing of where
    /// the file went: an earlier promotion that reported failure may have
    /// taken it. A failed call that left its reservation under the source
    /// name may have left the empty claim there, or the file itself, renamed
    /// over the claim with its reply lost: only a regular file with content
    /// is the file put back.
    fn after_failed_put_back(&self, from: &Path, error: &DiskError) -> Leftover {
        if error.reason() != reason::DISK_DESTINATION_EXISTS {
            let reading = self.disk.uncached_blocking(self.source);
            let back = match error.left() {
                Some(_) => reading.is(EntryKind::File) && reading.size().is_some_and(|len| len > 0),
                None => reading.exists() == Some(true),
            };
            if back {
                return Leftover::PutBack {
                    at: self.source.into(),
                };
            }
        }
        self.note_left(error);
        Leftover::PutBackFailed { at: from.into() }
    }

    /// Undoes a failed move without destroying the file.
    ///
    /// Once the source has been renamed or copied into the staging name, that
    /// file is the only copy, so deleting it on failure loses it. Sonarr
    /// draws the same line: `RollbackPartialMove` deletes the target only
    /// after confirming the source still exists, and `RollbackMove` moves the
    /// file back when it does not.
    ///
    /// Returns what the refusal should add when an empty reserved name was
    /// put back under the source name or left in place rather than released.
    #[must_use = "a refusal says when an empty reserved name was kept"]
    pub(crate) fn roll_back(&self, staged: &Path, held: StagedHolds) -> Option<String> {
        let (disk, source) = (self.disk, self.source);
        let reading = disk.uncached_blocking(staged);
        let held = match held {
            // Only the empty claim this move created is removed. An empty
            // file there may also be an empty source, moved by a rename that
            // reported failure, so it goes only while the source still reads
            // as present: one look at the source, on this failure alone. A
            // source that reads as gone gets the empty file back.
            StagedHolds::EmptyClaim if holds_only_the_claim(reading) => {
                let source_reads = disk.uncached_blocking(source);
                if source_reads.exists() == Some(true) {
                    let _ = disk.remove_blocking(staged, Remove::File);
                    return None;
                }
                return Some(self.settle_empty_claim(staged, source_reads));
            }
            StagedHolds::EmptyClaim if reading.exists() == Some(false) => return None,
            // An unread claim may be the source, moved by a rename that
            // reported failure. It goes back only when the source reads as
            // gone, and never over anything: an empty file under a source
            // name that holds nothing is the worst this can leave.
            StagedHolds::EmptyClaim if reading.exists().is_none() => {
                let source_gone = disk.uncached_blocking(source).exists() == Some(false);
                let left = match source_gone {
                    true => match disk.promote_staged_blocking(staged, source) {
                        Ok(()) => Leftover::PutBack { at: source.into() },
                        Err(error) => self.after_failed_put_back(staged, &error),
                    },
                    false => Leftover::AtReservedName { at: staged.into() },
                };
                if !matches!(left, Leftover::PutBack { .. }) {
                    tracing::error!(
                        staged = %staged.display(),
                        source = %source.display(),
                        state = reading.describe_absence(),
                        "the claim of a failed move could not be read; it is left in place"
                    );
                }
                self.left.borrow_mut().push(left);
                return None;
            }
            StagedHolds::EmptyClaim => StagedHolds::OnlyCopy,
            held => held,
        };
        let proven = match held {
            StagedHolds::Copy { proven } => proven,
            _ => true,
        };
        // The only copy goes back even unread: the promotion cannot replace
        // anything, so trying costs nothing.
        if reading.exists() != Some(true)
            && !(held == StagedHolds::OnlyCopy && reading.exists().is_none())
        {
            tracing::error!(
                staged = %staged.display(),
                source = %source.display(),
                state = reading.describe_absence(),
                "the staged file of a failed move does not read as present; nothing was rolled back"
            );
            // A copy at a name that could not be read may still be there.
            if reading.exists().is_none() {
                self.left
                    .borrow_mut()
                    .push(Leftover::AtReservedName { at: staged.into() });
            }
            return None;
        }

        // The only copy goes back whatever is at the source now: the
        // promotion refuses a file that has since appeared there.
        let source_present = match held {
            StagedHolds::OnlyCopy => Some(false),
            _ => disk.uncached_blocking(source).exists(),
        };
        match source_present {
            // The source survived, so the staged file is a copy of it.
            Some(true) => {
                let _ = disk.remove_blocking(staged, Remove::File);
            }
            // The staged file is the file: put it back where it came from,
            // never over a file that has since appeared there.
            Some(false) if proven => {
                let left = match disk.promote_staged_blocking(staged, source) {
                    Ok(()) => Leftover::PutBack { at: source.into() },
                    Err(error) => {
                        let left = self.after_failed_put_back(staged, &error);
                        if matches!(left, Leftover::PutBackFailed { .. }) {
                            tracing::error!(
                                staged = %staged.display(),
                                source = %source.display(),
                                "failed to restore a staged file after a failed move; the file is left at the staging path rather than removed"
                            );
                        }
                        left
                    }
                };
                self.left.borrow_mut().push(left);
            }
            Some(false) => {
                tracing::error!(
                    staged = %staged.display(),
                    source = %source.display(),
                    "the source is gone and the staged copy of it was never proven; the copy is left at the staging path"
                );
                self.left
                    .borrow_mut()
                    .push(Leftover::AtReservedName { at: staged.into() });
            }
            None => {
                tracing::error!(
                    staged = %staged.display(),
                    source = %source.display(),
                    "could not tell whether the source survived a failed move; the staged file is left in place"
                );
                self.left
                    .borrow_mut()
                    .push(Leftover::AtReservedName { at: staged.into() });
            }
        }
        None
    }

    /// Settles an empty file at a reserved name, `name`, that may be the
    /// source moved by a rename that reported failure, once the source read
    /// as `source_reads` and not as present. A source that reads as gone gets
    /// the file back under its name, never over anything there; one that
    /// cannot be read, or a put-back that fails, leaves the file where it is.
    /// Nothing is removed. Returns what the refusal adds, and records where
    /// the file is in [`PlaceRefusal::left`].
    fn settle_empty_claim(&self, name: &Path, source_reads: PathReading) -> String {
        let (disk, source) = (self.disk, self.source);
        if source_reads.exists() == Some(false) {
            let put_back = format!(
                "the empty file at {} may be the source and was put back at {}",
                name.display(),
                source.display()
            );
            return match disk.promote_staged_blocking(name, source) {
                Ok(()) => {
                    self.left
                        .borrow_mut()
                        .push(Leftover::PutBack { at: source.into() });
                    put_back
                }
                Err(error) => {
                    let left = self.after_failed_put_back(name, &error);
                    let put_back_after_all = matches!(left, Leftover::PutBack { .. });
                    self.left.borrow_mut().push(left);
                    if put_back_after_all {
                        return put_back;
                    }
                    tracing::error!(
                        name = %name.display(),
                        source = %source.display(),
                        error = %error.words(),
                        "the source of a failed move reads as gone and the empty file at its reserved name could not be put back; it is left in place"
                    );
                    format!(
                        "the empty file at {} may be the source and is left in place (putting it back at {} failed: {})",
                        name.display(),
                        source.display(),
                        error.words()
                    )
                }
            };
        }
        tracing::error!(
            name = %name.display(),
            source = %source.display(),
            state = source_reads.describe_absence(),
            "the source of a failed move cannot be read; the empty file at its reserved name is left in place"
        );
        self.left
            .borrow_mut()
            .push(Leftover::AtReservedName { at: name.into() });
        left_as_the_source(name, source, source_reads)
    }

    /// Finishes a move whose destination is in place by unlinking the source.
    /// The copy is then the file, so a source already gone is a finished
    /// move. A Windows read-only attribute that refuses the unlink is
    /// cleared, and restored if the unlink still fails. When the unlink
    /// fails, one look at the source decides: gone means the move finished;
    /// still there means the new destination is taken back, so no duplicate
    /// refuses the retry; unknown leaves both and reports the failure.
    pub(crate) fn unlink_moved_source(&self, done: Done) -> Result<Done, Fail> {
        let (disk, source) = (self.disk, self.source);
        let Err(error) = disk.remove_blocking(source, Remove::FileEvenReadOnly) else {
            return Ok(done);
        };
        match disk.uncached_blocking(source).exists() {
            Some(false) => Ok(done),
            Some(true) => {
                let _ = disk.remove_blocking(self.dest, Remove::File);
                Err(Fail::of(error, Side::Source))
            }
            None => Err(Fail::of(error, Side::Source)),
        }
    }

    /// Copies into a name this job already holds and proves the copy. The
    /// handle is opened for writing without creating, so the reservation is
    /// what decided the file may be written.
    fn copy_into(&self, target: &Path) -> Result<(), Fail> {
        let writing = Writing {
            opening: Opening::Held,
            hash: false,
            sync_folder: false,
            carry_permissions: true,
            chunk_bytes: crate::copy::CHUNK_BYTES,
            seen: None,
            phases: false,
            mark_target: false,
            watch: self.watch,
        };
        let sync = write_copy(self.disk, self.source, target, writing)
            .map_err(|fail| self.gate_failure(fail, target))?
            .sync;

        // A move removes its source next, so its copy is proven whatever
        // the policy asks.
        let unconfirmed = sync == SyncOutcome::Unconfirmed;
        if self.policy.verify == Verify::Quick || self.policy.method == Method::Move || unconfirmed
        {
            if unconfirmed {
                tracing::debug!(
                    dest = %target.display(),
                    "the destination filesystem does not support fsync; verifying the copy"
                );
            }
            self.verify_copy_with_retry(target)?;
        }
        Ok(())
    }

    /// A failed copy as a gated placement reports it: in the system's words.
    fn gate_failure(&self, fail: WriteFail, target: &Path) -> Fail {
        let side = match fail.step {
            Step::OpenSource => Side::Source,
            Step::Read | Step::Write | Step::Transfer => Side::Either,
            _ => Side::Destination,
        };
        match fail.cause {
            _ if fail.step == Step::Cancelled => Fail::new(
                Refused::Cancelled,
                io::ErrorKind::Interrupted,
                format!("the copy to {} was cancelled", target.display()),
            ),
            Cause::Disk(error) => Fail::of(error, side),
            Cause::Io(error) => Fail::io(error, target, side),
        }
    }

    /// Verifies a copy, looking once more after a pause.
    ///
    /// A network filesystem can report a stale size immediately after a
    /// write, so failing on the first look turns a good copy into a failed
    /// move. Sonarr does the same thing for the same reason, calling it out
    /// as needed for remote NAS devices.
    fn verify_copy_with_retry(&self, copy: &Path) -> Result<(), Fail> {
        if self.same_sample(copy).is_ok() {
            return Ok(());
        }
        self.disk.inner.clock.pause(COPY_VERIFY_RETRY_DELAY);
        self.same_sample(copy)
    }

    fn same_sample(&self, copy: &Path) -> Result<(), Fail> {
        let source = self.sample(self.source, Side::Source)?;
        let copied = self.sample(copy, Side::Destination)?;
        if source == copied {
            return Ok(());
        }
        let describe = |path: &Path, sample: &ContentSample| {
            format!(
                "{} (size={} sample_bytes={} sample_blake3={})",
                path.display(),
                sample.size_bytes,
                sample.sample_bytes,
                sample.sample_blake3
            )
        };
        Err(Fail::new(
            Refused::NotVerified,
            io::ErrorKind::InvalidData,
            format!(
                "copy verification failed: {} is not identical to {}",
                describe(self.source, &source),
                describe(copy, &copied)
            ),
        ))
    }

    fn sample(&self, path: &Path, side: Side) -> Result<ContentSample, Fail> {
        let unread = |error: io::Error| Fail {
            refused: Refused::NotVerified,
            detail: io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "copy verification could not read {}: {error}",
                    path.display()
                ),
            ),
        };
        let mut file = self
            .disk
            .open_blocking(path, Open::read())
            .map_err(|error| unread(Fail::of(error, side).detail))?;
        let size_bytes = file.metadata().map_err(unread)?.len();
        sample_reader(&mut file, size_bytes).map_err(|(_, error)| unread(error))
    }

    /// Renames through a uniquely named sibling so a case-only change lands
    /// even where the volume treats both names as the same file.
    fn move_via_intermediate_name(&self) -> Result<Done, Fail> {
        let (disk, source, dest) = (self.disk, self.source, self.dest);
        let parent = source.parent().unwrap_or_else(|| Path::new("."));
        let mut last_failure = None;
        for _ in 0..10 {
            let id = uuid::Uuid::new_v4().simple().to_string();
            let intermediate = parent.join(format!(".scryer-move-{}", &id[..8]));
            if self.claim(&intermediate).is_err() {
                continue;
            }
            // The claim only reserved the name; the rename replaces it.
            match disk.move_replacing_blocking(source, &intermediate) {
                // Neither hop may replace a file: on a case-sensitive volume
                // the destination is a different file whose name differs only
                // in case.
                Ok(_) => {
                    return match self.promote(&intermediate, dest) {
                        Ok(()) => Ok(Done::Renamed),
                        Err(fail) => {
                            // Put the file back rather than stranding it.
                            let left = match disk.promote_staged_blocking(&intermediate, source) {
                                Ok(()) => Leftover::PutBack { at: source.into() },
                                Err(error) => {
                                    let left = self.after_failed_put_back(&intermediate, &error);
                                    if matches!(left, Leftover::PutBackFailed { .. }) {
                                        tracing::error!(
                                            intermediate = %intermediate.display(),
                                            source = %source.display(),
                                            "failed to restore a file after a refused case-only rename; it is left at the intermediate name"
                                        );
                                    }
                                    left
                                }
                            };
                            self.left.borrow_mut().push(left);
                            Err(fail)
                        }
                    };
                }
                // The rename may have landed and lost its reply, so only the
                // empty claim is released; anything else there may be the
                // only copy. An empty file may be an empty source, so it is
                // released only while the source still reads as present.
                Err(error) => {
                    let reading = disk.uncached_blocking(&intermediate);
                    if !holds_only_the_claim(reading) {
                        tracing::error!(
                            intermediate = %intermediate.display(),
                            source = %source.display(),
                            "a case-only rename reported failure and its intermediate name is not the empty claim; it is left in place"
                        );
                        // The rename may have applied: the source's only copy
                        // can be at the intermediate name.
                        if reading.exists() != Some(false) {
                            self.left.borrow_mut().push(Leftover::AtReservedName {
                                at: intermediate.clone(),
                            });
                        }
                        return Err(Fail::of(error, Side::Either));
                    }
                    let source_reads = disk.uncached_blocking(source);
                    if source_reads.exists() != Some(true) {
                        let note = self.settle_empty_claim(&intermediate, source_reads);
                        return Err(Fail::of(error, Side::Either).noting(&note));
                    }
                    let _ = disk.remove_blocking(&intermediate, Remove::File);
                    last_failure = Some(Fail::of(error, Side::Either));
                }
            }
        }
        Err(last_failure.unwrap_or_else(|| {
            Fail::new(
                Refused::Failed,
                io::ErrorKind::Other,
                "could not claim an intermediate name for a case-only rename".to_string(),
            )
        }))
    }
}
