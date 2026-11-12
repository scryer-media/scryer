use super::super::{
    PlannedTitleFolderOverlap, RenamePlan, build_rename_plan_from_items,
    planned_title_folder_overlap, title_folder_overlap_in,
};
use super::*;
use scryer_domain::Title;
use std::sync::atomic::Ordering;

fn stored(path: &Path) -> String {
    path_to_stored_string(path)
}

fn folder(title_id: &str, path: &Path) -> RenamePlanTitleFolder {
    RenamePlanTitleFolder {
        title_id: title_id.to_string(),
        folder_path: stored(path),
    }
}

/// How a library root probes, the one fact a recorded folder's case comes
/// from.
fn root_case_insensitive(root: &Path) -> bool {
    folder_case_insensitive(&resolved_folder_identity(root).unwrap())
}

/// The spelling a lone resolution of `raw` gives, with its root's case.
fn lone_spelling(raw: &str, root: &Path) -> String {
    comparison_spelling(&resolved_on_its_own(raw), root_case_insensitive(root))
}

#[test]
fn recorded_folders_are_spelled_from_one_listing_per_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    std::fs::create_dir_all(root.join("Alder Vale")).unwrap();
    std::fs::create_dir_all(root.join("Organised").join("Birch Hollow")).unwrap();
    let roots = vec![stored(&root)];
    let mut recorded = vec![
        root.join("Alder Vale"),
        root.join("Cedar Reach"),
        root.join("Organised").join("Birch Hollow"),
        root.join("Organised").join("Dune Field"),
        root.join("Not Yet Made").join("Elm Rise"),
    ];
    // Folders that are not on disk at all still take their spelling from
    // the listing of the directory that would hold them.
    recorded.extend((0..64).map(|index| root.join(format!("Absent Title {index}"))));

    let mut facts = DiskFacts::default();
    for path in &recorded {
        let raw = stored(path);
        assert_eq!(
            facts.standing_spelling(&raw, &roots),
            lone_spelling(&raw, &root),
            "{raw}"
        );
    }
    let expected = DiskCounts {
        root_probes: 1,
        parent_reads: 3,
        folder_resolutions: 0,
    };
    assert_eq!(facts.counts, expected);

    // Asking again is answered from what was learned.
    for path in &recorded {
        facts.standing_spelling(&stored(path), &roots);
    }
    assert_eq!(facts.counts, expected);
}

#[test]
fn folders_the_listing_cannot_settle_are_resolved_on_their_own() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(root.join("Gorse Bank")).unwrap();
    std::fs::create_dir_all(outside.join("Heath Moor")).unwrap();
    std::fs::write(root.join("Ivy Lane.txt"), b"synthetic").unwrap();
    #[cfg(unix)]
    {
        std::fs::create_dir_all(outside.join("Fir Target")).unwrap();
        std::os::unix::fs::symlink(outside.join("Fir Target"), root.join("Fir Link")).unwrap();
    }
    let roots = vec![stored(&root)];
    let mut facts = DiskFacts::default();

    // Matched only with another letter case.
    let recased = stored(&root.join("gorse bank"));
    assert_eq!(
        facts.standing_spelling(&recased, &roots),
        lone_spelling(&recased, &root)
    );
    // A file, not a folder.
    let file = stored(&root.join("Ivy Lane.txt"));
    assert_eq!(
        facts.standing_spelling(&file, &roots),
        lone_spelling(&file, &root)
    );
    // Under no library root: spelled exactly as before, its own case too.
    let stray = stored(&outside.join("Heath Moor"));
    assert_eq!(
        facts.standing_spelling(&stray, &roots),
        per_folder_spelling(&stray)
    );
    let resolved_alone = 3 + usize::from(cfg!(unix));

    #[cfg(unix)]
    {
        // A title folder that is a link takes the spelling of its target.
        let linked = stored(&root.join("Fir Link"));
        let spelling = facts.standing_spelling(&linked, &roots);
        assert_eq!(spelling, lone_spelling(&linked, &root));
        assert!(
            spelling.to_lowercase().ends_with("fir target"),
            "{spelling}"
        );
    }

    assert_eq!(facts.counts.folder_resolutions, resolved_alone);
    assert_eq!(facts.counts.root_probes, 1);
}

async fn movie_library_with_roots(app: &AppUseCase, roots: &[&Path]) -> String {
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let library = app
        .services
        .catalog
        .libraries
        .get_by_id(&library_id)
        .await
        .unwrap()
        .unwrap();
    app.services
        .catalog
        .libraries
        .update(
            &library_id,
            library.name,
            library.slug,
            roots
                .iter()
                .enumerate()
                .map(|(index, root)| crate::LibraryRootDraft {
                    id: None,
                    path: stored(root),
                    is_default: index == 0,
                })
                .collect(),
        )
        .await
        .unwrap();
    library_id
}

async fn movie(
    app: &AppUseCase,
    actor: &scryer_domain::User,
    library_id: &str,
    name: &str,
    folder: Option<&Path>,
) -> Title {
    let title = app
        .create_title_without_hydration_in_library(
            actor,
            crate::NewTitle {
                name: name.into(),
                facet: MediaFacet::Movie,
                ..Default::default()
            },
            library_id.to_owned(),
        )
        .await
        .unwrap()
        .title;
    if let Some(folder) = folder {
        set_folder(app, &title, folder).await;
    }
    title
}

async fn set_folder(app: &AppUseCase, title: &Title, folder: &Path) {
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &stored(folder))
        .await
        .unwrap();
}

/// A plan that puts files of `title` in `folder`.
fn plan_moving(title: &Title, folder_path: &Path) -> RenamePlan {
    let mut plan = build_rename_plan_from_items(
        MediaFacet::Movie,
        Some(title.id.clone()),
        String::new(),
        Default::default(),
        Default::default(),
        Vec::new(),
    );
    let planned = folder(&title.id, folder_path);
    plan.title_folders = vec![planned.clone()];
    plan.folder_repairs = vec![planned];
    plan
}

fn describe(overlap: Option<PlannedTitleFolderOverlap<'_>>) -> Option<(String, String, bool)> {
    overlap.map(|overlap| {
        (
            overlap.outer.folder.title_id.clone(),
            overlap.inner.folder.title_id.clone(),
            overlap.nested,
        )
    })
}

/// The pair the guard's overlap step finds, by title id, through `context`
/// and through the lone-resolution spellings, which must agree.
async fn overlap_step(
    app: &AppUseCase,
    context: &RenameOverlapContext,
    plan: &RenamePlan,
) -> Option<(String, String, bool)> {
    let recorded = context
        .recorded_title_folders(app, &MediaFacet::Movie)
        .await
        .unwrap();
    let roots = app
        .all_library_root_folders()
        .await
        .unwrap()
        .into_iter()
        .map(|root| root.path)
        .collect::<Vec<_>>();
    let spellings = context
        .comparison_spellings(&plan.title_folders, &recorded, roots)
        .await;
    let standing = recorded
        .iter()
        .filter(|folder| folder.title_id != plan.title_folders[0].title_id)
        .map(|folder| RenamePlanTitleFolder {
            title_id: folder.title_id.clone(),
            folder_path: folder.folder_path.clone(),
        })
        .collect::<Vec<_>>();
    let shared = describe(title_folder_overlap_in(
        &plan.title_folders,
        &standing,
        &spellings,
    ));
    let lone = describe(planned_title_folder_overlap(&plan.title_folders, &standing));
    assert_eq!(shared, lone, "shared facts and lone resolution disagree");
    shared
}

fn refusal(result: AppResult<()>) -> Option<(Option<&'static str>, String)> {
    result
        .err()
        .map(|error| (error.validation_reason(), error.to_string()))
}

#[tokio::test]
async fn the_guard_decides_as_lone_resolution_does() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(root.join("Juniper Court")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let (app, actor, _) = crate::lib_tests::bootstrap_with_title_repo();
    let library_id = movie_library_with_roots(&app, &[&root]).await;
    let idle = movie(&app, &actor, &library_id, "Idle Feature", None).await;
    let mover = movie(&app, &actor, &library_id, "Moving Feature", None).await;
    let overlap = Some(crate::error_reasons::library::RENAME_FOLDERS_OVERLAP);

    // The same folder, which exists.
    set_folder(&app, &idle, &root.join("Juniper Court")).await;
    let plan = plan_moving(&mover, &root.join("Juniper Court"));
    let context = RenameOverlapContext::default();
    assert!(overlap_step(&app, &context, &plan).await.is_some());
    let (reason, message) = refusal(app.preflight_rename_title_folder_overlap(&plan).await)
        .expect("a shared folder is refused");
    assert_eq!(reason, overlap);
    assert!(message.contains("are the same folder"), "{message}");
    assert!(
        message.contains("the current folder of Idle Feature"),
        "{message}"
    );

    // Only the letter case differs, on folders not made yet: refused exactly
    // when the root's volume ignores case.
    set_folder(&app, &idle, &root.join("Kestrel Row")).await;
    let plan = plan_moving(&mover, &root.join("kestrel row"));
    let context = RenameOverlapContext::default();
    let found = overlap_step(&app, &context, &plan).await;
    assert_eq!(found.is_some(), root_case_insensitive(&root));
    assert_eq!(
        refusal(app.preflight_rename_title_folder_overlap(&plan).await).map(|(reason, _)| reason),
        root_case_insensitive(&root).then_some(overlap)
    );

    // Planned inside a title that stays.
    set_folder(&app, &idle, &root.join("Larch Hill")).await;
    let plan = plan_moving(&mover, &root.join("Larch Hill").join("Extras"));
    let context = RenameOverlapContext::default();
    assert_eq!(
        overlap_step(&app, &context, &plan).await,
        Some((idle.id.clone(), mover.id.clone(), true))
    );
    let (reason, message) = refusal(app.preflight_rename_title_folder_overlap(&plan).await)
        .expect("planned inside is refused");
    assert_eq!(reason, overlap);
    assert!(message.contains("is inside"), "{message}");

    // A title that stays, inside the planned folder.
    set_folder(&app, &idle, &root.join("Maple Ford").join("Inner")).await;
    let plan = plan_moving(&mover, &root.join("Maple Ford"));
    let context = RenameOverlapContext::default();
    assert_eq!(
        overlap_step(&app, &context, &plan).await,
        Some((mover.id.clone(), idle.id.clone(), true))
    );
    assert_eq!(
        refusal(app.preflight_rename_title_folder_overlap(&plan).await).map(|(reason, _)| reason),
        Some(overlap)
    );

    // Recorded outside every root: found as before, then left alone because
    // it is not a folder of its library.
    set_folder(&app, &idle, &outside.join("Oak Glen")).await;
    let plan = plan_moving(&mover, &outside.join("Oak Glen"));
    let context = RenameOverlapContext::default();
    assert!(overlap_step(&app, &context, &plan).await.is_some());
    assert!(
        app.preflight_rename_title_folder_overlap(&plan)
            .await
            .is_ok()
    );

    // A shared name prefix is not an overlap.
    set_folder(&app, &idle, &root.join("Pine Reach")).await;
    let plan = plan_moving(&mover, &root.join("Pine Reach Remastered"));
    let context = RenameOverlapContext::default();
    assert_eq!(overlap_step(&app, &context, &plan).await, None);
    assert!(
        app.preflight_rename_title_folder_overlap(&plan)
            .await
            .is_ok()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_linked_library_root_is_compared_through_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real-library");
    let linked = temp.path().join("linked-library");
    std::fs::create_dir_all(real.join("Quill Bay")).unwrap();
    std::os::unix::fs::symlink(&real, &linked).unwrap();
    let (app, actor, _) = crate::lib_tests::bootstrap_with_title_repo();
    let library_id = movie_library_with_roots(&app, &[&linked]).await;
    let idle = movie(
        &app,
        &actor,
        &library_id,
        "Idle Feature",
        Some(&linked.join("Nettle Way")),
    )
    .await;
    let mover = movie(&app, &actor, &library_id, "Moving Feature", None).await;

    let plan = plan_moving(&mover, &real.join("Nettle Way"));
    let context = RenameOverlapContext::default();
    assert_eq!(
        overlap_step(&app, &context, &plan).await,
        Some((idle.id.clone(), mover.id.clone(), false))
    );
    assert_eq!(
        refusal(app.preflight_rename_title_folder_overlap(&plan).await).map(|(reason, _)| reason),
        Some(Some(crate::error_reasons::library::RENAME_FOLDERS_OVERLAP))
    );
}

#[tokio::test]
async fn one_job_reads_once_and_sees_its_own_renames() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    std::fs::create_dir_all(root.join("Rowan Mead")).unwrap();
    let (app, actor, titles) = crate::lib_tests::bootstrap_with_title_repo();
    titles.counts_folder_writes.store(true, Ordering::SeqCst);
    let library_id = movie_library_with_roots(&app, &[&root]).await;
    for index in 0..24 {
        movie(
            &app,
            &actor,
            &library_id,
            &format!("Standing Feature {index}"),
            Some(&root.join(format!("Standing Feature {index}"))),
        )
        .await;
    }
    let first = movie(
        &app,
        &actor,
        &library_id,
        "First Feature",
        Some(&root.join("Rowan Mead")),
    )
    .await;
    let second = movie(&app, &actor, &library_id, "Second Feature", None).await;
    let third = movie(&app, &actor, &library_id, "Third Feature", None).await;

    let context = Arc::new(RenameOverlapContext::default());
    let in_job = |plan: RenamePlan| {
        let app = app.clone();
        let context = Arc::clone(&context);
        async move {
            with_rename_overlap_context(context, async {
                app.preflight_rename_title_folder_overlap(&plan).await
            })
            .await
        }
    };

    in_job(plan_moving(&first, &root.join("Sorrel Gate")))
        .await
        .unwrap();
    let after_first = context.disk_counts().await;
    assert_eq!(context.recorded_reads(), 1);
    assert_eq!(after_first.root_probes, 1);
    assert_eq!(after_first.parent_reads, 1);
    assert_eq!(after_first.folder_resolutions, 0);

    in_job(plan_moving(&second, &root.join("Tansy Field")))
        .await
        .unwrap();
    assert_eq!(context.recorded_reads(), 1, "nothing changed: reused");
    assert_eq!(context.disk_counts().await, after_first);

    // The first title's rename lands: a later title sees it at its new
    // folder and not at its old one.
    set_folder(&app, &first, &root.join("Sorrel Gate")).await;
    let (reason, message) = refusal(in_job(plan_moving(&third, &root.join("Sorrel Gate"))).await)
        .expect("the earlier rename's folder is taken");
    assert_eq!(
        reason,
        Some(crate::error_reasons::library::RENAME_FOLDERS_OVERLAP)
    );
    assert!(message.contains("First Feature"), "{message}");
    in_job(plan_moving(&third, &root.join("Rowan Mead")))
        .await
        .unwrap();
    assert_eq!(
        context.recorded_reads(),
        2,
        "read again once, after the write"
    );
    assert_eq!(context.disk_counts().await, after_first);
}
