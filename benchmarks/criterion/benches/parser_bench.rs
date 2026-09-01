use criterion::{Criterion, criterion_group, criterion_main};
use scryer_release_parser::{
    ContextAlias, ContextEpisode, ContextFacetHint, ContextTitle, ReleaseParseContext,
    analyze_release_against_targets, analyze_release_for_target,
};
use std::hint::black_box;

fn context(facet_hint: ContextFacetHint, title: &str) -> ReleaseParseContext {
    ReleaseParseContext {
        facet_hint,
        title: ContextTitle {
            name: title.to_string(),
        },
        aliases: Vec::new(),
        known_years: Vec::new(),
        imdb_ids: Vec::new(),
        episodes: Vec::new(),
    }
}

fn starfall_context() -> ReleaseParseContext {
    let mut target = context(ContextFacetHint::Anime, "Starfall Iron Eclipse");
    target.aliases = vec![
        ContextAlias {
            name: "Starfall".to_string(),
        },
        ContextAlias {
            name: "Starfall - Iron Eclipse".to_string(),
        },
        ContextAlias {
            name: "Starfall: Iron Eclipse".to_string(),
        },
    ];
    target.known_years.push(2022);
    target.episodes = vec![ContextEpisode {
        absolute_number: Some(14),
        title: Some("The Last 9 Signals".to_string()),
        ..Default::default()
    }];
    target
}

fn nightfall_context() -> ReleaseParseContext {
    let mut target = context(ContextFacetHint::Anime, "Midnight Alloy Dark Signal");
    target.aliases = vec![
        ContextAlias {
            name: "Midnight Alloy Dark".to_string(),
        },
        ContextAlias {
            name: "Midnight Alloy Dark Signal".to_string(),
        },
        ContextAlias {
            name: "Midnight Alloy Kage Requiem".to_string(),
        },
        ContextAlias {
            name: "Midnight Alloy".to_string(),
        },
    ];
    target.known_years.push(2022);
    target
}

fn parser_single_target_bench(c: &mut Criterion) {
    let target = starfall_context();
    let raw = "[Studio Nova] Starfall - Iron Eclipse - 014 - The Last 9 Signals [BD][1080p][HEVC 10bit x265][AAC] [Dual Audio][ENG Subs]";

    c.bench_function("parser/single_target_starfall_context", |b| {
        b.iter(|| analyze_release_for_target(black_box(raw), black_box(&target)))
    });
}

fn parser_multi_target_bench(c: &mut Criterion) {
    let mut classic_starfall = context(ContextFacetHint::Anime, "Starfall");
    classic_starfall.aliases = vec![ContextAlias {
        name: "Starfall".to_string(),
    }];
    classic_starfall.episodes = vec![ContextEpisode {
        absolute_number: Some(14),
        title: Some("Training".to_string()),
        ..Default::default()
    }];
    let mut neon_cipher = context(ContextFacetHint::Movie, "Neon Cipher");
    neon_cipher.known_years.push(2010);
    let targets = vec![classic_starfall, starfall_context(), neon_cipher];
    let raw = "[Studio Nova] Starfall - 014 - The Last 9 Signals [BD][1080p][HEVC 10bit x265][AAC]";

    c.bench_function("parser/multi_target_context_bank", |b| {
        b.iter(|| analyze_release_against_targets(black_box(raw), black_box(&targets)))
    });
}

fn parser_beam_dedupe_bench(c: &mut Criterion) {
    let target = nightfall_context();
    let raw = "[EMBER] MIDNIGHT ALLOY‼ Dark Signal (2022) (Season 1 | Part 02) [1080p] [Dual Audio HEVC 10 bits WEBRip AAC] (Midnight Alloy Kage Requiem) (Batch)";

    c.bench_function("parser/beam_dedupe_alias_dense", |b| {
        b.iter(|| analyze_release_for_target(black_box(raw), black_box(&target)))
    });
}

fn parser_trash_fact_index_bench(c: &mut Criterion) {
    let target = starfall_context();
    let raw =
        "[Studio Nova] Starfall - Iron Eclipse - 014 [1080p][WEB-DL][The Upscaler][PROPER][REPACK]";

    c.bench_function("parser/trash_fact_anchor_index", |b| {
        b.iter(|| analyze_release_for_target(black_box(raw), black_box(&target)))
    });
}

criterion_group!(
    benches,
    parser_single_target_bench,
    parser_multi_target_bench,
    parser_beam_dedupe_bench,
    parser_trash_fact_index_bench
);
criterion_main!(benches);
