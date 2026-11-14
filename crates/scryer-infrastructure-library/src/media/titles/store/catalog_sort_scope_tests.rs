//! The catalog's movie-media and external-rating sorts read only the page's
//! scope, and its year, runtime, popularity and status sorts are served by
//! per-library indexes. These tests hold each of them to the query it replaced:
//! the same titles in the same order, for both directions and every scope.

use super::catalog_order_tests::fixture_title;
use super::*;

struct SortFixture {
    id: &'static str,
    name: &'static str,
    facet: MediaFacet,
    monitored: bool,
    year: Option<i32>,
    runtime: Option<i64>,
    popularity: Option<f64>,
    status: Option<&'static str>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "one positional row per fixture keeps the fixture table readable"
)]
const fn sort_fixture(
    id: &'static str,
    name: &'static str,
    facet: MediaFacet,
    monitored: bool,
    year: Option<i32>,
    runtime: Option<i64>,
    popularity: Option<f64>,
    status: Option<&'static str>,
) -> SortFixture {
    SortFixture {
        id,
        name,
        facet,
        monitored,
        year,
        runtime,
        popularity,
        status,
    }
}

/// Missing values, ties on every sort value, ties that only the title order or
/// the id can break, and status spellings in the cases and spacing rows hold.
const SORT_FIXTURES: &[SortFixture] = &[
    sort_fixture(
        "m-emberfall",
        "Emberfall",
        MediaFacet::Movie,
        true,
        Some(2001),
        Some(95),
        Some(12.5),
        Some("Released"),
    ),
    sort_fixture(
        "m-emberfall-two",
        "Emberfall",
        MediaFacet::Movie,
        true,
        Some(2001),
        Some(95),
        Some(12.5),
        Some("released"),
    ),
    sort_fixture(
        "m-cinderwake",
        "The Cinderwake",
        MediaFacet::Movie,
        false,
        None,
        None,
        None,
        None,
    ),
    sort_fixture(
        "m-ashgrove",
        "Ashgrove",
        MediaFacet::Movie,
        true,
        Some(1987),
        Some(142),
        Some(0.0),
        Some(""),
    ),
    sort_fixture(
        "m-quillmere",
        "Quillmere",
        MediaFacet::Movie,
        true,
        None,
        Some(95),
        Some(88.25),
        Some("  "),
    ),
    sort_fixture(
        "m-thornhallow",
        "Thornhallow",
        MediaFacet::Movie,
        true,
        Some(2019),
        None,
        Some(12.5),
        Some(" In Production "),
    ),
    sort_fixture(
        "m-ashgrove-late",
        "Ashgrove",
        MediaFacet::Movie,
        false,
        Some(2019),
        Some(61),
        None,
        Some("in production"),
    ),
    sort_fixture(
        "s-emberfall",
        "Emberfall",
        MediaFacet::Series,
        true,
        Some(2001),
        Some(45),
        Some(3.5),
        Some("Returning"),
    ),
    sort_fixture(
        "s-cinderwake",
        "Cinderwake",
        MediaFacet::Series,
        true,
        Some(2010),
        Some(45),
        Some(64.0),
        Some("continuing"),
    ),
    sort_fixture(
        "s-ashgrove",
        "Ashgrove",
        MediaFacet::Series,
        false,
        None,
        Some(22),
        None,
        Some(" FINISHED"),
    ),
    sort_fixture(
        "s-quillmere",
        "Quillmere",
        MediaFacet::Series,
        true,
        Some(1987),
        None,
        Some(64.0),
        Some("Ended"),
    ),
    sort_fixture(
        "s-thornhallow",
        "Thornhallow",
        MediaFacet::Series,
        true,
        Some(2010),
        Some(22),
        Some(0.5),
        None,
    ),
    sort_fixture(
        "a-emberfall",
        "Emberfall",
        MediaFacet::Anime,
        true,
        Some(2001),
        Some(24),
        Some(12.5),
        Some("finished"),
    ),
    sort_fixture(
        "a-cinderwake",
        "Cinderwake",
        MediaFacet::Anime,
        false,
        Some(2024),
        Some(24),
        None,
        Some("upcoming"),
    ),
    sort_fixture(
        "a-ashgrove",
        "Ashgrove",
        MediaFacet::Anime,
        true,
        None,
        None,
        Some(7.0),
        Some("RETURNING "),
    ),
];

const RATING_KEYS: [TitleCatalogSortKey; 13] = [
    TitleCatalogSortKey::RatingImdb,
    TitleCatalogSortKey::RatingRottenTomatoes,
    TitleCatalogSortKey::RatingPopcornmeter,
    TitleCatalogSortKey::RatingMetacritic,
    TitleCatalogSortKey::RatingMetacriticUser,
    TitleCatalogSortKey::RatingLetterboxd,
    TitleCatalogSortKey::RatingTmdb,
    TitleCatalogSortKey::RatingTvdb,
    TitleCatalogSortKey::RatingTrakt,
    TitleCatalogSortKey::RatingMyanimelist,
    TitleCatalogSortKey::RatingAnilist,
    TitleCatalogSortKey::RatingAnidb,
    TitleCatalogSortKey::RatingMdblist,
];

const MOVIE_MEDIA_KEYS: [TitleCatalogSortKey; 3] = [
    TitleCatalogSortKey::MediaResolution,
    TitleCatalogSortKey::MediaHdr,
    TitleCatalogSortKey::MediaAudioCodec,
];

const INDEXED_KEYS: [(TitleCatalogSortKey, &str); 4] = [
    (TitleCatalogSortKey::Year, "year"),
    (TitleCatalogSortKey::Runtime, "runtime"),
    (TitleCatalogSortKey::Popularity, "popularity"),
    (TitleCatalogSortKey::Status, "status"),
];

async fn exec(datastore: &StoreDatastore, sql: &str, args: Vec<SqlArg>) {
    SqlRuntime::execute_write(datastore, "catalog_sort_scope_fixture", sql, args)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

struct MovieFile<'a> {
    id: &'a str,
    title_id: &'a str,
    path: &'a str,
    size: i64,
    role: &'a str,
    dims: (i64, i64),
    quality_id: &'a str,
    hdr: Option<&'a str>,
    audio: Option<&'a str>,
    audio_parsed: Option<&'a str>,
    created_at: &'a str,
}

fn movie_file<'a>(id: &'a str, title_id: &'a str, size: i64, dims: (i64, i64)) -> MovieFile<'a> {
    MovieFile {
        id,
        title_id,
        path: id,
        size,
        role: "primary",
        dims,
        quality_id: "",
        hdr: None,
        audio: None,
        audio_parsed: None,
        created_at: "2026-01-01T00:00:00Z",
    }
}

async fn insert_file(datastore: &StoreDatastore, file: MovieFile<'_>) {
    // The timestamp is spliced in as a literal so each engine reads it as its
    // own column type.
    let sql = format!(
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, role,
                                  video_width, video_height, quality_id, video_hdr_format,
                                  audio_codec, audio_codec_parsed, created_at)
         VALUES ({{}}, {{}}, {{}}, {{}}, 'complete', {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, '{}')",
        file.created_at
    );
    exec(
        datastore,
        &sql,
        vec![
            SqlArg::Text(file.id.into()),
            SqlArg::Text(file.title_id.into()),
            SqlArg::Text(file.path.into()),
            SqlArg::I64(file.size),
            SqlArg::Text(file.role.into()),
            SqlArg::I64(file.dims.0),
            SqlArg::I64(file.dims.1),
            SqlArg::Text(file.quality_id.into()),
            SqlArg::OptText(file.hdr.map(str::to_string)),
            SqlArg::OptText(file.audio.map(str::to_string)),
            SqlArg::OptText(file.audio_parsed.map(str::to_string)),
        ],
    )
    .await;
}

/// A source alias the way a stored row may spell it: other case, padded, or
/// broken up by the separators the sort's source test removes.
fn stored_source_spelling(alias: &str, variant: usize) -> String {
    let (head, tail) = alias.split_at(alias.len() / 2);
    match variant % 5 {
        0 => alias.to_ascii_uppercase(),
        1 => format!("  {head}_{tail} "),
        2 => format!("{}-{tail}", head.to_ascii_uppercase()),
        3 => alias
            .chars()
            .map(String::from)
            .collect::<Vec<_>>()
            .join(" "),
        _ => alias.to_string(),
    }
}

async fn insert_rating(
    datastore: &StoreDatastore,
    owner_column: &str,
    owner_id: &str,
    source: &str,
    normalized: f64,
) {
    exec(
        datastore,
        &format!(
            "INSERT INTO title_metadata_external_ratings
                 ({owner_column}, source, sort_index, value, normalized, votes)
             VALUES ({{}}, {{}}, 0, {{}}, {{}}, 10)"
        ),
        vec![
            SqlArg::Text(owner_id.into()),
            SqlArg::Text(source.into()),
            SqlArg::F64(normalized),
            SqlArg::F64(normalized),
        ],
    )
    .await;
}

async fn seed_sort_fixtures(datastore: &StoreDatastore) {
    let store = TitleStore::new(datastore.clone());
    for fixture in SORT_FIXTURES {
        let library = scryer_domain::default_library_id_for_facet(&fixture.facet);
        let root = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT id FROM library_roots WHERE library_id = {} LIMIT 1",
            &[SqlArg::Text(library.clone())],
        )
        .await
        .unwrap()
        .unwrap()
        .text("id")
        .unwrap();
        let mut title = fixture_title(fixture.id, fixture.name, fixture.year);
        title.library_id = library;
        title.facet = fixture.facet.clone();
        title.monitored = fixture.monitored;
        title.root_folder_id = root;
        TitleRepository::create(&store, title).await.unwrap();
        // Written directly so the stored values are exactly the fixture's,
        // whatever a title writer would normalize.
        exec(
            datastore,
            "UPDATE titles SET runtime_minutes = {}, popularity = {}, content_status = {}
              WHERE id = {}",
            vec![
                SqlArg::OptI64(fixture.runtime),
                SqlArg::OptF64(fixture.popularity),
                SqlArg::OptText(fixture.status.map(str::to_string)),
                SqlArg::Text(fixture.id.into()),
            ],
        )
        .await;
    }

    // Emberfall: the largest live primary file wins; the larger recycled file
    // and the larger additional file are not candidates.
    insert_file(
        datastore,
        MovieFile {
            hdr: Some("HDR10"),
            audio: Some("ac3"),
            audio_parsed: Some(" EAC3 "),
            ..movie_file("f-m-emberfall-small", "m-emberfall", 100, (1280, 720))
        },
    )
    .await;
    insert_file(
        datastore,
        MovieFile {
            hdr: Some("Dolby Vision"),
            audio: Some("TrueHD"),
            ..movie_file("f-m-emberfall-large", "m-emberfall", 900, (3840, 2160))
        },
    )
    .await;
    insert_file(
        datastore,
        MovieFile {
            path: "/media/Movies/Emberfall/.scryer-recycle/Emberfall.mkv",
            hdr: Some("HDR10+"),
            audio: Some("flac"),
            ..movie_file("f-m-emberfall-recycled", "m-emberfall", 5000, (7680, 4320))
        },
    )
    .await;
    insert_file(
        datastore,
        MovieFile {
            role: "additional",
            audio: Some("opus"),
            ..movie_file("f-m-emberfall-extra", "m-emberfall", 7000, (1920, 1080))
        },
    )
    .await;
    // Emberfall's twin: equal sizes, so the newer file wins.
    insert_file(
        datastore,
        MovieFile {
            hdr: Some("HDR10+"),
            audio: Some("dts"),
            created_at: "2026-01-01T00:00:00Z",
            ..movie_file("f-m-twin-older", "m-emberfall-two", 400, (1920, 1080))
        },
    )
    .await;
    insert_file(
        datastore,
        MovieFile {
            hdr: Some("dv"),
            audio: Some("AAC"),
            created_at: "2026-02-01T00:00:00Z",
            ..movie_file("f-m-twin-newer", "m-emberfall-two", 400, (1920, 1080))
        },
    )
    .await;
    // Cinderwake: equal sizes and times, so the id decides; negative sizes
    // count as zero.
    insert_file(
        datastore,
        MovieFile {
            quality_id: "1080p",
            audio_parsed: Some("aac"),
            ..movie_file("f-m-cinderwake-a", "m-cinderwake", -5, (0, 0))
        },
    )
    .await;
    insert_file(
        datastore,
        MovieFile {
            quality_id: " 720p ",
            hdr: Some("HLG"),
            audio: Some("  "),
            ..movie_file("f-m-cinderwake-b", "m-cinderwake", 0, (0, 0))
        },
    )
    .await;
    // Ashgrove: nothing a resolution can be read from, and blank codecs.
    insert_file(
        datastore,
        MovieFile {
            quality_id: "unknown",
            hdr: Some(" "),
            audio: Some(""),
            ..movie_file("f-m-ashgrove", "m-ashgrove", 250, (0, 0))
        },
    )
    .await;
    // Quillmere has no file. Thornhallow has only an additional file.
    insert_file(
        datastore,
        MovieFile {
            role: "additional",
            hdr: Some("HDR10"),
            audio: Some("ac3"),
            ..movie_file("f-m-thornhallow-extra", "m-thornhallow", 800, (1920, 1080))
        },
    )
    .await;
    insert_file(
        datastore,
        MovieFile {
            hdr: Some("HDR10"),
            audio: Some("EAC3"),
            ..movie_file("f-m-ashgrove-late", "m-ashgrove-late", 300, (1024, 576))
        },
    )
    .await;
    // Primary files of titles that are not movies describe no movie.
    insert_file(
        datastore,
        MovieFile {
            hdr: Some("Dolby Vision"),
            audio: Some("TrueHD"),
            ..movie_file("f-s-emberfall", "s-emberfall", 600, (3840, 2160))
        },
    )
    .await;
    insert_file(
        datastore,
        MovieFile {
            hdr: Some("HDR10"),
            audio: Some("flac"),
            ..movie_file("f-a-emberfall", "a-emberfall", 600, (1920, 1080))
        },
    )
    .await;

    // Ratings: every source alias of every sort, in several stored spellings,
    // spread over the titles so that titles hold none, one or several matching
    // rows, with ties and with 0-1 scores beside 0-10 ones.
    let normalized_values = [0.62, 7.4, 8.5, 0.74, 9.1, 3.0, 1.0];
    let mut stored = std::collections::HashSet::new();
    for (key_index, key) in RATING_KEYS.into_iter().enumerate() {
        let aliases = title_catalog_rating_sources_for_sort_key(key).unwrap();
        for (alias_index, alias) in aliases.iter().enumerate() {
            for variant in 0..3 {
                let title = &SORT_FIXTURES
                    [(key_index * 2 + alias_index * 3 + variant * 5) % SORT_FIXTURES.len()];
                let source = stored_source_spelling(alias, key_index + alias_index + variant);
                if !stored.insert((title.id, source.clone())) {
                    continue;
                }
                let normalized = normalized_values
                    [(key_index + 2 * alias_index + variant) % normalized_values.len()];
                insert_rating(datastore, "title_id", title.id, &source, normalized).await;
            }
        }
    }
    // Sources no sort reads, however close their spelling.
    for (title_id, source) in [
        ("m-quillmere", "imdbpro"),
        ("m-quillmere", "tmdb.org"),
        ("s-thornhallow", "the tvdb x"),
        ("a-ashgrove", "trakt/tv"),
        ("m-ashgrove", "IMDB\t"),
    ] {
        insert_rating(datastore, "title_id", title_id, source, 9.9).await;
    }
    // A movie entity's rating belongs to no title.
    exec(
        datastore,
        "INSERT INTO movie_entities (id, title, created_at, updated_at)
         VALUES ('me-quillmere', 'Quillmere Interlude', '2026-01-01T00:00:00Z',
                 '2026-01-01T00:00:00Z')",
        vec![],
    )
    .await;
    insert_rating(datastore, "movie_entity_id", "me-quillmere", "imdb", 9.9).await;
}

/// The movie-media join as it was: every primary movie file in the database.
fn previous_movie_media_subquery(dialect: TitleCatalogSqlDialect) -> String {
    let resolution = title_catalog_movie_media_resolution_expression("mf");
    let resolution_rank = title_catalog_movie_media_resolution_rank_expression("mf");
    let hdr_rank = title_catalog_movie_media_hdr_rank_expression("mf");
    format!(
        "SELECT title_id, resolution, hdr_format, audio_codec, resolution_rank, hdr_rank
           FROM (
                SELECT mf.title_id,
                       {resolution} AS resolution,
                       NULLIF(TRIM(mf.video_hdr_format), '') AS hdr_format,
                       COALESCE(NULLIF(TRIM(mf.audio_codec_parsed), ''), NULLIF(TRIM(mf.audio_codec), '')) AS audio_codec,
                       {resolution_rank} AS resolution_rank,
                       {hdr_rank} AS hdr_rank,
                       ROW_NUMBER() OVER (
                          PARTITION BY mf.title_id
                          ORDER BY CASE WHEN mf.size_bytes > 0 THEN mf.size_bytes ELSE 0 END DESC,
                                   mf.created_at DESC,
                                   mf.id DESC
                       ) AS media_row
                  FROM media_files mf
                  JOIN titles t ON t.id = mf.title_id
                 WHERE t.facet = 'movie'
                   AND {}
                   AND mf.role = 'primary'
           ) ranked
          WHERE media_row = 1",
        title_catalog_live_media_file_predicate(dialect, "mf")
    )
}

/// The rating join as it was: every rated title in every library.
fn previous_external_rating_subquery(sources: &[&str]) -> String {
    let source_list = sources
        .iter()
        .map(|source| sql_string_literal(source))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT ter.title_id,
                MAX(CASE
                    WHEN ter.normalized <= 1.0 THEN ter.normalized * 10.0
                    ELSE ter.normalized
                END) AS normalized
           FROM title_metadata_external_ratings ter
          WHERE LOWER(REPLACE(REPLACE(REPLACE(TRIM(ter.source), ' ', ''), '_', ''), '-', '')) IN ({source_list})
          GROUP BY ter.title_id"
    )
}

/// Each sort's ORDER BY as it was, ending in the full ascending title order.
fn previous_order_sql(sort: &TitleCatalogSort, dialect: TitleCatalogSqlDialect) -> String {
    let direction = match sort.direction {
        SortDirection::Asc => "ASC",
        SortDirection::Desc => "DESC",
    };
    let collation = match dialect {
        TitleCatalogSqlDialect::Sqlite => "BINARY",
        TitleCatalogSqlDialect::Postgres => "\"C\"",
    };
    let tie = format!(
        "catalog_sort_key COLLATE {collation} ASC, \
         catalog_name_tie_key COLLATE {collation} ASC, \
         CASE WHEN year IS NULL THEN 1 ELSE 0 END ASC, year ASC, \
         titles.id COLLATE {collation} ASC"
    );
    let missing_last = |expression: &str| {
        format!(
            "ORDER BY CASE WHEN {expression} IS NULL THEN 1 ELSE 0 END ASC, \
             {expression} {direction}, {tie}"
        )
    };
    match sort.key {
        TitleCatalogSortKey::Year => missing_last("year"),
        TitleCatalogSortKey::Runtime => missing_last("runtime_minutes"),
        TitleCatalogSortKey::Popularity => missing_last("popularity"),
        TitleCatalogSortKey::Status => {
            let raw_expression = "LOWER(TRIM(COALESCE(content_status, '')))";
            let expression = format!(
                "CASE {raw_expression} \
                 WHEN 'returning' THEN 'continuing' \
                 WHEN 'finished' THEN 'ended' \
                 ELSE {raw_expression} END"
            );
            format!(
                "ORDER BY CASE WHEN {expression} = '' THEN 1 ELSE 0 END {direction}, \
                 {expression} {direction}, {tie}"
            )
        }
        TitleCatalogSortKey::MediaResolution => missing_last("catalog_movie_media.resolution_rank"),
        TitleCatalogSortKey::MediaHdr => missing_last("catalog_movie_media.hdr_rank"),
        TitleCatalogSortKey::MediaAudioCodec => format!(
            "ORDER BY CASE WHEN NULLIF(TRIM(COALESCE(catalog_movie_media.audio_codec, '')), '') IS NULL THEN 1 ELSE 0 END ASC,
             LOWER(TRIM(COALESCE(catalog_movie_media.audio_codec, ''))) {direction}, {tie}"
        ),
        key if title_catalog_rating_sources_for_sort_key(key).is_some() => {
            missing_last("catalog_external_rating.normalized")
        }
        other => panic!("no previous order kept for {other:?}"),
    }
}

/// The page query as it was, unpaged.
fn previous_page_sql(
    facet: Option<MediaFacet>,
    library_ids: &[String],
    filter: &TitleCatalogFilter,
    sort: &TitleCatalogSort,
    dialect: TitleCatalogSqlDialect,
) -> (String, Vec<SqlArg>) {
    let join = match sort.key {
        TitleCatalogSortKey::Year
        | TitleCatalogSortKey::Runtime
        | TitleCatalogSortKey::Popularity
        | TitleCatalogSortKey::Status => String::new(),
        TitleCatalogSortKey::MediaResolution
        | TitleCatalogSortKey::MediaHdr
        | TitleCatalogSortKey::MediaAudioCodec => format!(
            " LEFT JOIN ({}) catalog_movie_media ON catalog_movie_media.title_id = titles.id",
            previous_movie_media_subquery(dialect)
        ),
        key => {
            let sources = title_catalog_rating_sources_for_sort_key(key)
                .unwrap_or_else(|| panic!("no previous form kept for {key:?}"));
            format!(
                " LEFT JOIN ({}) catalog_external_rating ON catalog_external_rating.title_id = titles.id",
                previous_external_rating_subquery(sources)
            )
        }
    };
    let (where_sql, args) =
        build_title_catalog_where_sql(facet, library_ids, None, filter, dialect);
    let order = previous_order_sql(sort, dialect);
    (
        format!("SELECT titles.id FROM titles{join} WHERE {where_sql} {order}"),
        args,
    )
}

async fn ids(datastore: &StoreDatastore, sql: &str, args: &[SqlArg], context: &str) -> Vec<String> {
    SqlRuntime::fetch_all(datastore.read_exec(), sql, args)
        .await
        .unwrap_or_else(|error| panic!("{context}: {error}\n{sql}"))
        .into_iter()
        .map(|row| row.text("id").unwrap())
        .collect()
}

fn sorted_keys() -> Vec<TitleCatalogSortKey> {
    INDEXED_KEYS
        .into_iter()
        .map(|(key, _)| key)
        .chain(MOVIE_MEDIA_KEYS)
        .chain(RATING_KEYS)
        .collect()
}

async fn assert_catalog_sorts_match_previous_queries(datastore: StoreDatastore) {
    let dialect = title_catalog_dialect_for_datastore(&datastore);
    seed_sort_fixtures(&datastore).await;
    let movie = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let series = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let anime = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let every_library = vec![movie.clone(), series.clone(), anime.clone()];

    type Scope<'a> = (
        &'a str,
        Option<MediaFacet>,
        Vec<String>,
        TitleCatalogFilter,
        usize,
    );
    let count = |keep: &dyn Fn(&SortFixture) -> bool| {
        SORT_FIXTURES.iter().filter(|fixture| keep(fixture)).count()
    };
    let scopes: Vec<Scope<'_>> = vec![
        (
            "movie library, movie facet",
            Some(MediaFacet::Movie),
            vec![movie.clone()],
            TitleCatalogFilter::default(),
            count(&|f| f.facet == MediaFacet::Movie),
        ),
        (
            "movie library, every facet",
            None,
            vec![movie.clone()],
            TitleCatalogFilter::default(),
            count(&|f| f.facet == MediaFacet::Movie),
        ),
        (
            "series library",
            Some(MediaFacet::Series),
            vec![series.clone()],
            TitleCatalogFilter::default(),
            count(&|f| f.facet == MediaFacet::Series),
        ),
        (
            "two libraries",
            None,
            vec![movie.clone(), anime.clone()],
            TitleCatalogFilter::default(),
            count(&|f| f.facet != MediaFacet::Series),
        ),
        (
            "three libraries",
            None,
            every_library.clone(),
            TitleCatalogFilter::default(),
            SORT_FIXTURES.len(),
        ),
        (
            "three libraries, monitored",
            None,
            every_library.clone(),
            TitleCatalogFilter {
                monitored: Some(true),
                ..Default::default()
            },
            count(&|f| f.monitored),
        ),
        (
            "movie library, picked titles",
            Some(MediaFacet::Movie),
            vec![movie.clone()],
            TitleCatalogFilter {
                title_ids: vec![
                    "m-emberfall".into(),
                    "m-cinderwake".into(),
                    "m-quillmere".into(),
                    "s-emberfall".into(),
                ],
                ..Default::default()
            },
            3,
        ),
        (
            "no library",
            None,
            Vec::new(),
            TitleCatalogFilter::default(),
            0,
        ),
    ];

    for (label, facet, libraries, filter, members) in &scopes {
        for key in sorted_keys() {
            let mut by_direction = Vec::new();
            for direction in [SortDirection::Asc, SortDirection::Desc] {
                let sort = TitleCatalogSort {
                    key,
                    direction,
                    ..Default::default()
                };
                let context = format!("{label}, {key:?} {direction:?}");
                let mut paged = Vec::new();
                for offset in (0..members + 4).step_by(4) {
                    let (sql, args) = build_title_catalog_page_sql(
                        facet.clone(),
                        libraries,
                        None,
                        filter,
                        sort.clone(),
                        4,
                        offset,
                        dialect,
                        TitleListProjection::default(),
                    );
                    paged.extend(ids(&datastore, &sql, &args, &context).await);
                }
                let (sql, args) =
                    previous_page_sql(facet.clone(), libraries, filter, &sort, dialect);
                let previous = ids(&datastore, &sql, &args, &context).await;
                assert_eq!(previous.len(), *members, "{context}: scope size");
                assert_eq!(paged, previous, "{context} vs previous SQL");
                by_direction.push(paged);
            }
            // Every sort has at least two distinct values across the whole
            // fixture set, so a sort that ignored its key would show here.
            if *label == "three libraries" {
                assert_ne!(
                    by_direction[0], by_direction[1],
                    "{key:?} should order the fixtures differently in each direction"
                );
            }
        }
    }
}

async fn sqlite_plan(datastore: &StoreDatastore, sql: &str, args: &[SqlArg]) -> Vec<String> {
    SqlRuntime::fetch_all(
        datastore.read_exec(),
        &format!("EXPLAIN QUERY PLAN {sql}"),
        args,
    )
    .await
    .unwrap_or_else(|error| panic!("query plan should load: {error}\n{sql}"))
    .iter()
    .map(|row| row.text("detail").unwrap())
    .collect()
}

/// One library's page of an indexed sort is read in order from that sort's
/// index, and a scoped join reads its rows through the scope.
async fn assert_sqlite_sort_plans(datastore: &StoreDatastore, state: &str) {
    let movie = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let page = |facet: Option<MediaFacet>, key, direction| {
        build_title_catalog_page_sql(
            facet,
            std::slice::from_ref(&movie),
            None,
            &TitleCatalogFilter::default(),
            TitleCatalogSort {
                key,
                direction,
                ..Default::default()
            },
            50,
            100,
            TitleCatalogSqlDialect::Sqlite,
            TitleListProjection::default(),
        )
    };
    for (key, name) in INDEXED_KEYS {
        for (direction, suffix) in [(SortDirection::Asc, "asc"), (SortDirection::Desc, "desc")] {
            for facet in [None, Some(MediaFacet::Movie)] {
                let (sql, args) = page(facet.clone(), key, direction);
                let plan = sqlite_plan(datastore, &sql, &args).await;
                let index = format!("idx_titles_library_{name}_order_{suffix}");
                assert!(
                    plan.iter().any(|detail| detail.contains(&index)),
                    "{state}: {key:?} {direction:?} facet {facet:?} should read {index}: {plan:?}"
                );
                assert!(
                    !plan.iter().any(|detail| detail.contains("TEMP B-TREE")),
                    "{state}: {key:?} {direction:?} facet {facet:?} should not sort: {plan:?}"
                );
            }
        }
    }
    for key in RATING_KEYS {
        let (sql, args) = page(Some(MediaFacet::Movie), key, SortDirection::Desc);
        assert!(sql.starts_with("WITH scoped_titles AS ("), "{sql}");
        let plan = sqlite_plan(datastore, &sql, &args).await;
        assert!(
            plan.iter().any(|detail| {
                detail.contains("idx_title_metadata_external_ratings_title_source_key")
            }),
            "{state}: {key:?} should read ratings by title and source: {plan:?}"
        );
        assert!(
            !plan
                .iter()
                .any(|detail| detail.contains("SCAN ter") || detail.contains("SCAN TABLE ter")),
            "{state}: {key:?} should not scan every rating: {plan:?}"
        );
    }
    for key in MOVIE_MEDIA_KEYS {
        let (sql, args) = page(Some(MediaFacet::Movie), key, SortDirection::Desc);
        assert!(sql.starts_with("WITH scoped_titles AS ("), "{sql}");
        let plan = sqlite_plan(datastore, &sql, &args).await;
        assert!(
            !plan
                .iter()
                .any(|detail| detail.contains("SCAN mf") || detail.contains("SCAN TABLE mf")),
            "{state}: {key:?} should not scan every file: {plan:?}"
        );
    }
}

#[tokio::test]
async fn sqlite_catalog_sorts_match_the_queries_they_replaced() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
        &pool, None, true,
    )
    .await
    .unwrap();
    let datastore = StoreDatastore::sqlite(pool, Arc::new(tokio::sync::Mutex::new(())));
    assert_sqlite_sort_plans(&datastore, "empty, without statistics").await;
    assert_catalog_sorts_match_previous_queries(datastore.clone()).await;
    assert_sqlite_sort_plans(&datastore, "seeded, without statistics").await;
}

#[tokio::test]
async fn postgres_catalog_sorts_match_the_queries_they_replaced() {
    let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
        return;
    };
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("catalog_sort_scope_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .unwrap();
    scryer_infrastructure_datastore::postgres::replay_source_catalog_for_fresh_install(&pool, None)
        .await
        .unwrap();
    assert_catalog_sorts_match_previous_queries(StoreDatastore::Postgres { pool: pool.clone() })
        .await;
    pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}
