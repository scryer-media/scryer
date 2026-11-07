#![recursion_limit = "256"]

//! Background acquisition walk benchmark at library scale.
//!
//! Seeds about one hundred thousand media files into the SQLite datastore: a
//! movie library where most movies carry one file, and a series library whose
//! episodes carry the rest, with a share of each missing or below cutoff. Then
//! it times every derivation sub-pass the walk depends on and, when the cache
//! is present, the cache build, an idle refresh and a one-title patch.
//!
//! Ignored by default: it is a measurement, not a regression gate. Run it with
//! `cargo nextest run -p scryer --test integration_acquisition_walk_bench
//! --run-ignored only`, and set `BENCH_OUT` to a file path to keep the report.
//! Every name is synthetic.

mod common;

use common::TestContext;

const MOVIES: usize = 30_000;
/// One movie in ten has no file.
const MOVIE_MISSING_EVERY: usize = 10;
const SERIES: usize = 1_500;
const SEASONS_PER_SERIES: usize = 3;
const EPISODES_PER_SEASON: usize = 18;
/// One episode in eight has no file.
const EPISODE_MISSING_EVERY: usize = 8;
/// One file in five sits at 720p, below a 1080p cutoff.
const LOW_QUALITY_EVERY: usize = 5;

/// The facet's library, and a synthetic root inside it every seeded title
/// sits under.
async fn library_and_root(ctx: &TestContext, facet: &str) -> (String, String) {
    let row: (String,) = sqlx::query_as("SELECT id FROM libraries WHERE facet = ? LIMIT 1")
        .bind(facet)
        .fetch_one(&ctx.db.pool)
        .await
        .expect("seeded library for facet");
    let root_id = format!("synthetic-root-{facet}");
    let path = format!("/synthetic/{facet}");
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(&root_id)
    .bind(&row.0)
    .bind(&path)
    .bind(&path)
    .bind(&now)
    .bind(&now)
    .execute(&ctx.db.pool)
    .await
    .expect("insert synthetic root");
    (row.0, root_id)
}

struct Seeded {
    titles: usize,
    episodes: usize,
    media_files: usize,
}

async fn seed(ctx: &TestContext) -> Seeded {
    let (movie_library, movie_root) = library_and_root(ctx, "movie").await;
    let (series_library, series_root) = library_and_root(ctx, "series").await;
    let now = chrono::Utc::now();
    let created = (now - chrono::Duration::days(400)).to_rfc3339();
    let recent_add = (now - chrono::Duration::days(1)).to_rfc3339();
    let mut tx = ctx.db.pool.begin().await.expect("begin seed");
    let mut titles = 0usize;
    let mut episodes = 0usize;
    let mut files = 0usize;

    let insert_title = "INSERT INTO titles (id, name, name_normalized, facet, monitored, created_at, slug, year, \
         first_aired, digital_release_date, metadata_fetched_at, min_availability, library_id, root_folder_id) \
         VALUES (?, ?, ?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
    let insert_file = "INSERT INTO media_files (id, title_id, file_path, size_bytes, scan_status, created_at, \
         video_width, video_height, role) VALUES (?, ?, ?, 1000000, 'scanned', ?, ?, ?, 'primary')";

    for index in 0..MOVIES {
        let id = format!("synthetic-movie-{index:06}");
        let year = 1990 + (index % 30) as i32;
        let name = format!("Synthetic Fixture {index:05} ({year})");
        let created_at = if index % 500 == 0 {
            &recent_add
        } else {
            &created
        };
        sqlx::query(insert_title)
            .bind(&id)
            .bind(&name)
            .bind(name.to_lowercase())
            .bind("movie")
            .bind(created_at)
            .bind(format!("synthetic-fixture-{index:05}"))
            .bind(year)
            .bind(format!("{year}-06-01"))
            .bind(format!("{year}-09-01"))
            .bind(&created)
            .bind("released")
            .bind(&movie_library)
            .bind(&movie_root)
            .execute(&mut *tx)
            .await
            .expect("insert movie");
        titles += 1;
        if index % MOVIE_MISSING_EVERY == 0 {
            continue;
        }
        let (width, height) = if index % LOW_QUALITY_EVERY == 1 {
            (1280, 720)
        } else {
            (1920, 1080)
        };
        sqlx::query(insert_file)
            .bind(format!("synthetic-movie-file-{index:06}"))
            .bind(&id)
            .bind(format!("/synthetic/movies/{id}/{id}.mkv"))
            .bind(&created)
            .bind(width)
            .bind(height)
            .execute(&mut *tx)
            .await
            .expect("insert movie file");
        files += 1;
    }

    for series in 0..SERIES {
        let id = format!("synthetic-series-{series:05}");
        let year = 1995 + (series % 28) as i32;
        let name = format!("Synthetic Fixture Series {series:05} ({year})");
        let created_at = if series % 300 == 0 {
            &recent_add
        } else {
            &created
        };
        sqlx::query(insert_title)
            .bind(&id)
            .bind(&name)
            .bind(name.to_lowercase())
            .bind("series")
            .bind(created_at)
            .bind(format!("synthetic-fixture-series-{series:05}"))
            .bind(year)
            .bind(format!("{year}-01-01"))
            .bind(Option::<String>::None)
            .bind(&created)
            .bind(Option::<String>::None)
            .bind(&series_library)
            .bind(&series_root)
            .execute(&mut *tx)
            .await
            .expect("insert series");
        titles += 1;
        for season in 1..=SEASONS_PER_SERIES {
            let collection_id = format!("{id}-season-{season}");
            sqlx::query(
                "INSERT INTO collections (id, title_id, collection_type, collection_index, created_at, monitored) \
                 VALUES (?, ?, 'season', ?, ?, 1)",
            )
            .bind(&collection_id)
            .bind(&id)
            .bind(season.to_string())
            .bind(&created)
            .execute(&mut *tx)
            .await
            .expect("insert season");
            for episode in 1..=EPISODES_PER_SEASON {
                let episode_id = format!("{id}-s{season:02}e{episode:02}");
                // The newest season of every twentieth series aired over the
                // last ten days, so the hot lane has work in it.
                let air_date = if series % 20 == 0 && season == SEASONS_PER_SERIES {
                    (now - chrono::Duration::days(((EPISODES_PER_SEASON - episode) % 10) as i64))
                        .format("%Y-%m-%d")
                        .to_string()
                } else {
                    format!(
                        "{}-{:02}-{:02}",
                        year + season as i32 - 1,
                        1 + episode % 12,
                        1 + episode
                    )
                };
                sqlx::query(
                    "INSERT INTO episodes (id, title_id, collection_id, episode_type, episode_number, \
                     season_number, title, air_date, created_at, monitored) \
                     VALUES (?, ?, ?, 'standard', ?, ?, ?, ?, ?, 1)",
                )
                .bind(&episode_id)
                .bind(&id)
                .bind(&collection_id)
                .bind(episode.to_string())
                .bind(season.to_string())
                .bind(format!("Synthetic Episode {episode:02}"))
                .bind(&air_date)
                .bind(&created)
                .execute(&mut *tx)
                .await
                .expect("insert episode");
                episodes += 1;
                if episodes.is_multiple_of(EPISODE_MISSING_EVERY) {
                    continue;
                }
                let file_id = format!("{episode_id}-file");
                let (width, height) = if episodes % LOW_QUALITY_EVERY == 1 {
                    (1280, 720)
                } else {
                    (1920, 1080)
                };
                sqlx::query(insert_file)
                    .bind(&file_id)
                    .bind(&id)
                    .bind(format!("/synthetic/series/{id}/{episode_id}.mkv"))
                    .bind(&created)
                    .bind(width)
                    .bind(height)
                    .execute(&mut *tx)
                    .await
                    .expect("insert episode file");
                sqlx::query(
                    "INSERT INTO file_episode_map (file_id, episode_id, role) VALUES (?, ?, 'primary')",
                )
                .bind(&file_id)
                .bind(&episode_id)
                .execute(&mut *tx)
                .await
                .expect("map episode file");
                files += 1;
            }
        }
    }
    tx.commit().await.expect("commit seed");
    Seeded {
        titles,
        episodes,
        media_files: files,
    }
}

fn median(mut values: Vec<std::time::Duration>) -> std::time::Duration {
    values.sort();
    values[values.len() / 2]
}

fn ms(value: std::time::Duration) -> String {
    format!("{:.1} ms", value.as_secs_f64() * 1000.0)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "library-scale measurement; run explicitly"]
async fn background_walk_derivation_at_one_hundred_thousand_files() {
    let ctx = TestContext::new().await;
    let seeded = seed(&ctx).await;
    let mut report = format!(
        "fixture: {} titles, {} episodes, {} media files\n",
        seeded.titles, seeded.episodes, seeded.media_files
    );

    const RUNS: usize = 3;
    let mut profiles = Vec::new();
    for _ in 0..RUNS {
        profiles.push(
            ctx.app
                .profile_acquisition_target_derivation()
                .await
                .expect("profile derivation"),
        );
    }
    let last = profiles.last().expect("profiles").clone();
    let pass =
        |select: fn(&scryer_application::AcquisitionDerivationProfile) -> std::time::Duration| {
            median(profiles.iter().map(select).collect())
        };
    report.push_str(&format!(
        "full derivation (median of {RUNS}): missing {}, cutoff_quality {}, format_scoring {}, filters {}, total {}\n",
        ms(pass(|p| p.missing)),
        ms(pass(|p| p.cutoff_quality)),
        ms(pass(|p| p.format_scoring)),
        ms(pass(|p| p.filters)),
        ms(pass(|p| p.missing + p.cutoff_quality + p.format_scoring + p.filters)),
    ));
    report.push_str(&format!(
        "targets: {} ({} hot), uncached Vec<AcquisitionTarget> estimate {} bytes\n",
        last.targets, last.hot_targets, last.target_vec_bytes
    ));

    // The walk's own read: built once, then reused or patched.
    let timed = |label: &str, elapsed: std::time::Duration, outcome: (usize, &str)| {
        format!(
            "{label}: {} ({} targets, source {})\n",
            ms(elapsed),
            outcome.0,
            outcome.1
        )
    };
    let started = std::time::Instant::now();
    let built = ctx
        .app
        .refresh_acquisition_target_cache()
        .await
        .expect("build cache");
    report.push_str(&timed("cache build", started.elapsed(), built));
    let mut idle = Vec::new();
    let mut idle_outcome = built;
    for _ in 0..RUNS {
        let started = std::time::Instant::now();
        idle_outcome = ctx
            .app
            .refresh_acquisition_target_cache()
            .await
            .expect("idle read");
        idle.push(started.elapsed());
    }
    report.push_str(&timed(
        "idle walk read (median)",
        median(idle),
        idle_outcome,
    ));
    for title_id in ["synthetic-movie-000001", "synthetic-series-00001"] {
        ctx.app.mark_acquisition_title_changed(title_id);
        let started = std::time::Instant::now();
        let patched = ctx
            .app
            .refresh_acquisition_target_cache()
            .await
            .expect("patch one title");
        report.push_str(&timed(
            &format!("one-title patch ({title_id})"),
            started.elapsed(),
            patched,
        ));
    }
    let stats = ctx.app.acquisition_target_cache_stats().await;
    report.push_str(&format!(
        "cache: {} entries, {} bytes estimated, {} titles with a time recheck\n",
        stats.entries, stats.bytes, stats.titles_with_recheck
    ));

    println!("{report}");
    if let Ok(path) = std::env::var("BENCH_OUT") {
        std::fs::write(path, &report).expect("write bench report");
    }
}
