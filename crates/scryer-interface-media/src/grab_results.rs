//! Outcomes of background grabs, held until the operator who started one reads
//! it back.
//!
//! A grab job reports how it ended on its job run, in words and as a reason
//! code. The typed payload, or the exact error the synchronous mutation would
//! have raised, is kept here so the client that started the grab can handle it
//! the way it handles the synchronous answer. Outcomes live in memory only: a
//! restart drops them, and the job run still says how the grab ended.

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

use crate::types::GrabJobResultPayload;

/// How many finished grabs are remembered for one user. Each is read within
/// moments of finishing, so this only bounds what an abandoned page leaves
/// behind. A user's newest outcome replaces only that user's oldest one.
const REMEMBERED_GRAB_RESULTS_PER_ACTOR: usize = 64;

/// How many finished grabs are remembered in all. When it is reached, an
/// outcome replaces its own user's oldest one, or is not kept when that user
/// holds none; it never replaces another user's outcome.
const REMEMBERED_GRAB_RESULTS: usize = 4096;

type GrabResult = Result<GrabJobResultPayload, async_graphql::Error>;

struct RememberedGrab {
    actor_user_id: String,
    result: GrabResult,
}

#[derive(Default)]
struct GrabResults {
    /// Each user's held run ids, oldest first.
    oldest_first_by_actor: HashMap<String, VecDeque<String>>,
    by_run_id: HashMap<String, RememberedGrab>,
}

impl GrabResults {
    fn remember(
        &mut self,
        run_id: &str,
        actor_user_id: &str,
        result: GrabResult,
        per_actor: usize,
        total: usize,
    ) {
        if let Some(held) = self.by_run_id.get_mut(run_id) {
            if held.actor_user_id == actor_user_id {
                held.result = result;
            }
            return;
        }
        let own_count = self
            .oldest_first_by_actor
            .get(actor_user_id)
            .map_or(0, VecDeque::len);
        let make_room = own_count >= per_actor || self.by_run_id.len() >= total;
        if make_room {
            let evicted = self
                .oldest_first_by_actor
                .get_mut(actor_user_id)
                .and_then(VecDeque::pop_front);
            match evicted {
                Some(evicted) => {
                    self.by_run_id.remove(&evicted);
                }
                // Full, and this user holds nothing to give up.
                None => return,
            }
        }
        self.by_run_id.insert(
            run_id.to_string(),
            RememberedGrab {
                actor_user_id: actor_user_id.to_string(),
                result,
            },
        );
        self.oldest_first_by_actor
            .entry(actor_user_id.to_string())
            .or_default()
            .push_back(run_id.to_string());
    }

    fn result_for(&self, run_id: &str, actor_user_id: &str) -> Option<GrabResult> {
        self.by_run_id
            .get(run_id)
            .filter(|remembered| remembered.actor_user_id == actor_user_id)
            .map(|remembered| remembered.result.clone())
    }
}

static GRAB_RESULTS: LazyLock<Mutex<GrabResults>> = LazyLock::new(Mutex::default);

/// Keep the outcome of the grab job `run_id` for the user who started it.
pub fn remember_grab_result(run_id: &str, actor_user_id: &str, result: GrabResult) {
    GRAB_RESULTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remember(
            run_id,
            actor_user_id,
            result,
            REMEMBERED_GRAB_RESULTS_PER_ACTOR,
            REMEMBERED_GRAB_RESULTS,
        );
}

/// The outcome of the grab job `run_id`, when one is held and `actor_user_id`
/// started it.
pub fn grab_result_for(run_id: &str, actor_user_id: &str) -> Option<GrabResult> {
    GRAB_RESULTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .result_for(run_id, actor_user_id)
}

#[cfg(test)]
mod tests {
    use async_graphql::ErrorExtensions;

    use super::*;

    fn run_id(name: &str) -> String {
        format!("grab-results-test-{name}-{}", std::process::id())
    }

    #[test]
    fn an_outcome_is_returned_only_to_the_user_who_started_the_grab() {
        let id = run_id("owner");
        remember_grab_result(&id, "user-a", Ok(GrabJobResultPayload::default()));

        assert!(grab_result_for(&id, "user-a").is_some());
        assert!(grab_result_for(&id, "user-b").is_none());
        assert!(grab_result_for(&run_id("never-started"), "user-a").is_none());
    }

    #[test]
    fn a_refusal_keeps_its_code_and_reason() {
        let id = run_id("refusal");
        let refusal = async_graphql::Error::new("validation: refused").extend_with(|_, e| {
            e.set("code", "VALIDATION_ERROR");
            e.set("reason", "EXAMPLE_REASON");
        });
        remember_grab_result(&id, "user-a", Err(refusal));

        let Some(Err(error)) = grab_result_for(&id, "user-a") else {
            panic!("the refusal is held");
        };
        let extensions = error.extensions.expect("extensions are kept");
        assert_eq!(
            extensions.get("reason"),
            Some(&async_graphql::Value::from("EXAMPLE_REASON"))
        );
    }

    #[test]
    fn one_user_filling_their_share_never_drops_another_users_outcome() {
        let mut results = GrabResults::default();
        results.remember("b-1", "user-b", Ok(GrabJobResultPayload::default()), 3, 100);

        for index in 0..10 {
            results.remember(
                &format!("a-{index}"),
                "user-a",
                Ok(GrabJobResultPayload::default()),
                3,
                100,
            );
        }

        assert!(results.result_for("b-1", "user-b").is_some());
        assert!(results.result_for("a-6", "user-a").is_none());
        for index in 7..10 {
            assert!(
                results
                    .result_for(&format!("a-{index}"), "user-a")
                    .is_some()
            );
        }
        assert_eq!(results.by_run_id.len(), 4);
    }

    #[test]
    fn a_full_store_never_drops_another_users_outcome() {
        let mut results = GrabResults::default();
        results.remember("a-1", "user-a", Ok(GrabJobResultPayload::default()), 10, 2);
        results.remember("b-1", "user-b", Ok(GrabJobResultPayload::default()), 10, 2);

        // The store is full: user C holds nothing to give up, so its outcome
        // is not kept; user A replaces only its own oldest outcome.
        results.remember("c-1", "user-c", Ok(GrabJobResultPayload::default()), 10, 2);
        results.remember("a-2", "user-a", Ok(GrabJobResultPayload::default()), 10, 2);

        assert!(results.result_for("c-1", "user-c").is_none());
        assert!(results.result_for("a-1", "user-a").is_none());
        assert!(results.result_for("a-2", "user-a").is_some());
        assert!(results.result_for("b-1", "user-b").is_some());
        assert_eq!(results.by_run_id.len(), 2);
    }
}
