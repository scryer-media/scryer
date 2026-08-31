use super::*;
use crate::acquisition::convergence::CoverageReopen;
use crate::acquisition::submission::{
    CanonicalDownloadSubmissionIntent, CanonicalDownloadSubmissionOutcome,
};
use crate::acquisition_decision_helpers::{
    FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES, extract_grabbed_release_title,
    is_download_submit_unavailable_error,
};
use crate::acquisition_release_search::{
    ReleaseAutoDecisionCode, annotate_auto_decision, automatic_candidate_delay_decision,
    serialize_decision_explanation,
};
use crate::contracts::{SubmissionConflictPolicy, SubmissionScopeConflict, WantedSearchOutcome};
use crate::domain_events::{
    new_global_domain_event, new_title_domain_event, title_context_snapshot,
};
use crate::types::{
    DecisionCodeCount, PendingRelease, PendingReleaseObservation, PendingReleaseRole,
    PendingReleaseStatus, PendingReleaseStatusCount, TitleAcquisitionDiagnostics,
    WantedStatusCount,
};
use crate::{JobKey, JobTriggerSource};
use chrono::{DateTime, Duration, Utc};
use futures_util::{StreamExt, stream::FuturesUnordered};
use scryer_domain::{
    DomainEventPayload, DomainEventStream, DownloadFailedEventData, Id, NewDomainEvent,
    ReleaseBlocklistedEventData, ReleaseGrabbedEventData,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};
use tracing::{debug, info, trace, warn};

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("wanted_sync.rs");
include!("client_snapshot.rs");
include!("decisions.rs");
include!("conflicts.rs");
include!("search.rs");
include!("public_api.rs");
include!("diagnostics.rs");
include!("task_runner.rs");
