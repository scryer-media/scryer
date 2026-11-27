//! Evaluating one bad-file report against the report rules, and recording why.
//!
//! A report rule decides a bad-file report with `mark_bad` or
//! `reject_report`. It runs through the request engine and records its trace
//! in the same table as a request decision, under a `request_id` of
//! `media_issue:<id>`, but only report rules run, the input carries
//! `input.report` instead of `input.request`, and the arbitration has no
//! permission fallback: without a rule's verdict, a person decides.
//!
//! A rule can only mark the file bad or reject the report. It never recycles
//! the file or starts a search; a reporter's request for those waits for an
//! operator.
//!
//! Like request evaluation, nothing here fails the caller. Anything that keeps
//! the rules from speaking leaves the report for review.

use chrono::{DateTime, Utc};
use scryer_domain::{Id, Library, MediaIssue, RequestRuleDecisionRecord, Title, User};
use scryer_rules::request::{
    Observation, REQUEST_INPUT_SCHEMA_VERSION, RequestClockDoc, RequestFactsDoc, RequestInput,
    RequestLibraryDoc, RequestReportDoc, RequestReportFactsDoc, RequestRequesterDoc,
};

use crate::AppUseCase;
use crate::helpers::{HashDomain, blake3_identity_hex};
use crate::request_rules::arbitration::{
    FALLBACK_ERROR, ReportArbitration, ReportVerdict, ScopedError, ScopedVote, arbitrate_report,
};
use crate::request_rules::evaluation::recorded_votes;
use crate::request_rules::facts::fact_reason;

/// Prefix on the `request_id` of a decision trace that explains a bad-file
/// report rather than a media request.
pub const MEDIA_ISSUE_DECISION_REQUEST_ID_PREFIX: &str = "media_issue:";

/// What the report rules concluded about a report.
pub type ReportRuleOutcome = ReportVerdict;

/// The report a rule judges, with what the host knows about the file.
pub struct MediaIssueReportSubject<'a> {
    pub issue: &'a MediaIssue,
    pub title: &'a Title,
    /// `grabbed` when the file came from a known grab, otherwise `unknown`.
    pub provenance: &'static str,
}

/// The verdict, and the trace that explains it when one was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRuleEvaluation {
    pub outcome: ReportRuleOutcome,
    pub decision_id: Option<String>,
}

impl ReportRuleEvaluation {
    fn review(decision_id: Option<String>) -> Self {
        Self {
            outcome: ReportVerdict::Review,
            decision_id,
        }
    }
}

/// The `input.report` section for one report.
pub(crate) fn report_doc(
    issue: &MediaIssue,
    provenance: &str,
    reporter_requested_media: Observation<bool>,
) -> RequestReportDoc {
    RequestReportDoc {
        reason: issue.reason.as_str().to_string(),
        remediation: issue.remediation.as_str().to_string(),
        provenance: provenance.to_string(),
        has_details: issue
            .details
            .as_deref()
            .is_some_and(|details| !details.trim().is_empty()),
        title_id: issue.title_id.clone(),
        media_file_id: issue.media_file_id.clone(),
        affected_episode_ids: issue.affected_episode_ids.clone(),
        affected_series_movie_link_ids: issue.affected_series_movie_link_ids.clone(),
        facts: RequestReportFactsDoc {
            reporter_requested_media,
        },
    }
}

/// The document a report rule evaluates. Every top-level fact is unknown, so a
/// rule that reads a title or requester fact holds the report for review.
pub(crate) fn build_report_input(
    evaluation_time: DateTime<Utc>,
    requester: RequestRequesterDoc,
    library: &Library,
    report: RequestReportDoc,
) -> RequestInput {
    RequestInput {
        schema_version: REQUEST_INPUT_SCHEMA_VERSION,
        evaluation_time,
        now: RequestClockDoc::at(evaluation_time),
        requester,
        library: RequestLibraryDoc {
            id: library.id.clone(),
            name: library.name.clone(),
            facet: library.facet.as_str().to_string(),
            is_default: library.is_default,
        },
        request: None,
        report: Some(report),
        facts: RequestFactsDoc::all_unknown(fact_reason::NOT_YET_COLLECTED),
    }
}

impl AppUseCase {
    /// Run the report rules covering the report's library and record the
    /// trace. With no report rule covering the library, the report waits for
    /// review and no trace is written: no rule took part.
    pub(crate) async fn evaluate_media_issue_report(
        &self,
        actor: &User,
        library: &Library,
        subject: &MediaIssueReportSubject<'_>,
    ) -> ReportRuleEvaluation {
        let issue = subject.issue;
        let cache = self.request_rules_engine_snapshot();
        if cache.unavailable {
            tracing::warn!(
                media_issue_id = issue.id.as_str(),
                "request rules engine generation is unavailable; the report waits for review"
            );
            return self.record_report_error_fallback(issue).await;
        }
        let consulted: Vec<(String, crate::request_rules::engine::RequestRuleScope)> = cache
            .report_scopes_for_library(&library.id)
            .into_iter()
            .map(|(id, scope)| (id.clone(), scope.clone()))
            .collect();
        if consulted.is_empty() {
            return ReportRuleEvaluation::review(None);
        }

        let evaluation_time = Utc::now();
        let requester = match self
            .request_requester_doc(actor, &library.id, evaluation_time)
            .await
        {
            Ok(requester) => requester,
            Err(error) => {
                tracing::warn!(
                    media_issue_id = issue.id.as_str(),
                    error = %error,
                    "could not read the reporter for report rules; the report waits for review"
                );
                return self.record_report_error_fallback(issue).await;
            }
        };
        // Read only when a loaded report rule can read it.
        let reporter_requested_media = if cache.report_reads_requester_relationship {
            crate::media_issues::reporter_requested_affected_media(
                self,
                actor,
                subject.title,
                &issue.affected_episode_ids,
                &issue.affected_series_movie_link_ids,
            )
            .await
        } else {
            Observation::unknown(fact_reason::NOT_YET_COLLECTED)
        };
        let input = build_report_input(
            evaluation_time,
            requester,
            library,
            report_doc(issue, subject.provenance, reporter_requested_media),
        );
        let input_hash = blake3_identity_hex(
            HashDomain::RequestRuleInput,
            serde_json::to_string(&input).unwrap_or_default(),
        );

        let mut evaluator = cache.engine.evaluator();
        let outcome = match evaluator.evaluate_in_scope(&input, |rule_set_id| {
            cache
                .scopes
                .get(rule_set_id)
                .is_some_and(|scope| scope.covers_report(&library.id))
        }) {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(
                    media_issue_id = issue.id.as_str(),
                    error = %error,
                    "report rule evaluation failed; the report waits for review"
                );
                return self.record_report_error_fallback(issue).await;
            }
        };

        let mut votes: Vec<ScopedVote> = Vec::new();
        for record in &outcome.records {
            let Some(scope) = cache.scopes.get(&record.rule_set_id) else {
                continue;
            };
            if !scope.covers_report(&library.id) {
                continue;
            }
            votes.push(ScopedVote {
                rule_set_id: record.rule_set_id.clone(),
                rule_set_name: record.rule_set_name.clone(),
                revision_number: scope.revision_number,
                content_hash: scope.content_hash.clone(),
                decision: record.decision.clone(),
            });
        }
        let mut errors: Vec<ScopedError> = Vec::new();
        for (rule_set_id, scope) in &consulted {
            if let Some(message) = cache.load_errors.get(rule_set_id) {
                errors.push(ScopedError {
                    rule_set_id: rule_set_id.clone(),
                    rule_set_name: scope.name.clone(),
                    revision_number: scope.revision_number,
                    content_hash: scope.content_hash.clone(),
                    message: message.clone(),
                });
            }
        }
        for failure in &outcome.errors {
            let Some(scope) = cache.scopes.get(&failure.rule_set_id) else {
                continue;
            };
            if !scope.covers_report(&library.id) {
                continue;
            }
            errors.push(ScopedError {
                rule_set_id: failure.rule_set_id.clone(),
                rule_set_name: failure.rule_set_name.clone(),
                revision_number: scope.revision_number,
                content_hash: scope.content_hash.clone(),
                message: failure.message.clone(),
            });
        }

        let arbitration = arbitrate_report(&votes, &errors);
        let decision_id = self
            .record_report_decision(issue, &arbitration, &votes, &errors, &input_hash)
            .await;
        ReportRuleEvaluation {
            outcome: arbitration.verdict,
            decision_id,
        }
    }

    /// The trace write, best effort: a lost trace must not change the verdict.
    async fn record_report_decision(
        &self,
        issue: &MediaIssue,
        arbitration: &ReportArbitration,
        votes: &[ScopedVote],
        errors: &[ScopedError],
        input_hash: &str,
    ) -> Option<String> {
        let now = Utc::now();
        let outcome = arbitration.outcome();
        let record = RequestRuleDecisionRecord {
            id: Id::new().0,
            request_id: format!("{MEDIA_ISSUE_DECISION_REQUEST_ID_PREFIX}{}", issue.id),
            evaluated_at: now,
            effective_outcome: outcome,
            policy_outcome: outcome,
            fallback_reason: arbitration.fallback_reason.map(str::to_string),
            votes_json: serde_json::to_string(&recorded_votes(votes, errors))
                .unwrap_or_else(|_| "[]".to_string()),
            tags: Vec::new(),
            shaping_json: None,
            input_hash: input_hash.to_string(),
            input_schema_version: i64::from(REQUEST_INPUT_SCHEMA_VERSION),
            created_at: now,
        };
        match self
            .services
            .customization
            .request_rule_decisions
            .record(&record)
            .await
        {
            Ok(()) => Some(record.id),
            Err(error) => {
                tracing::warn!(
                    media_issue_id = issue.id.as_str(),
                    error = %error,
                    "could not record a report rule decision trace"
                );
                None
            }
        }
    }

    /// The report rules could not be consulted: the report waits for review,
    /// and the trace says policy never got to speak.
    async fn record_report_error_fallback(&self, issue: &MediaIssue) -> ReportRuleEvaluation {
        let arbitration = ReportArbitration {
            verdict: ReportVerdict::Review,
            fallback_reason: Some(FALLBACK_ERROR),
            deciding_rule_set_ids: Vec::new(),
            reasons: Vec::new(),
        };
        let decision_id = self
            .record_report_decision(issue, &arbitration, &[], &[], "")
            .await;
        ReportRuleEvaluation::review(decision_id)
    }
}
