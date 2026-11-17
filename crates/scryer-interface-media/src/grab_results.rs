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

/// How many finished grabs are remembered. Each is read within moments of
/// finishing, so this only bounds what an abandoned page leaves behind.
const REMEMBERED_GRAB_RESULTS: usize = 512;

type GrabResult = Result<GrabJobResultPayload, async_graphql::Error>;

struct RememberedGrab {
    actor_user_id: String,
    result: GrabResult,
}

#[derive(Default)]
struct GrabResults {
    oldest_first: VecDeque<String>,
    by_run_id: HashMap<String, RememberedGrab>,
}

static GRAB_RESULTS: LazyLock<Mutex<GrabResults>> = LazyLock::new(Mutex::default);

/// Keep the outcome of the grab job `run_id` for the user who started it.
pub fn remember_grab_result(run_id: &str, actor_user_id: &str, result: GrabResult) {
    let mut results = GRAB_RESULTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if results
        .by_run_id
        .insert(
            run_id.to_string(),
            RememberedGrab {
                actor_user_id: actor_user_id.to_string(),
                result,
            },
        )
        .is_none()
    {
        results.oldest_first.push_back(run_id.to_string());
    }
    while results.oldest_first.len() > REMEMBERED_GRAB_RESULTS {
        if let Some(evicted) = results.oldest_first.pop_front() {
            results.by_run_id.remove(&evicted);
        }
    }
}

/// The outcome of the grab job `run_id`, when one is held and `actor_user_id`
/// started it.
pub fn grab_result_for(run_id: &str, actor_user_id: &str) -> Option<GrabResult> {
    let results = GRAB_RESULTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    results
        .by_run_id
        .get(run_id)
        .filter(|remembered| remembered.actor_user_id == actor_user_id)
        .map(|remembered| remembered.result.clone())
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
}
