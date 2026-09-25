//! The search lane's release-name prefetch.
//!
//! A search evaluates a batch of releases against one subject, and the
//! collision guard has to see what each release is named. It used to fetch
//! every release name in all three facets; it now fetches only the names that
//! can reach the guard, in the subject's facet. That is sound only while the
//! filter and the matcher agree, so these pin the two against each other: the
//! selective index answers every release exactly as an index holding the whole
//! library does, and a guard check for a name nobody fetched is refused.

use super::*;
use crate::acquisition::release_search::{
    TitleIdentityAmbiguity, canonical_title_evidence, match_parsed_release_to_title_evidence,
};
use crate::import_title_resolution::MonitoredTitleMatcher;
use crate::title_matching::relaxed::{
    SpellingCandidates, SpellingIdentity, neutral_spelling_forms,
};

const RELEASES: &[&str] = &[
    "The.Silver.Harbor.2019.1080p.WEB.H265-GRP",
    "The.Silver.Harbour.2019.1080p.WEB.H265-GRP",
    "The.Silver.Harboar.2019.1080p.WEB.H265-GRP",
    "Amber.Meadow.Lantern.2019.1080p.WEB.H265-GRP",
    "Quorrel.Vane.Chronicle.2019.1080p.WEB.H265-GRP",
    "Quorrel.Vane.Chronicel.2019.1080p.WEB.H265-GRP",
    // One lookup key, two numbers guards: shouted, `MIX` reads as a Roman
    // numeral. Each reaches a different name of the same subject.
    "The.Midnight.Mix.Chronicel.2019.1080p.WEB.H265-GRP",
    "THE.MIDNIGHT.MIX.CHRONICEL.2019.1080p.WEB.H265-GRP",
];

async fn add_movie(app: &AppUseCase, user: &User, name: &str, monitored: bool) -> Title {
    app.add_title(
        user,
        NewTitle {
            name: name.into(),
            facet: MediaFacet::Movie,
            monitored,
            year: Some(2019),
            ..Default::default()
        },
    )
    .await
    .expect("create title")
}

/// Three subjects, a near-spelled rival of one of them, a lookalike in another
/// facet, and an unrelated title. The third subject carries a shouted alias,
/// whose `MIX` the numbers guard reads as a Roman numeral.
async fn library() -> (AppUseCase, Vec<Title>, Vec<Title>) {
    let (app, user) = bootstrap();
    let harbor = add_movie(&app, &user, "The Silver Harbor", true).await;
    let rival = add_movie(&app, &user, "The Silver Harbour", false).await;
    let quorrel = add_movie(&app, &user, "Quorrel Vane Chronicle", true).await;
    let unrelated = add_movie(&app, &user, "Amber Meadow Lantern", true).await;
    let mix = add_movie(&app, &user, "The Midnight Mix Chronicle", true).await;
    app.services
        .catalog
        .titles
        .update_title_hydrated_metadata(
            &mix.id,
            TitleMetadataUpdate {
                aliases: vec!["THE MIDNIGHT MIX CHRONICLA".to_string()],
                year: Some(2019),
                ..Default::default()
            },
        )
        .await
        .expect("store the alias");
    let mix = app
        .services
        .catalog
        .titles
        .get_by_id(&mix.id)
        .await
        .expect("reload the title")
        .expect("the title exists");
    let lookalike = app
        .add_title(
            &user,
            NewTitle {
                name: "Quorrel Vane Chronicel".into(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2019),
                ..Default::default()
            },
        )
        .await
        .expect("create series");
    let subjects = vec![harbor.clone(), quorrel.clone(), mix.clone()];
    (
        app,
        subjects,
        vec![harbor, rival, quorrel, unrelated, lookalike, mix],
    )
}

fn batch_anchors(releases: &[&str]) -> Vec<(String, String)> {
    crate::title_matching::relaxed::release_batch_anchors(releases.iter().copied())
}

/// The matched key, and the spelling lane's (observed, distance, exact) when
/// that lane made the match.
type Outcome = Option<(String, Option<(String, usize, bool)>)>;

fn outcome(raw: &str, subject: &Title, index: SpellingCandidates) -> Outcome {
    let evidence = canonical_title_evidence(subject).with_ambiguity(
        TitleIdentityAmbiguity::default().with_spelling_candidates(Arc::new(index)),
    );
    match_parsed_release_to_title_evidence(&crate::parse_release_metadata(raw), &evidence).map(
        |matched| {
            (
                matched.matched_key,
                matched
                    .spelling
                    .map(|spelling| (spelling.observed, spelling.distance, spelling.exact)),
            )
        },
    )
}

#[tokio::test]
async fn a_selective_prefetch_answers_every_release_as_the_whole_library_does() {
    let (app, subjects, all) = library().await;
    let matcher = MonitoredTitleMatcher::new(app.services.catalog.titles.clone());
    let anchors = batch_anchors(RELEASES);

    for subject in &subjects {
        let identity = SpellingIdentity::new(subject);
        let mut selective = matcher
            .title_spelling_candidates(subject)
            .await
            .expect("subject candidates")
            .as_ref()
            .clone();
        let before = selective.fetched_anchor_count();
        matcher
            .extend_spelling_candidates_for_collisions(&mut selective, &anchors, &identity)
            .await
            .expect("prefetch");
        // One fetch per release name that can reach this subject's guard, in
        // this subject's facet only — never one per name per facet.
        let reachable = anchors
            .iter()
            .filter(|(observed, raw)| {
                crate::title_matching::relaxed::anchor_reaches_collision_check(
                    observed, raw, &identity,
                )
            })
            .count();
        assert!(
            reachable < anchors.len(),
            "{}: nothing was skipped",
            subject.name
        );
        assert_eq!(selective.fetched_anchor_count() - before, reachable);

        for raw in RELEASES {
            // The prefetch reads `candidate.title`; the matcher reads the
            // parsed release's `raw_title`. They must name the same anchors,
            // or the guard would be asked about a name nobody fetched.
            assert_eq!(
                neutral_spelling_forms(raw).0,
                neutral_spelling_forms(&crate::parse_release_metadata(raw).raw_title).0,
                "{raw}"
            );
            assert_eq!(
                outcome(raw, subject, selective.clone()),
                outcome(raw, subject, SpellingCandidates::from_titles(&all)),
                "{}: {raw}",
                subject.name
            );
        }
    }

    // Non-vacuous: the exact name and a lone typo match, the rival and the
    // near tie are refused.
    let [harbor, quorrel, mix] = subjects.as_slice() else {
        unreachable!()
    };
    let whole = || SpellingCandidates::from_titles(&all);
    assert!(outcome(RELEASES[0], harbor, whole()).is_some());
    assert!(outcome(RELEASES[1], harbor, whole()).is_none());
    assert!(outcome(RELEASES[2], harbor, whole()).is_none());
    assert!(outcome(RELEASES[3], harbor, whole()).is_none());
    assert!(
        outcome(RELEASES[5], quorrel, whole())
            .and_then(|(_, spelling)| spelling)
            .is_some_and(|(_, distance, exact)| distance > 0 && !exact)
    );
    // Both casings match, each through its own name, so the batch has to
    // have fetched both.
    assert_ne!(
        scryer_domain::title_spelling::title_numbers_key("The Midnight Mix Chronicel"),
        scryer_domain::title_spelling::title_numbers_key("THE MIDNIGHT MIX CHRONICEL"),
    );
    assert!(outcome(RELEASES[6], mix, whole()).is_some());
    assert!(outcome(RELEASES[7], mix, whole()).is_some());
}

#[tokio::test]
async fn a_selective_index_refuses_a_guard_check_it_never_fetched() {
    let (app, subjects, _) = library().await;
    let quorrel = &subjects[1];
    let identity = SpellingIdentity::new(quorrel);
    let matcher = MonitoredTitleMatcher::new(app.services.catalog.titles.clone());
    let base = matcher
        .title_spelling_candidates(quorrel)
        .await
        .expect("subject candidates")
        .as_ref()
        .clone();
    // Only the exact release was in the batch, so the typo was never fetched.
    let exact_only = batch_anchors(&[RELEASES[4]]);
    let typo = RELEASES[5];

    let mut selective = base.clone();
    matcher
        .extend_spelling_candidates_for_collisions(&mut selective, &exact_only, &identity)
        .await
        .expect("prefetch");
    assert!(outcome(typo, quorrel, selective).is_none());

    // The same index fetched the typo's neighbours, and the typo matches.
    let mut fetched = base;
    matcher
        .extend_spelling_candidates_for_collisions(&mut fetched, &batch_anchors(&[typo]), &identity)
        .await
        .expect("prefetch");
    assert!(outcome(typo, quorrel, fetched).is_some());
}
