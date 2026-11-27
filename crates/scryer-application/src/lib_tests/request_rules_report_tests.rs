//! Report rules: a rule decides media requests or bad-file reports, never
//! both, and each kind runs only against its own subject.

use super::*;
use crate::request_rules::arbitration::{
    FALLBACK_REPORT_CONFLICT, ReportVerdict, arbitrate_report,
};
use crate::request_rules::{MEDIA_ISSUE_DECISION_REQUEST_ID_PREFIX, MediaIssueReportSubject};
use scryer_rules::request::RequestReportVote;

const MARK_BAD_CORRUPTED: &str = r#"package rules
import rego.v1

mark_bad if {
	input.report.reason == "corrupted"
}
"#;

const REJECT_CORRUPTED: &str = r#"package rules
import rego.v1

reject_report if {
	input.report.reason == "corrupted"
}
"#;

const MIXED: &str = r#"package rules
import rego.v1

approve if {
	input.request.origin == "manual"
}

mark_bad if {
	input.report.reason == "corrupted"
}
"#;

fn report_vote(id: &str, report_vote: RequestReportVote) -> ScopedVote {
    let mut scoped = vote(id, RequestVote::Abstain, false, &[]);
    scoped.decision.report_vote = report_vote;
    scoped
}

fn sample_issue(title: &Title) -> scryer_domain::MediaIssue {
    scryer_domain::MediaIssue {
        id: "issue-fixture".to_string(),
        library_id: title.library_id.clone(),
        title_id: title.id.clone(),
        media_file_id: "media-file-fixture".to_string(),
        file_path_snapshot: "/data/test/Fixture Title (2001)/fixture.mkv".to_string(),
        file_version: "1:2".to_string(),
        content: scryer_domain::BlocklistContentIdentity {
            scheme: crate::fs_integrity::IMPORT_CONTENT_PROOF_SCHEME.to_string(),
            size_bytes: 4096,
            sample_bytes: 4096,
            sample_blake3: "00".repeat(32),
        },
        reporter_user_id: "user-fixture".to_string(),
        reason: scryer_domain::MediaIssueReason::Corrupted,
        details: Some("stops halfway".to_string()),
        remediation: scryer_domain::MediaIssueRemediation::RecycleAndSearch,
        authorized_remediation: None,
        status: scryer_domain::MediaIssueStatus::Pending,
        affected_episode_ids: Vec::new(),
        affected_series_movie_link_ids: Vec::new(),
        rule_decision_id: None,
        blocklist_id: None,
        reviewed_by_user_id: None,
        reviewed_at: None,
        execution_status: scryer_domain::MediaIssueExecutionStatus::None,
        execution_error_code: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

async fn evaluate_report(
    harness: &MediaRequestTestHarness,
) -> crate::request_rules::ReportRuleEvaluation {
    let title = make_due_hydration_title("title-fixture", MediaFacet::Movie, 9401);
    let library = harness
        .app
        .services
        .catalog
        .libraries
        .get_by_id(&title.library_id)
        .await
        .expect("library read")
        .expect("default movie library exists");
    let issue = sample_issue(&title);
    harness
        .app
        .evaluate_media_issue_report(
            &harness.user,
            &library,
            &MediaIssueReportSubject {
                issue: &issue,
                title: &title,
                provenance: "grabbed",
            },
        )
        .await
}

async fn report_traces(
    harness: &MediaRequestTestHarness,
) -> Vec<scryer_domain::RequestRuleDecisionRecord> {
    harness
        .request_rule_decisions
        .recorded()
        .await
        .into_iter()
        .filter(|record| {
            record
                .request_id
                .starts_with(MEDIA_ISSUE_DECISION_REQUEST_ID_PREFIX)
        })
        .collect()
}

// ── Arbitration (pure) ──────────────────────────────────────────────────────

#[test]
fn report_arbitration_follows_its_table() {
    let mark = report_vote("m", RequestReportVote::MarkBad);
    let reject = report_vote("r", RequestReportVote::Reject);
    let both = report_vote("b", RequestReportVote::Conflict);
    let silent = report_vote("s", RequestReportVote::Abstain);
    let held = vote("h", RequestVote::Manual, true, &[]);

    assert_eq!(
        arbitrate_report(&[mark.clone()], &[]).verdict,
        ReportVerdict::MarkBad
    );
    assert_eq!(
        arbitrate_report(&[reject.clone()], &[]).verdict,
        ReportVerdict::Reject
    );

    let disagreement = arbitrate_report(&[mark.clone(), reject.clone()], &[]);
    assert_eq!(disagreement.verdict, ReportVerdict::Review);
    assert_eq!(disagreement.fallback_reason, Some(FALLBACK_REPORT_CONFLICT));
    assert_eq!(disagreement.deciding_rule_set_ids, vec!["m", "r"]);

    let one_rule_both = arbitrate_report(&[both], &[]);
    assert_eq!(one_rule_both.verdict, ReportVerdict::Review);
    assert_eq!(
        one_rule_both.fallback_reason,
        Some(FALLBACK_REPORT_CONFLICT)
    );

    let nothing = arbitrate_report(&[silent], &[]);
    assert_eq!(nothing.verdict, ReportVerdict::Review);
    assert_eq!(nothing.fallback_reason, Some(FALLBACK_NO_RULE_MATCHED));

    let held_out_votes_mark = arbitrate_report(&[mark.clone(), held], &[]);
    assert_eq!(held_out_votes_mark.verdict, ReportVerdict::Review);
    assert_eq!(held_out_votes_mark.fallback_reason, Some(FALLBACK_HELD));

    let failed = ScopedError {
        rule_set_id: "e".to_string(),
        rule_set_name: "rule e".to_string(),
        revision_number: 1,
        content_hash: "hash-e".to_string(),
        message: "boom".to_string(),
    };
    let errored = arbitrate_report(&[mark], &[failed]);
    assert_eq!(errored.verdict, ReportVerdict::Review);
    assert_eq!(errored.fallback_reason, Some(FALLBACK_ERROR));
}

// ── The flow ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_approve_all_request_rule_leaves_a_report_in_review() {
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Approve everything", APPROVE_EVERYTHING).await;
    enable(&harness, &detail.rule_set.id).await;

    let evaluation = evaluate_report(&harness).await;
    assert_eq!(evaluation.outcome, ReportVerdict::Review);
    // No report rule covers the library, so none took part and no trace
    // explains the report.
    assert_eq!(evaluation.decision_id, None);
    assert!(report_traces(&harness).await.is_empty());
}

#[tokio::test]
async fn a_mark_bad_rule_does_not_touch_a_media_request() {
    let baseline = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let baseline_id = submit(&baseline, &library_id, 9402, Some(30)).await;
    let baseline_status = baseline
        .media_requests
        .requests
        .lock()
        .await
        .iter()
        .find(|request| request.id == baseline_id)
        .expect("request stored")
        .status;

    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Mark corrupted files bad", MARK_BAD_CORRUPTED).await;
    enable(&harness, &detail.rule_set.id).await;
    let request_id = submit(&harness, &library_id, 9402, Some(30)).await;
    let status = harness
        .media_requests
        .requests
        .lock()
        .await
        .iter()
        .find(|request| request.id == request_id)
        .expect("request stored")
        .status;

    assert_eq!(status, baseline_status);
    // The report rule never ran for the request, so no trace names it.
    assert!(
        harness
            .request_rule_decisions
            .recorded()
            .await
            .iter()
            .all(|record| !record.votes_json.contains(&detail.rule_set.id)),
    );
}

#[tokio::test]
async fn a_rule_mixing_report_and_request_heads_is_refused_at_save() {
    let harness = bootstrap_media_request_app();
    let refusal = create_rule_result(&harness, "Mixed", MIXED)
        .await
        .expect_err("a rule decides requests or reports, not both");
    assert!(
        diagnostic_codes(&refusal)
            .contains(&scryer_rules::diagnostic_codes::RULE_MIXES_REPORT_AND_REQUEST_HEADS),
        "{refusal:?}"
    );
}

#[tokio::test]
async fn report_rules_that_disagree_send_the_report_to_review() {
    let harness = bootstrap_media_request_app();
    let mark = create_rule(&harness, "Mark corrupted files bad", MARK_BAD_CORRUPTED).await;
    let reject = create_rule(&harness, "Reject corrupted reports", REJECT_CORRUPTED).await;
    enable(&harness, &mark.rule_set.id).await;

    let marked = evaluate_report(&harness).await;
    assert_eq!(marked.outcome, ReportVerdict::MarkBad);
    assert!(marked.decision_id.is_some());

    enable(&harness, &reject.rule_set.id).await;
    let disputed = evaluate_report(&harness).await;
    assert_eq!(disputed.outcome, ReportVerdict::Review);

    let traces = report_traces(&harness).await;
    assert_eq!(traces.len(), 2);
    let disputed_trace = traces
        .iter()
        .find(|trace| Some(&trace.id) == disputed.decision_id.as_ref())
        .expect("the disputed report is traced");
    assert_eq!(
        disputed_trace.request_id,
        format!("{MEDIA_ISSUE_DECISION_REQUEST_ID_PREFIX}issue-fixture")
    );
    assert_eq!(
        disputed_trace.fallback_reason.as_deref(),
        Some(FALLBACK_REPORT_CONFLICT)
    );
    assert_eq!(
        disputed_trace.effective_outcome,
        RequestDecisionOutcome::ManualReview
    );
}

#[tokio::test]
async fn a_report_rule_reading_a_title_fact_holds_the_report() {
    const READS_TITLE_FACT: &str = r#"package rules
import rego.v1

mark_bad if {
	input.facts.is_adult == false
}
"#;
    let harness = bootstrap_media_request_app();
    let detail = create_rule(&harness, "Reads a title fact", READS_TITLE_FACT).await;
    enable(&harness, &detail.rule_set.id).await;

    let evaluation = evaluate_report(&harness).await;
    assert_eq!(evaluation.outcome, ReportVerdict::Review);
    let traces = report_traces(&harness).await;
    assert_eq!(traces[0].fallback_reason.as_deref(), Some(FALLBACK_HELD));
}
