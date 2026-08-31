import assert from "node:assert/strict";
import test from "node:test";

import type {
  MovieEntity,
  SeriesMovieLink,
  TitleCollection,
} from "@/components/containers/series-overview-container";
import { buildSeriesTimelineItems, seasonHeading } from "./helpers.ts";

function collection(
  id: string,
  collectionIndex: string,
  collectionType = "season",
): TitleCollection {
  return {
    id,
    titleId: "title-1",
    collectionType,
    collectionIndex,
    label: collectionIndex === "0" ? "Specials" : `Season ${collectionIndex}`,
    orderedPath: null,
    narrativeOrder: null,
    fileSizeBytes: null,
    firstEpisodeNumber: null,
    lastEpisodeNumber: null,
    monitored: true,
    episodesOwned: null,
    episodesMonitored: null,
    episodesTotal: null,
    episodeRecordsTotal: null,
    createdAt: "2026-01-01T00:00:00Z",
  };
}

function movieEntity(id: string, title: string): MovieEntity {
  return {
    id,
    title,
    sortTitle: null,
    slug: null,
    year: 2019,
    overview: null,
    posterUrl: null,
    backgroundUrl: null,
    language: null,
    runtimeMinutes: 60,
    contentStatus: null,
    studio: null,
    digitalReleaseDate: null,
    imdbId: null,
    tvdbId: null,
    tmdbId: null,
    malId: null,
    anidbId: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };
}

function link(
  id: string,
  overrides: Partial<SeriesMovieLink> = {},
): SeriesMovieLink {
  return {
    id,
    seriesTitleId: "title-1",
    movie: movieEntity(`${id}-movie`, id),
    placement: null,
    narrativeOrder: null,
    afterSeason: null,
    beforeSeason: null,
    linkedEpisodeId: null,
    associationConfidence: null,
    continuityStatus: null,
    movieForm: null,
    confidence: null,
    signalSummary: null,
    source: null,
    monitoringOverride: null,
    metadataActive: true,
    monitored: true,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function itemIds(items: ReturnType<typeof buildSeriesTimelineItems>) {
  return items.map((item) => (
    item.kind === "collection" ? item.collection.id : item.link.id
  ));
}

test("buildSeriesTimelineItems places narrative movies between seasons", () => {
  const items = buildSeriesTimelineItems(
    [collection("season-2", "2"), collection("season-3", "3")],
    [link("movie-2-5", { narrativeOrder: "2.5" })],
  );

  assert.deepEqual(itemIds(items), ["season-3", "movie-2-5", "season-2"]);
});

test("buildSeriesTimelineItems keeps specials-placement movies above Specials", () => {
  const items = buildSeriesTimelineItems(
    [collection("specials", "0", "specials"), collection("season-1", "1")],
    [link("movie-specials", { placement: "specials", afterSeason: 0 })],
  );

  assert.deepEqual(itemIds(items), ["season-1", "movie-specials", "specials"]);
});

test("seasonHeading localizes Specials regardless of an upstream label", () => {
  const t = (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) =>
    key === "title.specials"
      ? "Specials"
      : `Season ${String(values?.number ?? "")}`.trim();

  assert.equal(
    seasonHeading(
      { ...collection("specials", "0", "SPECIALS"), label: "特別編" },
      t,
    ),
    "Specials",
  );
  assert.equal(
    seasonHeading(
      { ...collection("season-zero", "0", "SEASON"), label: "特別編" },
      t,
    ),
    "Specials",
  );
  assert.equal(
    seasonHeading(
      { ...collection("season-2", "2"), label: "Hidden Inventory" },
      t,
    ),
    "Season 2: Hidden Inventory",
  );
});

test("buildSeriesTimelineItems ignores linked S00 episode for movie placement", () => {
  const items = buildSeriesTimelineItems(
    [
      collection("season-2", "2"),
      collection("season-3", "3"),
      collection("specials", "0", "specials"),
    ],
    [
      link("linked-movie", {
        narrativeOrder: "2.5",
        linkedEpisodeId: "episode-s00e13",
      }),
    ],
  );

  assert.deepEqual(itemIds(items), [
    "season-3",
    "linked-movie",
    "season-2",
    "specials",
  ]);
});

test("buildSeriesTimelineItems renders movie-only timelines", () => {
  const items = buildSeriesTimelineItems([], [link("movie-only")]);

  assert.deepEqual(itemIds(items), ["movie-only"]);
});
