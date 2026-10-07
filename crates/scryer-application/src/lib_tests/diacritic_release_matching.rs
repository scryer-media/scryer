//! Release comparison across the spellings a group uses for a diacritic name.
//!
//! A catalog name keeps its diacritics. A release name usually does not, and
//! groups disagree about what to put in their place. The same German series
//! ships as `Die.Höhle.der.Löwen`, as `Die.Hohle.der.Lowen` with the marks
//! dropped, and as `Die.Hoehle.der.Loewen` with each mark written out the way
//! the language spells it on a typewriter; `Straße` ships as `Strasse`. French
//! loses its ligature and its accent in one step, `cœur` becoming `coeur`.
//! All of them name one subject.
//!
//! Established locale equivalents can identify a unique local title without
//! corroboration. Actual typos still need corroboration, and collisions remain
//! ambiguous. German phonebook expansion requires a German language tag.
//!
//! These go through the identity gate — the monitored-title matcher the import
//! path, the tracked-download sweep and the acquisition lanes all resolve
//! through — so what is asserted is the comparison itself rather than any one
//! caller's wrapper. The persisted lanes that feed it candidates are covered
//! where they are real, in `scryer-infrastructure-runtime`'s
//! `title_name_candidates` and `title_fuzzy_index` tests.

use super::*;

const SUBJECT_YEAR: i32 = 2019;

/// A subject: the name the catalog stores, and the spellings a release uses
/// for it. The first spelling is always the catalog's own, because a group
/// that keeps the diacritics must not be the case that breaks.
struct Subject {
    catalog: &'static str,
    spellings: &'static [&'static str],
}

/// German, Portuguese, French and Spanish names, each with the marks kept,
/// dropped, and — where the language has one — written out. The German pair
/// carries the partial case too, one mark kept and one dropped, which is what
/// a group produces by hand-typing half a name.
const SUBJECTS: &[Subject] = &[
    Subject {
        catalog: "Die Höhle der Löwen",
        spellings: &[
            "Die Höhle der Löwen",
            "Die Hohle der Lowen",
            "Die Hoehle der Loewen",
            "Die Höhle der Lowen",
        ],
    },
    Subject {
        catalog: "Die Müller Straße",
        spellings: &[
            "Die Müller Straße",
            "Die Muller Strasse",
            "Die Mueller Strasse",
        ],
    },
    Subject {
        catalog: "Coração de Açúcar",
        spellings: &["Coração de Açúcar", "Coracao de Acucar"],
    },
    Subject {
        catalog: "Le cœur de Chloé",
        spellings: &["Le cœur de Chloé", "Le coeur de Chloe"],
    },
    Subject {
        catalog: "El último día",
        spellings: &["El último día", "El ultimo dia"],
    },
];

fn scene_name(spelling: &str) -> String {
    spelling.replace(' ', ".")
}

/// The catalog under test, and the id each stored name was given.
type Catalog = (AppUseCase, std::collections::BTreeMap<String, String>);

async fn catalog_with(subjects: &[(&str, MediaFacet, Option<i32>)]) -> Catalog {
    let (app, user) = bootstrap();
    let mut ids = std::collections::BTreeMap::new();
    for (name, facet, year) in subjects {
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: (*name).into(),
                    facet: facet.clone(),
                    monitored: true,
                    year: *year,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");
        if name.starts_with("Die ") {
            app.services
                .catalog
                .titles
                .update_title_hydrated_metadata(
                    &title.id,
                    TitleMetadataUpdate {
                        metadata_language: Some("deu".into()),
                        ..Default::default()
                    },
                )
                .await
                .expect("tag German names");
        }
        ids.insert((*name).to_string(), title.id);
    }
    (app, ids)
}

async fn catalog_of_subjects(facet: MediaFacet) -> Catalog {
    let subjects = SUBJECTS
        .iter()
        .map(|subject| (subject.catalog, facet.clone(), Some(SUBJECT_YEAR)))
        .collect::<Vec<_>>();
    catalog_with(&subjects).await
}

fn title_id_for<'a>(ids: &'a std::collections::BTreeMap<String, String>, name: &str) -> &'a str {
    ids.get(name)
        .unwrap_or_else(|| panic!("the catalog holds {name}"))
        .as_str()
}

async fn resolve_episode(app: &AppUseCase, release: &str) -> Option<String> {
    let matcher = app
        .monitored_title_matcher()
        .await
        .expect("build the monitored title matcher");
    let parsed = crate::release_parser::parse_release_metadata(release);
    matcher
        .resolve_episode(&parsed, Some("series"))
        .await
        .expect("resolution must not fail")
        .map(|resolved| resolved.title.id)
}

async fn resolve_movie(app: &AppUseCase, release: &str) -> Option<String> {
    let matcher = app
        .monitored_title_matcher()
        .await
        .expect("build the monitored title matcher");
    let parsed = crate::release_parser::parse_release_metadata(release);
    matcher
        .resolve_movie(&parsed)
        .await
        .expect("resolution must not fail")
        .map(|resolved| resolved.title.id)
}

/// Every episodic shape a group ships, each carrying the subject's year.
type ReleaseShape = (&'static str, fn(&str) -> String);
const SERIES_SHAPES: &[ReleaseShape] = &[
    ("single episode", |name| {
        format!("{name}.{SUBJECT_YEAR}.S01E01.1080p.WEB-DL.H264-Group")
    }),
    ("season pack", |name| {
        format!("{name}.{SUBJECT_YEAR}.S01.1080p.WEB-DL.AAC2.0.H.264-Group")
    }),
    ("multi-episode range", |name| {
        format!("{name}.{SUBJECT_YEAR}.S01E01-E03.1080p.WEB-DL.H264-Group")
    }),
];

#[tokio::test]
async fn every_diacritic_spelling_of_a_dated_series_release_reaches_its_title() {
    let (app, ids) = catalog_of_subjects(MediaFacet::Series).await;

    let mut unresolved = Vec::new();
    for subject in SUBJECTS {
        let expected = title_id_for(&ids, subject.catalog);
        for spelling in subject.spellings {
            for (shape, build) in SERIES_SHAPES {
                let release = build(&scene_name(spelling));
                if resolve_episode(&app, &release).await.as_deref() != Some(expected) {
                    unresolved.push(format!("{} / {shape}: {release}", subject.catalog));
                }
            }
        }
    }

    assert!(
        unresolved.is_empty(),
        "every spelling of every shape names one subject; these did not reach it:\n{}",
        unresolved.join("\n")
    );
}

/// Movies carry their year by convention, so the whole matrix holds for them
/// without the caveat the series tests above have to make.
#[tokio::test]
async fn every_diacritic_spelling_of_a_movie_release_reaches_its_title() {
    let (app, ids) = catalog_of_subjects(MediaFacet::Movie).await;

    let mut unresolved = Vec::new();
    for subject in SUBJECTS {
        let expected = title_id_for(&ids, subject.catalog);
        for spelling in subject.spellings {
            let release = format!(
                "{}.{SUBJECT_YEAR}.1080p.BluRay.x264-Group",
                scene_name(spelling)
            );
            if resolve_movie(&app, &release).await.as_deref() != Some(expected) {
                unresolved.push(format!("{}: {release}", subject.catalog));
            }
        }
    }

    assert!(
        unresolved.is_empty(),
        "every spelling names one movie; these did not reach it:\n{}",
        unresolved.join("\n")
    );
}

/// Folding a mark away must not fold two subjects together. These two German
/// series differ in one word, and that word is the one carrying the umlaut, so
/// a comparison sloppy about diacritics answers the wrong title for both
/// transliterations of both names. They share a year, which removes the year
/// as a tiebreaker and leaves the names to do the work.
#[tokio::test]
async fn a_neighbouring_diacritic_subject_is_not_absorbed() {
    let (app, ids) = catalog_with(&[
        (
            "Die Höhle der Löwen",
            MediaFacet::Series,
            Some(SUBJECT_YEAR),
        ),
        (
            "Die Höhle der Bären",
            MediaFacet::Series,
            Some(SUBJECT_YEAR),
        ),
    ])
    .await;

    let lions = title_id_for(&ids, "Die Höhle der Löwen").to_string();
    let bears = title_id_for(&ids, "Die Höhle der Bären").to_string();

    for (release, expected, subject) in [
        (
            "Die.Hoehle.der.Loewen.2019.S01E01.1080p.WEB-DL.H264-Group",
            &lions,
            "Löwen",
        ),
        (
            "Die.Hohle.der.Lowen.2019.S01E01.1080p.WEB-DL.H264-Group",
            &lions,
            "Löwen",
        ),
        (
            "Die.Hoehle.der.Baeren.2019.S01E01.1080p.WEB-DL.H264-Group",
            &bears,
            "Bären",
        ),
        (
            "Die.Hohle.der.Baren.2019.S01E01.1080p.WEB-DL.H264-Group",
            &bears,
            "Bären",
        ),
    ] {
        assert_eq!(
            resolve_episode(&app, release).await.as_deref(),
            Some(expected.as_str()),
            "{release} names the {subject} subject and no other"
        );
    }
}

/// A dropped diacritic is an equivalence; a mangled word is not. The year that
/// lets every folded spelling above through does not let this through, which
/// is what keeps the tolerance bounded rather than generous.
#[tokio::test]
async fn a_corroborating_year_does_not_admit_a_mangled_name() {
    let (app, _ids) =
        catalog_with(&[("Harbor Lights", MediaFacet::Series, Some(SUBJECT_YEAR))]).await;

    assert!(
        resolve_episode(&app, "Harbor.Lights.2019.S01E01.1080p.WEB-DL.H264-Group")
            .await
            .is_some(),
        "the control resolves, so a None below is the name and not the shape"
    );

    assert_eq!(
        resolve_episode(&app, "Harbor.Ligths.2019.S01E01.1080p.WEB-DL.H264-Group").await,
        None,
        "a transposed word is a misspelling, not another spelling"
    );
}

/// A unique established locale equivalent does not need a release year or ID.
#[tokio::test]
async fn an_undated_series_release_accepts_unique_locale_equivalents() {
    let (app, ids) = catalog_of_subjects(MediaFacet::Series).await;
    for subject in SUBJECTS {
        let expected = title_id_for(&ids, subject.catalog);
        for spelling in subject.spellings {
            let release = format!("{}.S01E01.1080p.WEB-DL.H264-Group", scene_name(spelling));
            assert_eq!(
                resolve_episode(&app, &release).await.as_deref(),
                Some(expected),
                "{release}"
            );
        }
    }
}
