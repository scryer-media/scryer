//! "Files are already there": verify and adopt content the user moved outside
//! Scryer (US3, FR-050–053).
//!
//! Adoption never rewrites stored path prefixes. It scans the destination and
//! matches tracked media using stored identity information, size, media
//! characteristics, and stored content signatures — the sampled proof always,
//! and the persisted full BLAKE3 where one exists (FR-050). Insufficient proof
//! produces an unresolved state, never a guess (FR-052).
//!
//! # Three things live here
//!
//! 1. [`match_title_adoption`] — the matcher (T050). Pure: the caller assembles
//!    [`TrackedMediaFact`]s and [`DestinationFileFact`]s and gets accounting
//!    back, the way [`crate::location::classify`] and
//!    [`crate::location::collisions`] take their facts. Every FR-050/FR-051 rule
//!    is therefore testable from literals.
//! 2. The plan-time value types the accounting becomes: [`AdoptedMediaFile`],
//!    [`AdoptionFileProof`] (carried on the confirmed instruction set so a
//!    resumed run proves the same thing the preview promised).
//! 3. [`AdoptionFileVerifier`] — the executor's [`TitleFileMover`] for
//!    [`crate::location::model::LocationExecutionMode::FilesAlreadyThere`]. It
//!    copies nothing: it proves the destination at the operation's depth and
//!    lets the shared runner do everything else.
//!
//! # Evidence tiers, and what each one licenses
//!
//! The matcher considers a destination file for a tracked media file only when
//! nothing *excludes* it — a different size, or a content signature the two
//! sides both carry and disagree on. Among the surviving candidates it takes the
//! strongest tier available (see [`AdoptionMatchStrength`]) and requires that
//! tier to name exactly one file, breaking a tie only on structural identity
//! (relative path, then file name, then the stored file signature). Anything
//! left over is [`AdoptionAccounting::Ambiguous`] — never a guess (FR-052).
//!
//! The tier is recorded rather than thrown away because FR-053 hangs off it: a
//! match is enough to *adopt* at every tier, but only a full-hash match is
//! enough to recycle a source copy the user still has. That is the whole
//! difference between "Scryer knows where your files went" and "Scryer may
//! delete the ones you kept".

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use scryer_domain::{
    AppliedVerificationDepth, FileVerificationOutcome, ImportContentProof, VerificationDepth,
};

use crate::file_source_signature::FileSourceSignature;
use crate::location::collisions::FullHash;
use crate::location::execution::RootMoveCatalog;
use crate::location::executor::{FileMoveRequest, TitleFileMover};
use crate::location::root_move::RootMoveExecutionPlan;
use crate::location::verify::{VerifiedFile, hash_existing_file};
use crate::stored_paths::stored_path_to_path_buf;
use crate::AppResult;

/// How one file at the destination is accounted for during adoption (FR-051).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionAccounting {
    /// Tracked media matched to exactly one destination file with sufficient
    /// proof.
    AccountedFor,
    /// Tracked media with no matching destination file; blocks confirmation.
    Missing,
    /// A destination file that no tracked media claims; surfaced, never ignored.
    Additional,
    /// More than one plausible match, or proof too weak to decide; blocks
    /// confirmation (FR-052).
    Ambiguous,
}

impl AdoptionAccounting {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccountedFor => "accounted_for",
            Self::Missing => "missing",
            Self::Additional => "additional",
            Self::Ambiguous => "ambiguous",
        }
    }

    /// Confirmation is blocked while required tracked media is missing or
    /// ambiguous (FR-052).
    pub fn blocks_confirmation(&self) -> bool {
        matches!(self, Self::Missing | Self::Ambiguous)
    }
}

/// Strength of the evidence that tied a tracked media file to a destination
/// file. Recorded so the guarantee given is auditable afterwards (C4).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionMatchStrength {
    /// Persisted full BLAKE3 matched: the strongest proof available.
    FullHash,
    /// Size plus the sampled head+tail proof matched.
    SampledProof,
    /// Only stored identity and media characteristics lined up; not sufficient
    /// on its own to recycle a source copy (FR-053).
    IdentityOnly,
}

impl AdoptionMatchStrength {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FullHash => "full_hash",
            Self::SampledProof => "sampled_proof",
            Self::IdentityOnly => "identity_only",
        }
    }

    /// Source cleanup is left to the user unless Scryer can prove the source
    /// Source cleanup is left to the user unless Scryer can prove the source
    /// copy is redundant (FR-053).
    pub fn permits_source_recycle(&self) -> bool {
        matches!(self, Self::FullHash)
    }

    /// Strongest first, so the matcher can walk tiers in order without encoding
    /// the order twice.
    fn rank(&self) -> u8 {
        match self {
            Self::FullHash => 2,
            Self::SampledProof => 1,
            Self::IdentityOnly => 0,
        }
    }
}

/// Machine-readable reason codes on the plan items adoption emits, so the UI
/// groups and translates rather than parsing prose (C3).
pub mod plan_reasons {
    /// A tracked media file was found at the destination and is adopted where
    /// it lies; no bytes move (FR-050).
    pub const ADOPTED_AT_DESTINATION: &str = "adopted_at_destination";
    /// A tracked media file has no match at the destination, so confirmation is
    /// blocked (FR-052).
    pub const ADOPTION_MEDIA_MISSING: &str = "adoption_media_missing";
    /// More than one destination file could be this tracked media, or the proof
    /// was too weak to choose; confirmation is blocked (FR-052).
    pub const ADOPTION_MEDIA_AMBIGUOUS: &str = "adoption_media_ambiguous";
    /// A destination file no tracked media claims. Surfaced, never ignored
    /// (FR-051); adoption does not delete it and does not adopt it.
    pub const ADOPTION_ADDITIONAL_FILE: &str = "adoption_additional_file";
    /// The destination folder adoption was pointed at could not be read, so
    /// nothing about this title can be accounted for.
    pub const ADOPTION_DESTINATION_UNREADABLE: &str = "adoption_destination_unreadable";
    /// The user still holds a source copy that a full-hash match proved
    /// redundant, so it is recycled rather than left behind (FR-053).
    pub const ADOPTION_REDUNDANT_SOURCE: &str = "adoption_redundant_source";
    /// The source mount is not available. Adoption proceeds anyway, and source
    /// cleanup stays with the user (FR-053, US3.3).
    pub const ADOPTION_SOURCE_UNAVAILABLE: &str = "adoption_source_unavailable";
}

// ── Facts in ─────────────────────────────────────────────────────────────────

/// One tracked media file adoption has to find, as the catalog knows it.
///
/// `sampled_proof` is the head+tail proof of the file **at its stored source
/// path**, present only when that path is still readable. Its absence is the
/// ordinary case — the whole premise of US3 is that the source moved — and it
/// is not evidence of anything (FR-053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedMediaFact {
    pub media_file_id: String,
    /// Stored path the catalog holds today.
    pub source_path: String,
    /// Path relative to the title's source folder, when the file lived inside
    /// it. External moves preserve layout, so this is the strongest structural
    /// discriminator available.
    pub relative_path: Option<String>,
    pub file_name: String,
    /// Size the catalog recorded for this file.
    pub size_bytes: u64,
    /// The persisted full BLAKE3 (D4). [`FullHash::Stale`] proves nothing.
    pub full_blake3: FullHash,
    /// The stored file signature (mtime), which an external `mv` preserves.
    pub signature: Option<FileSourceSignature>,
    /// Head+tail proof of the source file, when the source is still readable.
    pub sampled_proof: Option<ImportContentProof>,
}

impl TrackedMediaFact {
    pub fn new(
        media_file_id: impl Into<String>,
        source_path: impl Into<String>,
        size_bytes: u64,
    ) -> Self {
        let source_path = source_path.into();
        let file_name = file_name_of(&source_path);
        Self {
            media_file_id: media_file_id.into(),
            source_path,
            relative_path: None,
            file_name,
            size_bytes,
            full_blake3: FullHash::Absent,
            signature: None,
            sampled_proof: None,
        }
    }

    pub fn with_relative_path(mut self, relative_path: impl Into<String>) -> Self {
        self.relative_path = Some(relative_path.into());
        self
    }

    pub fn with_full_blake3(mut self, hash: FullHash) -> Self {
        self.full_blake3 = hash;
        self
    }

    pub fn with_signature(mut self, signature: Option<FileSourceSignature>) -> Self {
        self.signature = signature;
        self
    }

    pub fn with_sampled_proof(mut self, proof: Option<ImportContentProof>) -> Self {
        self.sampled_proof = proof;
        self
    }
}

/// One file the destination scan found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationFileFact {
    /// Stored path of the file at the destination.
    pub path: String,
    /// Path relative to the destination folder.
    pub relative_path: Option<String>,
    pub file_name: String,
    pub size_bytes: u64,
    /// A full BLAKE3 the caller chose to compute for this file. Absent by
    /// default: hashing every destination file to render a preview would make
    /// an interactive screen read the whole library.
    pub full_blake3: FullHash,
    pub signature: Option<FileSourceSignature>,
    pub sampled_proof: Option<ImportContentProof>,
}

impl DestinationFileFact {
    pub fn new(path: impl Into<String>, size_bytes: u64) -> Self {
        let path = path.into();
        let file_name = file_name_of(&path);
        Self {
            path,
            relative_path: None,
            file_name,
            size_bytes,
            full_blake3: FullHash::Absent,
            signature: None,
            sampled_proof: None,
        }
    }

    pub fn with_relative_path(mut self, relative_path: impl Into<String>) -> Self {
        self.relative_path = Some(relative_path.into());
        self
    }

    pub fn with_full_blake3(mut self, hash: FullHash) -> Self {
        self.full_blake3 = hash;
        self
    }

    pub fn with_signature(mut self, signature: Option<FileSourceSignature>) -> Self {
        self.signature = signature;
        self
    }

    pub fn with_sampled_proof(mut self, proof: Option<ImportContentProof>) -> Self {
        self.sampled_proof = proof;
        self
    }
}

fn file_name_of(stored_path: &str) -> String {
    stored_path_to_path_buf(stored_path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| stored_path.to_string())
}

// ── Accounting out ───────────────────────────────────────────────────────────

/// What the executor re-proves for one adopted file, carried on the confirmed
/// instruction set so a resumed run proves what the preview promised (FR-089).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdoptionFileProof {
    /// The evidence that tied this destination file to this media file.
    pub strength: AdoptionMatchStrength,
    /// The persisted full BLAKE3 the destination is proven against, when the
    /// catalog holds a current one (D4).
    pub full_blake3: Option<String>,
    /// The stored file signature, as corroboration in the quick floor.
    pub signature: Option<String>,
}

/// One tracked media file matched to one destination file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptedMediaFile {
    pub media_file_id: String,
    pub source_path: String,
    pub destination_path: String,
    pub size_bytes: u64,
    pub proof: AdoptionFileProof,
}

/// One tracked media file adoption could not account for. Either variant blocks
/// confirmation (FR-052).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnaccountedMediaFile {
    pub media_file_id: String,
    pub source_path: String,
    pub size_bytes: u64,
    /// [`AdoptionAccounting::Missing`] or [`AdoptionAccounting::Ambiguous`].
    pub accounting: AdoptionAccounting,
    /// Why, in the words the preview shows (C3).
    pub detail: String,
}

/// A destination file no tracked media claimed (FR-051).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdditionalDestinationFile {
    pub path: String,
    pub size_bytes: u64,
}

/// Complete per-class counts for one title's destination (FR-051).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AdoptionCounts {
    pub accounted_for: i64,
    pub missing: i64,
    pub additional: i64,
    pub ambiguous: i64,
}

impl AdoptionCounts {
    /// Confirmation is blocked while required tracked media is missing or
    /// ambiguous (FR-052). Additional files never block: they are the user's,
    /// and adoption neither adopts nor removes them.
    pub fn blocks_confirmation(&self) -> bool {
        self.missing > 0 || self.ambiguous > 0
    }
}

/// One title's destination accounting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleAdoptionAccounting {
    pub adopted: Vec<AdoptedMediaFile>,
    pub unaccounted: Vec<UnaccountedMediaFile>,
    pub additional: Vec<AdditionalDestinationFile>,
}

impl TitleAdoptionAccounting {
    pub fn counts(&self) -> AdoptionCounts {
        let mut counts = AdoptionCounts {
            accounted_for: self.adopted.len() as i64,
            additional: self.additional.len() as i64,
            ..AdoptionCounts::default()
        };
        for file in &self.unaccounted {
            match file.accounting {
                AdoptionAccounting::Ambiguous => counts.ambiguous += 1,
                _ => counts.missing += 1,
            }
        }
        counts
    }

    /// FR-052, as the preview asks it.
    pub fn blocks_confirmation(&self) -> bool {
        self.counts().blocks_confirmation()
    }

    /// Bytes this title adopts. Nothing is written, so this is what will be
    /// *read* at the operation's verification depth, not what needs free space.
    pub fn adopted_bytes(&self) -> u64 {
        self.adopted
            .iter()
            .fold(0_u64, |total, file| total.saturating_add(file.size_bytes))
    }
}

// ── The matcher (T050) ───────────────────────────────────────────────────────

/// Match one title's tracked media against what the destination folder holds
/// (FR-050, FR-051).
///
/// Performs no IO. Deterministic: candidates are considered strongest tier
/// first and, within a tier, in the order the caller supplied the tracked
/// files. Two tracked files that can only be the same destination file leave
/// the later one unaccounted rather than doubling up on it — the content is not
/// there twice, and saying it is would be the guess FR-052 forbids.
pub fn match_title_adoption(
    tracked: &[TrackedMediaFact],
    destination: &[DestinationFileFact],
) -> TitleAdoptionAccounting {
    let mut claimed: BTreeSet<usize> = BTreeSet::new();
    let mut adopted: Vec<AdoptedMediaFile> = Vec::new();
    let mut unaccounted: Vec<UnaccountedMediaFile> = Vec::new();

    // Strongest tier first across the whole title, so a file that can be proven
    // by hash claims its destination before a file that can only be recognised
    // by name reaches for the same one.
    let mut order: Vec<(usize, u8)> = tracked
        .iter()
        .enumerate()
        .map(|(index, fact)| (index, best_available_rank(fact, destination)))
        .collect();
    order.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));

    let mut resolved: BTreeMap<usize, Result<AdoptedMediaFile, UnaccountedMediaFile>> =
        BTreeMap::new();

    for (index, _) in order {
        let fact = &tracked[index];
        let candidates: Vec<(usize, AdoptionMatchStrength)> = destination
            .iter()
            .enumerate()
            .filter(|(position, _)| !claimed.contains(position))
            .filter_map(|(position, candidate)| {
                evidence_for(fact, candidate).map(|strength| (position, strength))
            })
            .collect();

        if candidates.is_empty() {
            resolved.insert(
                index,
                Err(UnaccountedMediaFile {
                    media_file_id: fact.media_file_id.clone(),
                    source_path: fact.source_path.clone(),
                    size_bytes: fact.size_bytes,
                    accounting: AdoptionAccounting::Missing,
                    detail: format!(
                        "no file at the destination matches {} ({} bytes)",
                        fact.file_name, fact.size_bytes
                    ),
                }),
            );
            continue;
        }

        let best = candidates
            .iter()
            .map(|(_, strength)| strength.rank())
            .max()
            .unwrap_or(0);
        let contenders: Vec<usize> = candidates
            .iter()
            .filter(|(_, strength)| strength.rank() == best)
            .map(|(position, _)| *position)
            .collect();
        let strength = candidates
            .iter()
            .find(|(_, strength)| strength.rank() == best)
            .map(|(_, strength)| *strength)
            .unwrap_or(AdoptionMatchStrength::IdentityOnly);

        match narrow(fact, destination, &contenders) {
            Some(position) => {
                claimed.insert(position);
                resolved.insert(
                    index,
                    Ok(AdoptedMediaFile {
                        media_file_id: fact.media_file_id.clone(),
                        source_path: fact.source_path.clone(),
                        destination_path: destination[position].path.clone(),
                        size_bytes: destination[position].size_bytes,
                        proof: AdoptionFileProof {
                            strength,
                            full_blake3: fact.full_blake3.as_known().map(str::to_string),
                            signature: fact
                                .signature
                                .as_ref()
                                .map(|signature| signature.value.clone()),
                        },
                    }),
                );
            }
            None => {
                resolved.insert(
                    index,
                    Err(UnaccountedMediaFile {
                        media_file_id: fact.media_file_id.clone(),
                        source_path: fact.source_path.clone(),
                        size_bytes: fact.size_bytes,
                        accounting: AdoptionAccounting::Ambiguous,
                        detail: format!(
                            "{} destination files could be {}, and the stored proof cannot choose between them",
                            contenders.len(),
                            fact.file_name
                        ),
                    }),
                );
            }
        }
    }

    // Report in the caller's order, not in resolution order: the preview lists
    // a title's files the way the catalog holds them.
    for index in 0..tracked.len() {
        match resolved.remove(&index) {
            Some(Ok(file)) => adopted.push(file),
            Some(Err(file)) => unaccounted.push(file),
            None => {}
        }
    }

    let additional = destination
        .iter()
        .enumerate()
        .filter(|(position, _)| !claimed.contains(position))
        .map(|(_, file)| AdditionalDestinationFile {
            path: file.path.clone(),
            size_bytes: file.size_bytes,
        })
        .collect();

    TitleAdoptionAccounting {
        adopted,
        unaccounted,
        additional,
    }
}

/// The strongest tier this tracked file could reach against *any* destination
/// file, ignoring claims. Only used to order the passes.
fn best_available_rank(fact: &TrackedMediaFact, destination: &[DestinationFileFact]) -> u8 {
    destination
        .iter()
        .filter_map(|candidate| evidence_for(fact, candidate))
        .map(|strength| strength.rank())
        .max()
        .unwrap_or(0)
}

/// What, if anything, ties `candidate` to `fact`.
///
/// `None` means excluded, which is a stronger statement than "no evidence": a
/// size that differs, or a content signature both sides carry and disagree on,
/// positively rules the candidate out.
fn evidence_for(
    fact: &TrackedMediaFact,
    candidate: &DestinationFileFact,
) -> Option<AdoptionMatchStrength> {
    if fact.size_bytes != candidate.size_bytes {
        return None;
    }

    match (fact.full_blake3.as_known(), candidate.full_blake3.as_known()) {
        (Some(tracked), Some(found)) if tracked.eq_ignore_ascii_case(found) => {
            return Some(AdoptionMatchStrength::FullHash);
        }
        // Both sides hashed and the hashes differ: this is not the file.
        (Some(_), Some(_)) => return None,
        _ => {}
    }

    match (
        fact.sampled_proof.as_ref(),
        candidate.sampled_proof.as_ref(),
    ) {
        (Some(tracked), Some(found)) if tracked == found => {
            return Some(AdoptionMatchStrength::SampledProof);
        }
        (Some(_), Some(_)) => return None,
        _ => {}
    }

    Some(AdoptionMatchStrength::IdentityOnly)
}

/// Reduce equally-strong contenders to one, or give up.
///
/// The discriminators are structural rather than evidential — they cannot make
/// a match stronger, only pick between candidates the evidence already ranked
/// the same. Relative path first because an external move preserves layout,
/// then the file name, then the stored file signature (an `mv` preserves the
/// mtime; a `cp` does not, so it is corroboration and never a gate).
fn narrow(
    fact: &TrackedMediaFact,
    destination: &[DestinationFileFact],
    contenders: &[usize],
) -> Option<usize> {
    if contenders.len() == 1 {
        return contenders.first().copied();
    }

    let by_relative: Vec<usize> = contenders
        .iter()
        .copied()
        .filter(|position| {
            match (
                fact.relative_path.as_deref(),
                destination[*position].relative_path.as_deref(),
            ) {
                (Some(tracked), Some(found)) => tracked == found,
                _ => false,
            }
        })
        .collect();
    if by_relative.len() == 1 {
        return by_relative.first().copied();
    }
    let pool = if by_relative.is_empty() {
        contenders.to_vec()
    } else {
        by_relative
    };

    let by_name: Vec<usize> = pool
        .iter()
        .copied()
        .filter(|position| destination[*position].file_name == fact.file_name)
        .collect();
    if by_name.len() == 1 {
        return by_name.first().copied();
    }
    let pool = if by_name.is_empty() { pool } else { by_name };

    let by_signature: Vec<usize> = pool
        .iter()
        .copied()
        .filter(|position| {
            match (
                fact.signature.as_ref(),
                destination[*position].signature.as_ref(),
            ) {
                (Some(tracked), Some(found)) => tracked == found,
                _ => false,
            }
        })
        .collect();
    if by_signature.len() == 1 {
        return by_signature.first().copied();
    }

    None
}

// ── Destination folder resolution ────────────────────────────────────────────

/// Which folder adoption accounts against.
///
/// The calculated policy folder is the first answer, because that is where a
/// managed move would have put the content and what the preview promises
/// (FR-013). But the ordinary way a user moves a title by hand is `mv` — the
/// folder keeps the name it had — so a folder under the destination root
/// carrying the *source* folder's name is the second answer. Neither existing
/// leaves the calculated folder in place, and the accounting then reports every
/// tracked file missing, which is the truthful outcome rather than a silent one.
pub fn choose_adoption_folder(
    calculated: PathBuf,
    calculated_exists: bool,
    source_named: Option<PathBuf>,
    source_named_exists: bool,
) -> PathBuf {
    if calculated_exists {
        return calculated;
    }
    match source_named {
        Some(folder) if source_named_exists => folder,
        _ => calculated,
    }
}

// ── The executor branch: verify, never copy ──────────────────────────────────

/// The [`TitleFileMover`] for [`LocationExecutionMode::FilesAlreadyThere`].
///
/// Every file it is handed is already at its destination path. It moves
/// nothing; it proves the destination at the operation's depth against what the
/// catalog stored, records the applied depth, and hands the runner a
/// [`VerifiedFile`]. The rest of the operation — the completeness gate, the
/// per-title checkpoints, the ownership flip, safe cancel, resume — is the
/// shared machinery, unchanged.
///
/// # Why `hashes` is the recycle switch
///
/// [`crate::location::execution::RootMoveReconciler::clean_up_title`] recycles
/// a source only for a file whose verification record carries hashes, because
/// for a managed move that is exactly the set of files whose bytes were copied
/// and proven. Adoption keeps the same contract and gets FR-053 for free:
/// hashes are attached **only** when the destination's full BLAKE3 was computed
/// and matched the catalog's persisted one. That is the provable-redundancy
/// exception, and nothing weaker ever reaches the recycler — every other tier
/// leaves the user's source copy exactly where they put it.
pub struct AdoptionFileVerifier {
    /// Per-destination-path proof, read off the confirmed plan.
    proofs: BTreeMap<String, AdoptionFileProof>,
    /// Where a freshly computed full hash is persisted, so an adopted file
    /// leaves the backfill queue instead of being read twice (D4).
    catalog: Option<Arc<dyn RootMoveCatalog>>,
}

impl std::fmt::Debug for AdoptionFileVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdoptionFileVerifier")
            .field("proofs", &self.proofs.len())
            .finish()
    }
}

impl AdoptionFileVerifier {
    /// Build the verifier from the confirmed instruction set, so what is proven
    /// at execution time is what the preview described (FR-089).
    pub fn new(plan: &RootMoveExecutionPlan) -> Self {
        Self {
            proofs: plan.adoption_proofs.clone(),
            catalog: None,
        }
    }

    pub fn with_catalog(mut self, catalog: Arc<dyn RootMoveCatalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    async fn persist_hashes(
        &self,
        media_file_id: Option<&str>,
        hashes: &scryer_domain::StreamedContentHashes,
    ) {
        let (Some(catalog), Some(media_file_id)) = (self.catalog.as_ref(), media_file_id) else {
            return;
        };
        let persisted = crate::location::model::PersistedContentHashes::from_streamed(
            hashes,
            chrono::Utc::now(),
        );
        if let Err(error) = catalog
            .set_media_file_content_hashes(media_file_id, &persisted)
            .await
        {
            tracing::warn!(
                error = %error,
                media_file_id,
                "failed to persist the full hash an adoption computed; the backfill job will recompute it"
            );
        }
    }
}

#[async_trait]
impl TitleFileMover for AdoptionFileVerifier {
    async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
        let destination = &request.file.destination_path;
        let source = &request.file.source_path;
        let stored_destination = request.file.stored_destination();
        let proof = self.proofs.get(&stored_destination);

        let metadata = match tokio::fs::symlink_metadata(destination).await {
            Ok(metadata) => metadata,
            Err(error) => {
                return Ok(unavailable(
                    source,
                    destination,
                    request.depth,
                    format!(
                        "the content this adoption expects at {} is not there: {error}",
                        destination.display()
                    ),
                ));
            }
        };
        if !metadata.is_file() {
            return Ok(unavailable(
                source,
                destination,
                request.depth,
                format!("{} is not a regular file", destination.display()),
            ));
        }
        if metadata.len() != request.file.size_bytes {
            return Ok(VerifiedFile {
                source_path: source.clone(),
                destination_path: destination.clone(),
                hashes: None,
                depth: AppliedVerificationDepth::exact(request.depth),
                outcome: FileVerificationOutcome::Mismatch,
                detail: Some(format!(
                    "{} is {} bytes; the preview accounted for {} bytes",
                    destination.display(),
                    metadata.len(),
                    request.file.size_bytes
                )),
            });
        }

        // Full depth, and a persisted hash to compare against: the only
        // combination that can prove the destination end to end (FR-041/FR-050).
        let persisted = proof.and_then(|proof| proof.full_blake3.clone());
        if request.depth == VerificationDepth::Full
            && let Some(expected) = persisted
        {
            return match hash_existing_file(destination).await {
                Ok(hashes) => {
                    if hashes.full_blake3.eq_ignore_ascii_case(&expected) {
                        self.persist_hashes(request.file.media_file_id.as_deref(), &hashes)
                            .await;
                        Ok(VerifiedFile {
                            source_path: source.clone(),
                            destination_path: destination.clone(),
                            // The provable-redundancy switch (FR-053): only this
                            // path attaches hashes, so only this path lets the
                            // cleanup step recycle a source copy the user kept.
                            hashes: Some(hashes),
                            depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
                            outcome: FileVerificationOutcome::Verified,
                            detail: Some(format!(
                                "{} was read end to end and matched the full hash the catalog holds for this file",
                                destination.display()
                            )),
                        })
                    } else {
                        Ok(VerifiedFile {
                            source_path: source.clone(),
                            destination_path: destination.clone(),
                            hashes: None,
                            depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
                            outcome: FileVerificationOutcome::Mismatch,
                            detail: Some(format!(
                                "{} does not match the full hash the catalog holds for this file",
                                destination.display()
                            )),
                        })
                    }
                }
                Err(error) => Ok(quick_floor(
                    source,
                    destination,
                    request.depth,
                    proof,
                    Some(format!("the destination could not be read in full: {error}")),
                )
                .await),
            };
        }

        Ok(quick_floor(
            source,
            destination,
            request.depth,
            proof,
            (request.depth == VerificationDepth::Full).then(|| {
                "no current full-file hash is stored for this file, so the destination was proven at the quick floor; the backfill job will hash it"
                    .to_string()
            }),
        )
        .await)
    }
}

fn unavailable(
    source: &Path,
    destination: &Path,
    requested: VerificationDepth,
    detail: String,
) -> VerifiedFile {
    VerifiedFile {
        source_path: source.to_path_buf(),
        destination_path: destination.to_path_buf(),
        hashes: None,
        depth: AppliedVerificationDepth::exact(requested),
        outcome: FileVerificationOutcome::Unavailable,
        detail: Some(detail),
    }
}

/// The floor adoption can always reach: the size already compared, plus the
/// head+tail proof against the source when the source is still there, plus the
/// stored file signature as corroboration.
///
/// A source that is gone is not a failure here — it is the premise of US3
/// (FR-053) — so its absence downgrades the statement rather than the outcome.
async fn quick_floor(
    source: &Path,
    destination: &Path,
    requested: VerificationDepth,
    proof: Option<&AdoptionFileProof>,
    fallback_reason: Option<String>,
) -> VerifiedFile {
    let depth = match fallback_reason {
        Some(_) => AppliedVerificationDepth::quick_fallback(),
        None => AppliedVerificationDepth::exact(requested),
    };
    let mut notes: Vec<String> = fallback_reason.into_iter().collect();

    let source_present = tokio::fs::symlink_metadata(source).await.is_ok();
    if source_present {
        let source_path = source.to_path_buf();
        let destination_path = destination.to_path_buf();
        let compared = tokio::task::spawn_blocking(move || {
            crate::fs_integrity::verify_same_file(&source_path, &destination_path)
        })
        .await;
        match compared {
            Ok(Ok(())) => {
                notes.push(
                    "the source copy is still present and its head+tail proof matches the destination"
                        .to_string(),
                );
            }
            Ok(Err(error)) => {
                notes.push(error.to_string());
                return VerifiedFile {
                    source_path: source.to_path_buf(),
                    destination_path: destination.to_path_buf(),
                    hashes: None,
                    depth,
                    outcome: FileVerificationOutcome::Mismatch,
                    detail: Some(notes.join("; ")),
                };
            }
            Err(error) => notes.push(format!(
                "the head+tail comparison against the source could not run: {error}"
            )),
        }
    } else {
        notes.push(
            "the source is no longer present, so the destination was proven against the size and identity the catalog stored (FR-053)"
                .to_string(),
        );
    }

    if let Some(strength) = proof.map(|proof| proof.strength) {
        notes.push(format!("adoption match: {}", strength.as_str()));
    }

    VerifiedFile {
        source_path: source.to_path_buf(),
        destination_path: destination.to_path_buf(),
        hashes: None,
        depth,
        outcome: FileVerificationOutcome::Verified,
        detail: Some(notes.join("; ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proof(size: u64, sample: &str) -> ImportContentProof {
        ImportContentProof {
            size_bytes: size,
            sample_bytes: 32,
            sample_blake3: sample.to_string(),
        }
    }

    fn signature(value: &str) -> FileSourceSignature {
        FileSourceSignature {
            scheme: "unix_mtime_nsec_v1".to_string(),
            value: value.to_string(),
        }
    }

    /// US3.1: stored identity, size, and stored content signatures match the
    /// tracked media to what the user moved.
    #[test]
    fn a_file_moved_by_hand_is_accounted_for_by_size_and_identity() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/Show/S01E01.mkv", 1000)
                .with_relative_path("S01E01.mkv"),
        ];
        let destination =
            vec![DestinationFileFact::new("/new/Show/S01E01.mkv", 1000)
                .with_relative_path("S01E01.mkv")];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(accounting.counts().accounted_for, 1);
        assert!(!accounting.blocks_confirmation());
        assert_eq!(
            accounting.adopted[0].destination_path,
            "/new/Show/S01E01.mkv"
        );
        assert_eq!(
            accounting.adopted[0].proof.strength,
            AdoptionMatchStrength::IdentityOnly
        );
    }

    #[test]
    fn a_persisted_full_hash_is_the_strongest_match_and_licenses_recycling() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)
                .with_full_blake3(FullHash::known("abcdef")),
        ];
        let destination = vec![
            DestinationFileFact::new("/new/renamed.mkv", 4096)
                .with_full_blake3(FullHash::known("ABCDEF")),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        let adopted = &accounting.adopted[0];
        assert_eq!(adopted.proof.strength, AdoptionMatchStrength::FullHash);
        assert!(adopted.proof.strength.permits_source_recycle());
        assert_eq!(adopted.proof.full_blake3.as_deref(), Some("abcdef"));
    }

    #[test]
    fn the_sampled_proof_matches_when_the_source_is_still_readable() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)
                .with_sampled_proof(Some(proof(4096, "head-tail"))),
        ];
        let destination = vec![
            DestinationFileFact::new("/new/movie.mkv", 4096)
                .with_sampled_proof(Some(proof(4096, "head-tail"))),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(
            accounting.adopted[0].proof.strength,
            AdoptionMatchStrength::SampledProof
        );
        assert!(!accounting.adopted[0].proof.strength.permits_source_recycle());
    }

    #[test]
    fn a_full_hash_beats_a_sampled_proof_when_both_are_available() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)
                .with_full_blake3(FullHash::known("abcdef"))
                .with_sampled_proof(Some(proof(4096, "head-tail"))),
        ];
        let destination = vec![
            DestinationFileFact::new("/new/a.mkv", 4096)
                .with_sampled_proof(Some(proof(4096, "head-tail"))),
            DestinationFileFact::new("/new/b.mkv", 4096)
                .with_full_blake3(FullHash::known("abcdef")),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(accounting.adopted[0].destination_path, "/new/b.mkv");
        assert_eq!(
            accounting.adopted[0].proof.strength,
            AdoptionMatchStrength::FullHash
        );
        assert_eq!(accounting.additional.len(), 1);
    }

    /// US3.2: a tracked file with no match at the destination blocks the
    /// confirmation.
    #[test]
    fn tracked_media_with_no_destination_file_is_missing_and_blocks() {
        let tracked = vec![TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)];
        let destination = vec![DestinationFileFact::new("/new/other.mkv", 512)];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(accounting.counts().missing, 1);
        assert_eq!(accounting.counts().additional, 1);
        assert!(accounting.blocks_confirmation());
        assert_eq!(
            accounting.unaccounted[0].accounting,
            AdoptionAccounting::Missing
        );
    }

    /// US3.2: two equally plausible candidates are unresolved, never guessed.
    #[test]
    fn two_equally_plausible_destination_files_are_ambiguous_and_block() {
        let tracked = vec![TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)];
        let destination = vec![
            DestinationFileFact::new("/new/one.mkv", 4096),
            DestinationFileFact::new("/new/two.mkv", 4096),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert!(accounting.blocks_confirmation());
        assert_eq!(accounting.counts().ambiguous, 1);
        assert_eq!(
            accounting.unaccounted[0].accounting,
            AdoptionAccounting::Ambiguous
        );
        assert_eq!(accounting.counts().additional, 2);
    }

    #[test]
    fn an_identical_file_name_breaks_a_tie_between_same_sized_candidates() {
        let tracked = vec![TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)];
        let destination = vec![
            DestinationFileFact::new("/new/other.mkv", 4096),
            DestinationFileFact::new("/new/movie.mkv", 4096),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert!(!accounting.blocks_confirmation());
        assert_eq!(accounting.adopted[0].destination_path, "/new/movie.mkv");
    }

    #[test]
    fn the_relative_path_breaks_a_tie_before_the_file_name_does() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/Show/Season 02/ep.mkv", 4096)
                .with_relative_path("Season 02/ep.mkv"),
        ];
        let destination = vec![
            DestinationFileFact::new("/new/Show/Season 01/ep.mkv", 4096)
                .with_relative_path("Season 01/ep.mkv"),
            DestinationFileFact::new("/new/Show/Season 02/ep.mkv", 4096)
                .with_relative_path("Season 02/ep.mkv"),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(
            accounting.adopted[0].destination_path,
            "/new/Show/Season 02/ep.mkv"
        );
    }

    #[test]
    fn the_stored_file_signature_breaks_a_tie_the_name_cannot() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)
                .with_signature(Some(signature("111:222"))),
        ];
        let destination = vec![
            DestinationFileFact::new("/new/a.mkv", 4096).with_signature(Some(signature("999:000"))),
            DestinationFileFact::new("/new/b.mkv", 4096).with_signature(Some(signature("111:222"))),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(accounting.adopted[0].destination_path, "/new/b.mkv");
    }

    #[test]
    fn disagreeing_full_hashes_exclude_a_candidate_rather_than_ranking_it_lower() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)
                .with_full_blake3(FullHash::known("aaaa")),
        ];
        let destination = vec![
            DestinationFileFact::new("/new/movie.mkv", 4096)
                .with_full_blake3(FullHash::known("bbbb")),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(accounting.counts().missing, 1);
        assert_eq!(accounting.counts().accounted_for, 0);
    }

    #[test]
    fn a_stale_persisted_hash_proves_nothing_and_falls_back_to_identity() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)
                .with_full_blake3(FullHash::Stale),
        ];
        let destination = vec![
            DestinationFileFact::new("/new/movie.mkv", 4096)
                .with_full_blake3(FullHash::known("bbbb")),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(
            accounting.adopted[0].proof.strength,
            AdoptionMatchStrength::IdentityOnly
        );
        assert_eq!(accounting.adopted[0].proof.full_blake3, None);
    }

    #[test]
    fn two_tracked_files_never_share_one_destination_file() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)
                .with_full_blake3(FullHash::known("aaaa")),
            TrackedMediaFact::new("file-2", "/old/copy.mkv", 4096),
        ];
        let destination = vec![
            DestinationFileFact::new("/new/movie.mkv", 4096)
                .with_full_blake3(FullHash::known("aaaa")),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(accounting.counts().accounted_for, 1);
        assert_eq!(accounting.counts().missing, 1);
        assert_eq!(accounting.adopted[0].media_file_id, "file-1");
        assert_eq!(accounting.unaccounted[0].media_file_id, "file-2");
    }

    #[test]
    fn destination_files_nothing_claims_are_additional_and_never_block() {
        let tracked = vec![TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)];
        let destination = vec![
            DestinationFileFact::new("/new/movie.mkv", 4096),
            DestinationFileFact::new("/new/movie.nfo", 12),
            DestinationFileFact::new("/new/poster.jpg", 88),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert!(!accounting.blocks_confirmation());
        assert_eq!(accounting.counts().additional, 2);
    }

    #[test]
    fn the_accounting_reports_files_in_the_order_the_catalog_holds_them() {
        let tracked = vec![
            TrackedMediaFact::new("file-1", "/old/a.mkv", 10),
            TrackedMediaFact::new("file-2", "/old/b.mkv", 20)
                .with_full_blake3(FullHash::known("bb")),
        ];
        let destination = vec![
            DestinationFileFact::new("/new/a.mkv", 10),
            DestinationFileFact::new("/new/b.mkv", 20).with_full_blake3(FullHash::known("bb")),
        ];

        let accounting = match_title_adoption(&tracked, &destination);

        assert_eq!(
            accounting
                .adopted
                .iter()
                .map(|file| file.media_file_id.as_str())
                .collect::<Vec<_>>(),
            vec!["file-1", "file-2"]
        );
    }

    #[test]
    fn an_empty_destination_leaves_every_tracked_file_missing() {
        let tracked = vec![TrackedMediaFact::new("file-1", "/old/movie.mkv", 4096)];

        let accounting = match_title_adoption(&tracked, &[]);

        assert!(accounting.blocks_confirmation());
        assert_eq!(accounting.counts().missing, 1);
    }

    #[test]
    fn a_title_with_no_tracked_media_adopts_nothing_and_blocks_nothing() {
        let destination = vec![DestinationFileFact::new("/new/movie.mkv", 4096)];

        let accounting = match_title_adoption(&[], &destination);

        assert!(!accounting.blocks_confirmation());
        assert_eq!(accounting.counts().additional, 1);
    }

    #[test]
    fn the_calculated_folder_wins_when_it_exists_and_the_moved_folder_wins_otherwise() {
        let calculated = PathBuf::from("/root/Title (2024)");
        let moved = PathBuf::from("/root/Title.2024.1080p");

        assert_eq!(
            choose_adoption_folder(calculated.clone(), true, Some(moved.clone()), true),
            calculated
        );
        assert_eq!(
            choose_adoption_folder(calculated.clone(), false, Some(moved.clone()), true),
            moved
        );
        assert_eq!(
            choose_adoption_folder(calculated.clone(), false, Some(moved), false),
            calculated
        );
        assert_eq!(
            choose_adoption_folder(calculated.clone(), false, None, false),
            calculated
        );
    }
}
