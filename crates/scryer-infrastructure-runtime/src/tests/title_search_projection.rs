//! The persisted multilingual projection, proven on both dialects.
//!
//! These assertions are the contract the release/import resolver will read in
//! phase 3 instead of rebuilding an index per process: one literal
//! (diacritic-preserving) form, one lenient (diacritic-folded) form, the bucket
//! facts (script, numbers guard, length), the equality keys that are not
//! bounded edit distances (romanization, ICU collation per profile), and the
//! stamp that says which collation data wrote those keys.

use super::*;
use scryer_domain::title_spelling;
use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime, StoreDatastore};

#[derive(Clone, Debug)]
struct ProjectedTerm {
    term_id: i64,
    term_kind: String,
    raw_term: String,
    normalized_term: String,
    literal_term: String,
    stripped_year_key: String,
    script: String,
    numbers_key: String,
    char_length: i64,
    romanization_key: Option<String>,
    language_tag: Option<String>,
    title_year: Option<i32>,
}

async fn projected_terms(
    datastore: &StoreDatastore,
    title_id: &str,
) -> AppResult<Vec<ProjectedTerm>> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT term_id, term_kind, raw_term, normalized_term, literal_term,
                stripped_year_key, script, numbers_key, char_length,
                romanization_key, language_tag, title_year
           FROM title_search_terms
          WHERE title_id = {}
          ORDER BY term_kind, normalized_term",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(ProjectedTerm {
                term_id: row.i64("term_id")?,
                term_kind: row.text("term_kind")?,
                raw_term: row.text("raw_term")?,
                normalized_term: row.text("normalized_term")?,
                literal_term: row.text("literal_term")?,
                stripped_year_key: row.text("stripped_year_key")?,
                script: row.text("script")?,
                numbers_key: row.text("numbers_key")?,
                char_length: row.i64("char_length")?,
                romanization_key: row.opt_text("romanization_key")?,
                language_tag: row.opt_text("language_tag")?,
                title_year: row.opt_i32("title_year")?,
            })
        })
        .collect()
}

async fn collation_keys(
    datastore: &StoreDatastore,
    term_id: i64,
) -> AppResult<Vec<(String, Vec<u8>)>> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT profile, collation_key
           FROM title_search_collation_keys
          WHERE term_id = {}
          ORDER BY profile",
        &[SqlArg::I64(term_id)],
    )
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok((
                row.text("profile")?,
                row.opt_bytes("collation_key")?
                    .expect("a projected collation key is never null"),
            ))
        })
        .collect()
}

fn term<'a>(terms: &'a [ProjectedTerm], term_kind: &str, normalized: &str) -> &'a ProjectedTerm {
    terms
        .iter()
        .find(|candidate| {
            candidate.term_kind == term_kind && candidate.normalized_term == normalized
        })
        .unwrap_or_else(|| {
            panic!("projection is missing a {term_kind} row for {normalized:?}: {terms:#?}")
        })
}

fn multilingual_title(
    id: &str,
    name: &str,
    language: Option<&str>,
    year: Option<i32>,
    aliases: &[&str],
    tagged_aliases: &[(&str, &str)],
) -> Title {
    let mut title = make_test_title(id, None);
    title.name = name.to_string();
    title.metadata_language = language.map(str::to_string);
    title.year = year;
    title.aliases = aliases.iter().map(|alias| alias.to_string()).collect();
    title.tagged_aliases = tagged_aliases
        .iter()
        .map(|(name, language)| TaggedAlias {
            name: (*name).to_string(),
            language: (*language).to_string(),
        })
        .collect();
    title
}

async fn profiles_for(datastore: &StoreDatastore, row: &ProjectedTerm) -> AppResult<Vec<String>> {
    Ok(collation_keys(datastore, row.term_id)
        .await?
        .into_iter()
        .map(|(profile, _)| profile)
        .collect())
}

async fn key_for(
    datastore: &StoreDatastore,
    row: &ProjectedTerm,
    profile: &str,
) -> AppResult<Vec<u8>> {
    collation_keys(datastore, row.term_id)
        .await?
        .into_iter()
        .find(|(candidate, _)| candidate == profile)
        .map(|(_, key)| key)
        .ok_or_else(|| {
            AppError::Validation(format!(
                "projection is missing the {profile} collation key for {:?}",
                row.normalized_term
            ))
        })
}

/// One body, run against SQLite and PostgreSQL. Anything that holds on one
/// dialect and not the other is a projection bug, not a dialect difference:
/// every value here is computed in Rust and written as a column.
async fn assert_multilingual_projection(
    catalog: &TitleStore,
    datastore: &StoreDatastore,
) -> AppResult<()> {
    // The stamp the migration's rebuild wrote. A build with different
    // collation data must not read these keys.
    let stamped: String = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT collation_version FROM title_search_meta WHERE id = {}",
        &[SqlArg::I64(1)],
    )
    .await?
    .expect("the projection carries a collation stamp row")
    .text("collation_version")?;
    assert_eq!(stamped, title_spelling::title_collation_data_version());

    // --- German: umlauts and ß -------------------------------------------
    TitleRepository::create(
        catalog,
        multilingual_title(
            "de-strasse",
            "Die Müller Straße",
            Some("de"),
            Some(1998),
            &["Grüße aus Berlin"],
            &[],
        ),
    )
    .await?;
    let terms = projected_terms(datastore, "de-strasse").await?;

    // The lenient form a UI query is matched against: `ü` folded to `u` and
    // `ß` spelled `ss`, so someone typing `Muller Strasse` reaches this row.
    let name = term(&terms, "name", "die muller strasse");
    assert_eq!(name.raw_term, "Die Müller Straße");
    // The literal form keeps every diacritic and the ß: it is what release
    // and import resolution key on. Only the lenient form folds them.
    assert_eq!(name.literal_term, "die müller straße");
    assert_eq!(name.stripped_year_key, "die müller straße");
    assert_eq!(name.script, "latin");
    assert_eq!(name.numbers_key, "");
    assert_eq!(name.char_length, 17);
    assert_eq!(name.romanization_key, None);
    assert_eq!(name.language_tag.as_deref(), Some("de"));
    assert_eq!(name.title_year, Some(1998));
    assert_eq!(
        profiles_for(datastore, name).await?,
        vec!["de".to_string(), "de-u-co-phonebk".to_string()]
    );

    let alias = term(&terms, "alias", "grusse aus berlin");
    assert_eq!(alias.literal_term, "grüße aus berlin");
    assert_eq!(alias.title_year, Some(1998));

    // The per-word typo lane carries the same fold, so a query typed
    // `Strasse` reaches the token row of a title written `Straße`.
    assert_eq!(
        term(&terms, "name_token", "strasse").literal_term,
        "strasse"
    );

    // A word inside a name does not date the name.
    let token = term(&terms, "name_token", "muller");
    assert_eq!(token.title_year, None);
    assert_eq!(token.char_length, 6);

    // --- German phonebook expansion --------------------------------------
    TitleRepository::create(
        catalog,
        multilingual_title("de-muller", "Müller", Some("de"), None, &[], &[]),
    )
    .await?;
    TitleRepository::create(
        catalog,
        multilingual_title("de-mueller", "Mueller", Some("de"), None, &[], &[]),
    )
    .await?;
    let umlaut = projected_terms(datastore, "de-muller").await?;
    let umlaut = term(&umlaut, "name", "muller").clone();
    let spelled_out = projected_terms(datastore, "de-mueller").await?;
    let spelled_out = term(&spelled_out, "name", "mueller").clone();
    assert_eq!(umlaut.literal_term, "müller");
    assert_eq!(spelled_out.literal_term, "mueller");
    assert_eq!(
        key_for(datastore, &umlaut, "de-u-co-phonebk").await?,
        key_for(datastore, &spelled_out, "de-u-co-phonebk").await?,
        "the phonebook profile is the lane that equates Müller and Mueller"
    );
    assert_ne!(
        key_for(datastore, &umlaut, "de").await?,
        key_for(datastore, &spelled_out, "de").await?,
        "the plain German profile does not equate them, which is why the \
         phonebook profile is projected alongside it"
    );

    // --- Japanese romanization variance ----------------------------------
    TitleRepository::create(
        catalog,
        multilingual_title(
            "ja-gassho",
            "Gasshō",
            Some("ja"),
            None,
            &["Gasshou", "Gassho"],
            &[
                ("Yuusha no Shimbun", "x-jat"),
                ("Yusha no Shinbun", "ja-Latn"),
                ("Kaze wo Miru", "x-jat"),
                ("Kaze o Miru", "x-jat"),
            ],
        ),
    )
    .await?;
    let terms = projected_terms(datastore, "ja-gassho").await?;

    // Macron, doubled vowel and bare spelling are one key. A bounded edit
    // distance cannot be trusted to cover this, so it is an equality key.
    assert_eq!(
        term(&terms, "name", "gassho").romanization_key.as_deref(),
        Some("gassho")
    );
    assert_eq!(term(&terms, "name", "gassho").literal_term, "gasshō");
    assert_eq!(
        term(&terms, "alias", "gasshou").romanization_key.as_deref(),
        Some("gassho")
    );
    assert_eq!(
        term(&terms, "alias", "gassho").romanization_key.as_deref(),
        Some("gassho")
    );

    // A tagged alias carries its own language tag, which is what makes a
    // Latin-script name a Japanese romanization at all.
    let shimbun = term(&terms, "tagged_alias", "yuusha no shimbun");
    let shinbun = term(&terms, "tagged_alias", "yusha no shinbun");
    assert_eq!(shimbun.language_tag.as_deref(), Some("x-jat"));
    assert_eq!(shinbun.language_tag.as_deref(), Some("ja-Latn"));
    assert_eq!(shimbun.romanization_key, shinbun.romanization_key);
    assert_eq!(
        shimbun.romanization_key.as_deref(),
        Some("yusha no shinbun")
    );

    let wo = term(&terms, "tagged_alias", "kaze wo miru");
    let o = term(&terms, "tagged_alias", "kaze o miru");
    assert_eq!(wo.romanization_key, o.romanization_key);
    assert_eq!(wo.romanization_key.as_deref(), Some("kaze o miru"));

    // --- CJK -------------------------------------------------------------
    TitleRepository::create(
        catalog,
        multilingual_title("zh-wandering", "流浪地球2", Some("zh"), None, &[], &[]),
    )
    .await?;
    let terms = projected_terms(datastore, "zh-wandering").await?;
    let name = term(&terms, "name", "流浪地球2");
    assert_eq!(name.script, "cjk");
    assert_eq!(name.numbers_key, "2");
    assert_eq!(name.char_length, 5);
    assert_eq!(name.romanization_key, None);
    assert_eq!(profiles_for(datastore, name).await?, vec!["zh".to_string()]);

    // --- Cyrillic --------------------------------------------------------
    TitleRepository::create(
        catalog,
        multilingual_title("ru-evening", "Майский вечер", Some("ru"), None, &[], &[]),
    )
    .await?;
    let terms = projected_terms(datastore, "ru-evening").await?;
    // Diacritic folding is not a Latin-only rule: the lenient form drops the
    // breve on `й` as well, which is exactly why the literal form has to be a
    // separate column rather than a recomputation of the lenient one.
    let name = term(&terms, "name", "маискии вечер");
    assert_eq!(name.script, "cyrillic");
    assert_eq!(name.literal_term, "майский вечер");
    assert_eq!(profiles_for(datastore, name).await?, vec!["ru".to_string()]);

    // A Ukrainian-tagged Cyrillic name collates under its own profile. The
    // lenient form folds `ї` to `і` the same way it drops the breve on `й`;
    // the literal form keeps both, and so does the collation key.
    TitleRepository::create(
        catalog,
        multilingual_title("uk-porch", "Ґанок і їжак", Some("uk"), None, &[], &[]),
    )
    .await?;
    let terms = projected_terms(datastore, "uk-porch").await?;
    let name = term(&terms, "name", "ґанок і іжак");
    assert_eq!(name.script, "cyrillic");
    assert_eq!(name.literal_term, "ґанок і їжак");
    assert_eq!(profiles_for(datastore, name).await?, vec!["uk".to_string()]);

    // No language tag means no supported collation for a Cyrillic name. The
    // projection must record *no* key rather than a Latin one: an absent key
    // is a lane that does not fire, a wrong key is a wrong match.
    TitleRepository::create(
        catalog,
        multilingual_title("ru-untagged", "Майский вечер", None, None, &[], &[]),
    )
    .await?;
    let terms = projected_terms(datastore, "ru-untagged").await?;
    let name = term(&terms, "name", "маискии вечер");
    assert_eq!(name.script, "cyrillic");
    assert!(profiles_for(datastore, name).await?.is_empty());

    // --- Mixed script under one language ---------------------------------
    TitleRepository::create(
        catalog,
        multilingual_title(
            "jp-mixed",
            "Kōkaku Kidōtai",
            Some("x-jat"),
            None,
            &["攻殻機動隊"],
            &[],
        ),
    )
    .await?;
    let terms = projected_terms(datastore, "jp-mixed").await?;
    let latin = term(&terms, "name", "kokaku kidotai");
    assert_eq!(latin.script, "latin");
    assert_eq!(latin.literal_term, "kōkaku kidōtai");
    assert_eq!(latin.romanization_key.as_deref(), Some("kokaku kidotai"));
    assert_eq!(
        profiles_for(datastore, latin).await?,
        vec!["und".to_string()],
        "an unknown language tag on a Latin name still collates as und"
    );
    let cjk = term(&terms, "alias", "攻殻機動隊");
    assert_eq!(cjk.script, "cjk");
    assert_eq!(cjk.romanization_key, None);
    assert!(
        profiles_for(datastore, cjk).await?.is_empty(),
        "a CJK name under a transliteration tag has no collation profile"
    );

    // --- Roman numerals and the year guard -------------------------------
    TitleRepository::create(
        catalog,
        multilingual_title("en-rocky-iv", "Rocky IV", Some("en"), Some(1985), &[], &[]),
    )
    .await?;
    TitleRepository::create(
        catalog,
        multilingual_title("en-rocky-4", "Rocky 4", Some("en"), Some(1985), &[], &[]),
    )
    .await?;
    let roman = projected_terms(datastore, "en-rocky-iv").await?;
    let roman = term(&roman, "name", "rocky iv");
    assert_eq!(roman.numbers_key, "roman:iv");
    let arabic = projected_terms(datastore, "en-rocky-4").await?;
    let arabic = term(&arabic, "name", "rocky 4");
    assert_eq!(arabic.numbers_key, "4");
    assert_ne!(
        roman.numbers_key, arabic.numbers_key,
        "the numbers guard separates sequels; it is not a spelling"
    );

    // Ordinary words that the Roman-numeral pattern accepts must not land in
    // the numbers guard: `Mix` parses as 1009, and a spurious number there
    // splits a title from its own aliases.
    for (id, name) in [
        ("en-mix", "Mix"),
        ("en-did", "Did"),
        ("en-mid", "Mid"),
        ("en-dim", "Dim"),
        ("en-civil", "Civil"),
    ] {
        TitleRepository::create(
            catalog,
            multilingual_title(id, name, Some("en"), None, &[], &[]),
        )
        .await?;
        let terms = projected_terms(datastore, id).await?;
        let lowered = name.to_lowercase();
        assert_eq!(
            term(&terms, "name", &lowered).numbers_key,
            "",
            "{name} must not read as a Roman numeral"
        );
    }

    // A lower-case numeral counts only behind a counting word.
    TitleRepository::create(
        catalog,
        multilingual_title("en-part-ii", "Harbour part ii", Some("en"), None, &[], &[]),
    )
    .await?;
    let terms = projected_terms(datastore, "en-part-ii").await?;
    assert_eq!(
        term(&terms, "name", "harbour part ii").numbers_key,
        "roman:ii"
    );

    // A trailing year is stripped for collision counting but kept in the
    // literal form, so `Tide Chart` and `Tide Chart 2023` are one identity
    // shape with two spellings.
    TitleRepository::create(
        catalog,
        multilingual_title(
            "en-tide-year",
            "Tide Chart 2023",
            Some("en"),
            Some(2023),
            &[],
            &[],
        ),
    )
    .await?;
    let terms = projected_terms(datastore, "en-tide-year").await?;
    let name = term(&terms, "name", "tide chart 2023");
    assert_eq!(name.literal_term, "tide chart 2023");
    assert_eq!(name.stripped_year_key, "tide chart");
    assert_eq!(name.numbers_key, "2023");

    Ok(())
}

#[tokio::test]
async fn multilingual_title_projection_on_sqlite() -> AppResult<()> {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_search_projection_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await?;
    let catalog = TitleStore::new(services.datastore());
    let result = assert_multilingual_projection(&catalog, &services.datastore()).await;
    services.pool().close().await;
    let _ = std::fs::remove_file(&db);
    result
}

#[tokio::test]
async fn multilingual_title_projection_on_postgres() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        eprintln!(
            "skipping PostgreSQL title search projection test; SCRYER_TEST_POSTGRES_URL is not set"
        );
        return Ok(());
    };

    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_test_{}_{}",
        std::process::id(),
        Id::new().0.replace('-', "_")
    );

    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to create schema: {error}")))?;

    let result = async {
        let mut url = url::Url::parse(&raw_url)
            .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        let services =
            crate::PostgresServices::new_with_mode(url.to_string(), crate::MigrationMode::Apply)
                .await?;
        let catalog = TitleStore::new(services.datastore());
        let result = assert_multilingual_projection(&catalog, &services.datastore()).await;
        services.pool().close().await;
        result
    }
    .await;

    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    admin_pool.close().await;
    cleanup.map_err(|error| AppError::Repository(format!("failed to drop schema: {error}")))?;
    result
}
