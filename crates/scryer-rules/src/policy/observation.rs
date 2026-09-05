//! Three-valued fact envelopes and the host-derived holds they justify.
//!
//! Every policy family that carries a fact snapshot uses the same envelope, the
//! same two input namespaces derived from it, and the same rule for deciding
//! that a policy must not be consulted at all. None of that is specific to
//! maintenance — it is specific to *facts Scryer may fail to observe* — so it
//! lives in the core where the next family inherits it rather than re-deriving
//! a subtly different version of it.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Status string an [`Observation`] serializes when Scryer could not find out.
pub(crate) const UNKNOWN_STATUS: &str = "unknown";
/// Status string an [`Observation`] serializes when it carries a value.
pub(crate) const KNOWN_STATUS: &str = "known";

/// Three-valued availability envelope wrapping every observable fact.
///
/// `Absent` means the source completed and confirmed there is no value.
/// `Unknown` means Scryer cannot know — the data is stale, unmapped,
/// unsupported, forbidden, or the lookup failed. An unknown fact is never
/// coerced to `false`, `0`, or `""`; rules that need certainty must test
/// `status` explicitly.
///
/// `Absent` may also carry a `reason`, using the same stable code vocabulary as
/// `Unknown`. It answers a different question — *why is there no value*, not
/// *why could Scryer not look* — and is optional, so an absence with nothing
/// useful to say still serializes without the field.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Observation<T: Serialize> {
    Known {
        value: T,
        #[serde(skip_serializing_if = "Option::is_none")]
        observed_at: Option<String>,
    },
    Absent {
        #[serde(skip_serializing_if = "Option::is_none")]
        observed_at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Unknown {
        reason: String,
    },
}

impl<T: Serialize> Observation<T> {
    /// A value Scryer observed, with no recorded observation time.
    pub fn known(value: T) -> Self {
        Self::Known {
            value,
            observed_at: None,
        }
    }

    /// A value Scryer observed at a specific RFC3339 timestamp.
    pub fn known_at(value: T, observed_at: impl Into<String>) -> Self {
        Self::Known {
            value,
            observed_at: Some(observed_at.into()),
        }
    }

    /// Confirmed absence: the source answered, and there is no value.
    pub fn absent() -> Self {
        Self::Absent {
            observed_at: None,
            reason: None,
        }
    }

    /// Confirmed absence, with the time the source answered.
    pub fn absent_at(observed_at: impl Into<String>) -> Self {
        Self::Absent {
            observed_at: Some(observed_at.into()),
            reason: None,
        }
    }

    /// Confirmed absence, carrying the stable machine code that explains why
    /// there is no value. Still an absence: the source answered.
    pub fn absent_because(reason: impl Into<String>) -> Self {
        Self::Absent {
            observed_at: None,
            reason: Some(reason.into()),
        }
    }

    /// Scryer cannot know the answer. `reason` is a stable machine code.
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self::Unknown {
            reason: reason.into(),
        }
    }
}

/// One serialized fact snapshot: the envelope map and the bare map derived
/// from it.
pub struct SerializedFacts {
    /// `input.observations` — every fact, envelope and all.
    pub observations: serde_json::Map<String, serde_json::Value>,
    /// `input.facts` — known facts only, unwrapped. An absent or unknown fact
    /// is a missing key, so `not input.facts.added_by_user_id` matches both a
    /// system-added title and one Scryer could not resolve; the engine deals
    /// with the second case before the rule ever runs.
    pub facts: serde_json::Map<String, serde_json::Value>,
}

/// Serialize a fact snapshot into both input namespaces.
///
/// Deriving `facts` from `observations` rather than storing both is the point —
/// the simple surface and the advanced one cannot drift apart.
pub fn serialize_fact_namespaces<T: Serialize + ?Sized>(
    facts: &T,
) -> Result<SerializedFacts, serde_json::Error> {
    let observations = match serde_json::to_value(facts)? {
        serde_json::Value::Object(map) => map,
        other => {
            return Err(serde::ser::Error::custom(format!(
                "fact snapshot must serialize to an object, got {other}"
            )));
        }
    };
    let facts = observations
        .iter()
        .filter_map(|(name, envelope)| {
            if envelope.get("status").and_then(serde_json::Value::as_str) != Some(KNOWN_STATUS) {
                return None;
            }
            let value = envelope.get("value")?;
            Some((name.clone(), value.clone()))
        })
        .collect();
    Ok(SerializedFacts {
        observations,
        facts,
    })
}

/// Facts this input's snapshot could not answer, mapped to the stable code
/// saying why.
///
/// Only `unknown` counts. An `absent` fact is an answer — the source replied
/// and there is nothing there — so a rule matching on the missing key is
/// deciding on real evidence and must not be held.
pub fn unobservable_facts(document: &serde_json::Value) -> BTreeMap<String, String> {
    let Some(observations) = document.get("observations").and_then(|obs| obs.as_object()) else {
        return BTreeMap::new();
    };

    observations
        .iter()
        .filter_map(|(name, envelope)| {
            if envelope.get("status").and_then(serde_json::Value::as_str) != Some(UNKNOWN_STATUS) {
                return None;
            }
            let reason = envelope
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(UNKNOWN_STATUS);
            Some((name.clone(), reason.to_string()))
        })
        .collect()
}

/// Reason codes explaining why a rule cannot be consulted, deduplicated and in
/// fact-name order. Empty when every fact the rule reads is observable, which
/// is the signal to evaluate it normally.
pub fn held_reason_codes(
    referenced_facts: &BTreeSet<String>,
    unobservable: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut codes: Vec<String> = Vec::new();
    for fact in referenced_facts {
        if let Some(reason) = unobservable.get(fact)
            && !codes.iter().any(|existing| existing == reason)
        {
            codes.push(reason.clone());
        }
    }
    codes
}
