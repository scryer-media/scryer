//! Release comparison across the romanizations of one Japanese name.
//!
//! A catalog stores an anime under its native name and whatever romanization
//! its metadata provider supplies. A release group romanizes the same name
//! independently, and the two disagree about things that carry no meaning: a
//! long vowel is written with a macron, doubled, or left bare (`Gasshō`,
//! `Gasshou`, `Gasshoo`, `Gassho`); the particle is written `wo` or `o`; `n`
//! before a labial is written `m` (`Shimbun`, `Shinbun`). Every one of those
//! is the same name, and release comparison has to say so for every shape a
//! release comes in.
//!
//! What makes the fold legal is the language tag. A romanized alias arrives
//! tagged `x-jat` — the AniDB/TVDB transliteration tag — and the fold applies
//! only to names marked that way, so it can equate two spellings of one
//! Japanese name without touching anything else. The third test here pins
//! that the tag is load-bearing rather than incidental.
//!
//! These go through the identity gate — the monitored-title matcher the import
//! path, the tracked-download sweep and the acquisition lanes all resolve
//! through — so what is asserted is the comparison itself rather than any one
//! caller's wrapper. The fold's own rules are pinned in `scryer-domain`'s
//! `title_spelling` tests, which also record what it leaves alone:
//! kunrei-shiki consonants, the `ha` and `he` particles, and the syllabic
//! apostrophe. What its doc comment claims and the code does not deliver —
//! the doubled `aa`, `ii` and `ee`, long `e` written `ei`, and `m` before
//! another `m` — is the ignored test at the bottom here.

use super::*;

const NATIVE_NAME: &str = "蒼雲の記録";

/// A romanized alias, and the spellings a group writes for it. The first is
/// always the catalogued one, because a group that happens to agree with the
/// provider must not be the case that breaks.
struct Romanization {
    alias: &'static str,
    axis: &'static str,
    spellings: &'static [&'static str],
}

const ROMANIZATIONS: &[Romanization] = &[
    Romanization {
        alias: "Yuusha no Gasshou",
        axis: "long vowels",
        spellings: &[
            "Yuusha no Gasshou",
            "Yusha no Gassho",
            "Yuusha no Gassho",
            "Yusha no Gasshou",
            "Yūsha no Gasshō",
            "Yusha no Gasshoo",
        ],
    },
    Romanization {
        alias: "Sayounara no Natsu",
        axis: "long vowels inside an ordinary word",
        spellings: &[
            "Sayounara no Natsu",
            "Sayonara no Natsu",
            "Sayōnara no Natsu",
        ],
    },
    Romanization {
        alias: "Hikari wo Utau",
        axis: "the wo particle",
        spellings: &["Hikari wo Utau", "Hikari o Utau"],
    },
    Romanization {
        alias: "Yuusha no Shimbun",
        axis: "m before a labial",
        spellings: &["Yuusha no Shimbun", "Yusha no Shinbun", "Yuusha no Shinbun"],
    },
    Romanization {
        alias: "Sempai no Uta",
        axis: "m before p",
        spellings: &["Sempai no Uta", "Senpai no Uta"],
    },
];

/// How a group wraps an anime name: the fansub bracket shape, the scene
/// season/episode shape, and a season pack.
type ReleaseShape = (&'static str, fn(&str) -> String);
const SHAPES: &[ReleaseShape] = &[
    ("fansub episode", |name| {
        format!("[Group] {name} - 03 [1080p]")
    }),
    ("scene episode", |name| {
        format!("{}.S01E03.1080p.WEB-DL.H264-Group", name.replace(' ', "."))
    }),
    ("season pack", |name| {
        format!(
            "{}.S01.1080p.WEB-DL.AAC2.0.H.264-Group",
            name.replace(' ', ".")
        )
    }),
];

/// An anime whose catalog name is the native one, with `romanization` supplied
/// as an alias. `tagged` is whether the provider marked it a romanization,
/// which is what licenses the fold.
async fn anime_with_romanization(romanization: &str, tagged: bool) -> (AppUseCase, String) {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: NATIVE_NAME.into(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    app.services
        .catalog
        .titles
        .update_title_hydrated_metadata(
            &title.id,
            TitleMetadataUpdate {
                aliases: vec![romanization.to_string()],
                tagged_aliases: if tagged {
                    vec![scryer_domain::TaggedAlias {
                        name: romanization.to_string(),
                        language: "x-jat".to_string(),
                    }]
                } else {
                    Vec::new()
                },
                ..Default::default()
            },
        )
        .await
        .expect("store the provider's alias set");

    (app, title.id)
}

async fn resolves_to(app: &AppUseCase, release: &str) -> Option<String> {
    let matcher = app
        .monitored_title_matcher()
        .await
        .expect("build the monitored title matcher");
    let parsed = crate::release_parser::parse_release_metadata(release);
    matcher
        .resolve_episode(&parsed, Some("anime"))
        .await
        .expect("resolution must not fail")
        .map(|resolved| resolved.title.id)
}

#[tokio::test]
async fn every_romanization_of_every_shape_reaches_the_anime() {
    let mut unresolved = Vec::new();
    for romanization in ROMANIZATIONS {
        let (app, expected) = anime_with_romanization(romanization.alias, true).await;
        for spelling in romanization.spellings {
            for (shape, build) in SHAPES {
                let release = build(spelling);
                if resolves_to(&app, &release).await.as_deref() != Some(expected.as_str()) {
                    unresolved.push(format!("{} / {shape}: {release}", romanization.axis));
                }
            }
        }
    }

    assert!(
        unresolved.is_empty(),
        "every romanization of every shape names one anime; these did not reach it:\n{}",
        unresolved.join("\n")
    );
}

/// A group and a catalog split a romanized name into words differently: a
/// compound, an auxiliary or a particle is written apart in one and joined in
/// the other. The word count differs, so no edit-distance lane can bridge it;
/// the romanization key joins the words, and every split reaches the anime.
#[tokio::test]
async fn a_romanized_name_split_into_words_or_joined_reaches_the_anime() {
    let mut unresolved = Vec::new();
    for (alias, spellings) in [
        (
            "Hoshikage no Machi",
            &[
                "Hoshi Kage no Machi",
                "Hoshikage no Machi",
                "Hoshikageno Machi",
            ][..],
        ),
        (
            "Kirameki ni Shite Mite",
            &["Kirameki ni Shitemite", "Kirameki ni Shite Mite"][..],
        ),
    ] {
        let (app, expected) = anime_with_romanization(alias, true).await;
        for spelling in spellings {
            for (shape, build) in SHAPES {
                let release = build(spelling);
                if resolves_to(&app, &release).await.as_deref() != Some(expected.as_str()) {
                    unresolved.push(format!("{alias} / {shape}: {release}"));
                }
            }
        }
    }
    assert!(
        unresolved.is_empty(),
        "every word split of a romanized name names one anime; these did not reach it:\n{}",
        unresolved.join("\n")
    );
}

/// Joining words is licensed by the same tag as every other romanization
/// rule, and it never joins two different names into one.
#[tokio::test]
async fn joining_words_needs_the_tag_and_keeps_other_names_apart() {
    let (untagged, _) = anime_with_romanization("Hoshikage no Machi", false).await;
    assert_eq!(
        resolves_to(&untagged, "[Group] Hoshi Kage no Machi - 03 [1080p]").await,
        None,
        "an untagged alias is not a romanization, so its words are not joined"
    );

    let (app, _) = anime_with_romanization("Hoshikage no Machi", true).await;
    for release in [
        "[Group] Hoshi Kaze no Machi - 03 [1080p]",
        "[Group] Hoshikage no Mori - 03 [1080p]",
        "[Group] Hoshi no Machi - 03 [1080p]",
    ] {
        assert_eq!(
            resolves_to(&app, release).await,
            None,
            "{release} is a different name and must not reach the anime"
        );
    }
}

/// Folding romanization variance must not fold two names together. These are
/// different words, not different spellings of one word, and the fold touches
/// exactly the characters that carry no meaning.
#[tokio::test]
async fn a_different_japanese_name_is_not_absorbed_by_the_fold() {
    let (app, _expected) = anime_with_romanization("Yuusha no Gasshou", true).await;

    for release in [
        "[Group] Kage no Gasshou - 03 [1080p]",
        "[Group] Yuurei no Gasshou - 03 [1080p]",
        "[Group] Yuusha no Monogatari - 03 [1080p]",
    ] {
        assert_eq!(
            resolves_to(&app, release).await,
            None,
            "{release} is a different name and must not reach the anime"
        );
    }
}

/// The fold is licensed by the transliteration tag, not guessed from the
/// letters. Without it the alias is an ordinary Latin string: the spelling the
/// provider supplied still resolves, and a group's own romanization of the
/// same name does not.
///
/// The tag does two things, and the second is why every test above can leave
/// the year out. A romanization at distance zero is the one locale
/// equivalence exempt from corroboration: every other one, including every
/// diacritic fold, needs a year or an asserted indexer id before the matcher
/// accepts it, which `diacritic_release_matching` pins from the other side.
/// An anime episode carries neither, so without the exemption a romaji
/// release would reach nothing at all.
///
/// This is also the catalog shape that matters in practice — a provider that
/// supplies a romanized alias without tagging it leaves every group spelling
/// but its own unreachable — so it is pinned rather than left implied.
#[tokio::test]
async fn the_romanization_tag_is_what_licenses_the_fold() {
    let (app, expected) = anime_with_romanization("Yuusha no Gasshou", false).await;

    assert_eq!(
        resolves_to(&app, "[Group] Yuusha no Gasshou - 03 [1080p]")
            .await
            .as_deref(),
        Some(expected.as_str()),
        "the spelling the provider supplied resolves with or without the tag"
    );

    for release in [
        "[Group] Yusha no Gassho - 03 [1080p]",
        "[Group] Yūsha no Gasshō - 03 [1080p]",
    ] {
        assert_eq!(
            resolves_to(&app, release).await,
            None,
            "{release} needs the romanization tag to be equated"
        );
    }
}

/// What the fold promises and does not deliver, at the release layer.
/// The Japanese romanization fold said it folded long vowels "written with a
/// macron, doubled, or bare" and `m` before a labial; it folds the doubled
/// form for `o` and `u` only, does not read long `e` written `ei` at all, and
/// covers `m` before `b` and `p` but not before `m`. Every release below is
/// the spelling a group ships (`Okaasan`, `Niisan`, `Oneesan`, `Sensei`,
/// `Gumma`) and none of them reaches its title.
///
/// This lands harder here than it looks at the unit level. A romanization is
/// the one equivalence exempt from corroboration, so there is no year to fall
/// back on: a miss in the fold is the whole answer.
///
/// Ignored until the code covers what its doc claims; the assertions are what
/// it should answer. Pinned at the unit level in `scryer-domain`'s
/// `the_fold_covers_every_spelling_its_doc_claims`, which records why `ei` is
/// the harder half.
#[tokio::test]
#[ignore = "doubled aa/ii/ee, long e written ei, and m before m are not folded; see the comment above"]
async fn every_spelling_the_fold_claims_reaches_the_anime() {
    for (alias, release) in [
        ("Okāsan no Uta", "[Group] Okaasan no Uta - 03 [1080p]"),
        ("Nīsan no Uta", "[Group] Niisan no Uta - 03 [1080p]"),
        ("Onēsan no Uta", "[Group] Oneesan no Uta - 03 [1080p]"),
        ("Sensē no Uta", "[Group] Sensei no Uta - 03 [1080p]"),
        ("Sensei no Uta", "[Group] Sensee no Uta - 03 [1080p]"),
        ("Gunma no Uta", "[Group] Gumma no Uta - 03 [1080p]"),
    ] {
        let (app, expected) = anime_with_romanization(alias, true).await;
        assert_eq!(
            resolves_to(&app, release).await.as_deref(),
            Some(expected.as_str()),
            "{release} is the doubled spelling of {alias}"
        );
    }
}
