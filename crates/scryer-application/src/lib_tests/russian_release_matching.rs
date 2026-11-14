//! Release comparison between a Russian catalog name and its Latin
//! transliteration.
//!
//! A catalog can hold a series under a Cyrillic name alone, with no Latin
//! alias, while a release names it in Latin letters. The two are in different
//! scripts, so no literal, collation or edit-distance lane relates them; the
//! Russian romanization rule set does, and only for a name the catalog marks
//! as Russian. Like the romaji tests beside this file, these go through the
//! identity gate, so what is asserted is the comparison every caller shares.
//! The transliteration table and what it leaves unfolded are pinned in
//! `scryer-domain`.

use super::*;

/// How a release wraps a series name.
type ReleaseShape = (&'static str, fn(&str) -> String);
const SHAPES: &[ReleaseShape] = &[
    ("scene episode", |name| {
        format!("{}.S01E09.2026.WEB-DL.1080p-GRP", name.replace(' ', "."))
    }),
    ("scene episode without a year", |name| {
        format!("{}.S01E09.1080p.WEB-DL.H264-GRP", name.replace(' ', "."))
    }),
    ("season pack", |name| {
        format!(
            "{}.S01.1080p.WEB-DL.AAC2.0.H.264-GRP",
            name.replace(' ', ".")
        )
    }),
];

async fn add_series(app: &AppUseCase, user: &User, name: &str, language: Option<&str>) -> String {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: name.into(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .update_title_hydrated_metadata(
            &title.id,
            TitleMetadataUpdate {
                metadata_language: language.map(str::to_string),
                ..Default::default()
            },
        )
        .await
        .expect("store the metadata language");
    title.id
}

async fn resolves_to(app: &AppUseCase, release: &str) -> Option<String> {
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

#[tokio::test]
async fn a_latin_release_reaches_a_cyrillic_only_title() {
    let mut unresolved = Vec::new();
    for (name, written) in [
        ("ИИскра", "IIskra"),
        ("Жухарь в Ночи", "Zhukhar v Nochi"),
        ("Тихий Чащук", "Tikhiy Chashchuk"),
        ("Юрная Цыпь 7", "Yurnaya Tsyp 7"),
    ] {
        for language in ["ru", "rus"] {
            let (app, user) = bootstrap();
            let expected = add_series(&app, &user, name, Some(language)).await;
            for (shape, build) in SHAPES {
                let release = build(written);
                if resolves_to(&app, &release).await.as_deref() != Some(expected.as_str()) {
                    unresolved.push(format!("{name} under {language} / {shape}: {release}"));
                }
            }
        }
    }
    assert!(
        unresolved.is_empty(),
        "the Latin transliteration of a Russian name names that series; these did not reach it:\n{}",
        unresolved.join("\n")
    );
}

/// Two catalog names can share one transliteration (`э` and `е` are both
/// `e`), and a Latin title can be named exactly what a Russian one
/// transliterates to. A release in that spelling must not be handed to the
/// Russian title while another library identity answers to it.
#[tokio::test]
async fn a_rival_with_the_same_key_blocks_the_match() {
    let (app, user) = bootstrap();
    add_series(&app, &user, "Эхор", Some("ru")).await;
    assert!(
        resolves_to(&app, "Ekhor.S01E09.2026.WEB-DL.1080p-GRP")
            .await
            .is_some(),
        "alone, the Russian title answers to its transliteration"
    );
    add_series(&app, &user, "Ехор", Some("ru")).await;
    for (_, build) in SHAPES {
        let release = build("Ekhor");
        assert_eq!(
            resolves_to(&app, &release).await,
            None,
            "{release} names two library identities"
        );
    }

    let (app, user) = bootstrap();
    let russian = add_series(&app, &user, "ИИскра", Some("ru")).await;
    let latin = add_series(&app, &user, "IIskra", Some("en")).await;
    for (_, build) in SHAPES {
        let release = build("IIskra");
        let resolved = resolves_to(&app, &release).await;
        assert_ne!(
            resolved.as_deref(),
            Some(russian.as_str()),
            "{release} is the literal name of another title"
        );
        assert_eq!(resolved.as_deref(), Some(latin.as_str()), "{release}");
    }
}

/// The catalog's language tag is what licenses the transliteration, and it is
/// one transliteration: another spelling of the same letters, another name,
/// or a name the rule set refuses reaches nothing.
#[tokio::test]
async fn only_the_one_transliteration_of_a_russian_name_reaches_it() {
    for language in [None, Some("uk"), Some("bg"), Some("en")] {
        let (app, user) = bootstrap();
        add_series(&app, &user, "ИИскра", language).await;
        assert_eq!(
            resolves_to(&app, "IIskra.S01E09.2026.WEB-DL.1080p-GRP").await,
            None,
            "a name tagged {language:?} is not read as Russian"
        );
    }

    let (app, user) = bootstrap();
    add_series(&app, &user, "Жухарь в Ночи", Some("ru")).await;
    for release in [
        // `х` written `h` or `x`: a different transliteration, not folded.
        "Zhuhar.v.Nochi.S01E09.2026.WEB-DL.1080p-GRP",
        "Zhuxar.v.Nochi.S01E09.2026.WEB-DL.1080p-GRP",
        // An extra word, a missing word, a different name.
        "Zhukhar.v.Nochi.Dva.S01E09.2026.WEB-DL.1080p-GRP",
        "Zhukhar.Nochi.S01E09.2026.WEB-DL.1080p-GRP",
        "Tikhiy.v.Nochi.S01E09.2026.WEB-DL.1080p-GRP",
        // A sequel number the catalog name does not carry.
        "Zhukhar.v.Nochi.2.S01E09.WEB-DL.1080p-GRP",
    ] {
        assert_eq!(
            resolves_to(&app, release).await,
            None,
            "{release} is not the title's transliteration"
        );
    }

    // A catalog name with a letter outside the Russian alphabet, or one
    // mixing scripts, has no key even when tagged Russian.
    for (name, release) in [
        ("Їскра", "Iskra.S01E09.2026.WEB-DL.1080p-GRP"),
        ("Іскра", "Iskra.S01E09.2026.WEB-DL.1080p-GRP"),
        ("ИИскра Show", "IIskra.Show.S01E09.2026.WEB-DL.1080p-GRP"),
    ] {
        let (app, user) = bootstrap();
        add_series(&app, &user, name, Some("ru")).await;
        assert_eq!(resolves_to(&app, release).await, None, "{name} / {release}");
    }
}
