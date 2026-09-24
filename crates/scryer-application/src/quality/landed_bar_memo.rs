//! Cross-cycle memo of re-derived landed bars for the background cutoff pass.
//!
//! The background acquisition cycle re-derives the landed bar of every occupied
//! scope whose profile sets `cutoff_score`, once a minute, to find the scopes
//! still below it. At idle every one of those derivations repeats the previous
//! minute's answer. This memo keeps each answer under a digest of **every input
//! the derivation reads** — the scorer version, the resolved scoring context,
//! the file row, the size basis and the scored episode span — so an entry can
//! only be hit by a derivation that would have produced the same number. A
//! changed input is a different key, not an invalidation somebody has to
//! remember to send.
//!
//! The one input a digest cannot see is the compiled rules engine. Replacing it
//! goes through [`crate::AppUseCase::swap_user_rules_engine`], which calls
//! [`LandedBarMemo::reset_for_engine`]: that clears the memo, advances its
//! generation (so a derivation that read the old engine cannot install its
//! result afterwards), and switches the memo off entirely while the engine
//! reads the clock, because such an engine is not a function of its input.
//!
//! Only the background cutoff pass reads and fills the memo. The grab gate and
//! the Wanted page keep re-deriving live.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Mutex, PoisonError};

/// Bump on any change to how a stored file is scored — the canonical scorer,
/// the release parser, the size bands, the tier ladder, or what
/// [`crate::quality::canonical_context::ResolvedScoringContext`] feeds them.
///
/// Memo entries are keyed on it, so a build whose scoring changed never reuses
/// a bar the previous scoring produced. (Entries do not outlive the process
/// today, so this matters once anything persists them; keep it honest now.)
pub(crate) const SCORER_VERSION: u32 = 1;

/// Hard bound on entries. A full pass drops every entry it did not touch, so
/// the memo normally holds one entry per live occupied scope; this is the
/// backstop. Reaching it clears the memo, which costs one pass of re-scoring
/// and never changes a result. About 40 bytes per entry, so ~20 MB at the cap.
const MAX_ENTRIES: usize = 500_000;

/// A 128-bit digest of one derivation's inputs.
pub(crate) type LandedBarKey = [u8; 16];

#[derive(Debug, Default)]
struct MemoState {
    /// Advanced by every engine swap. A ticket from an older generation may
    /// neither read nor fill.
    generation: u64,
    /// False while the active rules engine reads the clock.
    disabled: bool,
    /// The most recent full pass. Entries carry the pass that last touched
    /// them, and a completed full pass drops the ones it did not.
    pass: u64,
    entries: HashMap<LandedBarKey, MemoEntry>,
    hits: u64,
    fills: u64,
}

#[derive(Debug, Clone, Copy)]
struct MemoEntry {
    bar: i32,
    pass: u64,
}

/// What a derivation observed before it read the rules engine. Taken once per
/// batch, before any scoring context is resolved.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LandedBarMemoTicket {
    generation: u64,
    pass: u64,
}

/// Counters for one pass, reported by the cutoff pass's debug line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LandedBarMemoStats {
    pub entries: usize,
    pub hits: u64,
    pub fills: u64,
}

#[derive(Debug, Default)]
pub(crate) struct LandedBarMemo {
    state: Mutex<MemoState>,
}

impl LandedBarMemo {
    fn state(&self) -> std::sync::MutexGuard<'_, MemoState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Called after a new rules engine is installed.
    pub(crate) fn reset_for_engine(&self, engine_reads_clock: bool) {
        let mut state = self.state();
        state.generation = state.generation.wrapping_add(1);
        state.disabled = engine_reads_clock;
        state.entries.clear();
    }

    /// Start a full pass over every memoised scope. The returned id goes back
    /// to [`Self::finish_pass`] once the pass has completed.
    pub(crate) fn begin_pass(&self) -> u64 {
        let mut state = self.state();
        state.pass = state.pass.wrapping_add(1);
        state.hits = 0;
        state.fills = 0;
        state.pass
    }

    /// Drop every entry the completed pass `pass` did not touch — scopes whose
    /// file was deleted, re-scanned or re-scored under different inputs. A
    /// pass that a newer one has superseded sweeps nothing.
    pub(crate) fn finish_pass(&self, pass: u64) -> LandedBarMemoStats {
        let mut state = self.state();
        if state.pass == pass {
            state.entries.retain(|_, entry| entry.pass == pass);
        }
        LandedBarMemoStats {
            entries: state.entries.len(),
            hits: state.hits,
            fills: state.fills,
        }
    }

    /// `None` while the memo is off; the caller then derives every bar.
    pub(crate) fn ticket(&self) -> Option<LandedBarMemoTicket> {
        let state = self.state();
        (!state.disabled).then_some(LandedBarMemoTicket {
            generation: state.generation,
            pass: state.pass,
        })
    }

    pub(crate) fn get(&self, ticket: LandedBarMemoTicket, key: &LandedBarKey) -> Option<i32> {
        let mut state = self.state();
        if state.disabled || state.generation != ticket.generation {
            return None;
        }
        let entry = state.entries.get_mut(key)?;
        entry.pass = entry.pass.max(ticket.pass);
        let bar = entry.bar;
        state.hits += 1;
        Some(bar)
    }

    pub(crate) fn fill(&self, ticket: LandedBarMemoTicket, key: LandedBarKey, bar: i32) {
        let mut state = self.state();
        if state.disabled || state.generation != ticket.generation {
            return;
        }
        if state.entries.len() >= MAX_ENTRIES && !state.entries.contains_key(&key) {
            state.entries.clear();
        }
        state.entries.insert(
            key,
            MemoEntry {
                bar,
                pass: ticket.pass,
            },
        );
        state.fills += 1;
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.state().entries.len()
    }

    /// Hits and fills since the last [`Self::begin_pass`].
    #[cfg(test)]
    pub(crate) fn counters(&self) -> (u64, u64) {
        let state = self.state();
        (state.hits, state.fills)
    }
}

/// Streams `Debug` output into a hasher without building the string.
struct HashWriter<'a>(&'a mut blake3::Hasher);

impl std::fmt::Write for HashWriter<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.update(s.as_bytes());
        Ok(())
    }
}

/// Feeds `value`'s `Debug` rendering to `hasher`, length-delimited so two
/// adjacent fields cannot run into each other.
pub(crate) fn hash_debug(hasher: &mut blake3::Hasher, value: &impl std::fmt::Debug) {
    let mut rendered = blake3::Hasher::new();
    let _ = write!(HashWriter(&mut rendered), "{value:?}");
    hasher.update(rendered.finalize().as_bytes());
}

/// Feeds a JSON value to `hasher` with object keys sorted. The workspace builds
/// `serde_json` with `preserve_order`, so a map serialized from a `HashMap`
/// arrives in that map's iteration order; sorting makes equal values hash
/// equal.
pub(crate) fn hash_json(hasher: &mut blake3::Hasher, value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            hasher.update(b"{");
            for key in keys {
                hasher.update(&(key.len() as u64).to_le_bytes());
                hasher.update(key.as_bytes());
                hash_json(hasher, &map[key]);
            }
            hasher.update(b"}");
        }
        serde_json::Value::Array(items) => {
            hasher.update(b"[");
            hasher.update(&(items.len() as u64).to_le_bytes());
            for item in items {
                hash_json(hasher, item);
            }
            hasher.update(b"]");
        }
        other => {
            let rendered = other.to_string();
            hasher.update(&(rendered.len() as u64).to_le_bytes());
            hasher.update(rendered.as_bytes());
        }
    }
}

/// Truncates a finished digest to a memo key. 128 bits keeps an accidental
/// collision out of reach for any library size.
pub(crate) fn finish_key(hasher: &blake3::Hasher) -> LandedBarKey {
    let digest = hasher.finalize();
    let mut key = [0u8; 16];
    key.copy_from_slice(&digest.as_bytes()[..16]);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> LandedBarKey {
        [byte; 16]
    }

    #[test]
    fn a_filled_bar_is_returned_to_the_same_generation() {
        let memo = LandedBarMemo::default();
        let ticket = memo.ticket().expect("memo is on by default");
        assert_eq!(memo.get(ticket, &key(1)), None);
        memo.fill(ticket, key(1), 420);
        assert_eq!(memo.get(ticket, &key(1)), Some(420));
    }

    #[test]
    fn an_engine_swap_clears_the_memo_and_refuses_stale_tickets() {
        let memo = LandedBarMemo::default();
        let before = memo.ticket().unwrap();
        memo.fill(before, key(1), 420);

        memo.reset_for_engine(false);
        assert_eq!(memo.len(), 0);

        // A derivation that read the old engine finishes after the swap.
        memo.fill(before, key(2), 7);
        assert_eq!(memo.len(), 0, "a pre-swap result must not install");

        let after = memo.ticket().unwrap();
        assert_eq!(memo.get(after, &key(1)), None);
        assert_eq!(memo.get(before, &key(1)), None);
    }

    #[test]
    fn an_engine_that_reads_the_clock_turns_the_memo_off() {
        let memo = LandedBarMemo::default();
        memo.reset_for_engine(true);
        assert!(memo.ticket().is_none());

        memo.reset_for_engine(false);
        let ticket = memo.ticket().expect("a pure engine turns it back on");
        memo.fill(ticket, key(1), 1);
        assert_eq!(memo.get(ticket, &key(1)), Some(1));
    }

    #[test]
    fn a_completed_pass_drops_the_entries_it_did_not_touch() {
        let memo = LandedBarMemo::default();
        let first = memo.begin_pass();
        let ticket = memo.ticket().unwrap();
        memo.fill(ticket, key(1), 10);
        memo.fill(ticket, key(2), 20);
        memo.finish_pass(first);
        assert_eq!(memo.len(), 2);

        // The next pass only sees key 1: key 2's file is gone.
        let second = memo.begin_pass();
        let ticket = memo.ticket().unwrap();
        assert_eq!(memo.get(ticket, &key(1)), Some(10));
        let stats = memo.finish_pass(second);
        assert_eq!(memo.len(), 1);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.fills, 0);

        let ticket = memo.ticket().unwrap();
        assert_eq!(memo.get(ticket, &key(2)), None);
    }

    #[test]
    fn a_superseded_pass_sweeps_nothing() {
        let memo = LandedBarMemo::default();
        let stale = memo.begin_pass();
        let ticket = memo.ticket().unwrap();
        memo.fill(ticket, key(1), 10);
        let _current = memo.begin_pass();
        memo.finish_pass(stale);
        assert_eq!(memo.len(), 1);
    }

    #[test]
    fn json_hashing_ignores_map_order() {
        let a = serde_json::json!({"a": 1, "b": {"x": [1, 2], "y": "z"}});
        let b = serde_json::json!({"b": {"y": "z", "x": [1, 2]}, "a": 1});
        let c = serde_json::json!({"b": {"y": "z", "x": [2, 1]}, "a": 1});
        let digest = |value: &serde_json::Value| {
            let mut hasher = blake3::Hasher::new();
            hash_json(&mut hasher, value);
            finish_key(&hasher)
        };
        assert_eq!(digest(&a), digest(&b));
        assert_ne!(digest(&a), digest(&c), "array order is meaningful");
    }
}
