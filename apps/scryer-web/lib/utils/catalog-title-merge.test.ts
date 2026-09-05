import assert from "node:assert/strict";
import test from "node:test";

import type { TitleRecord } from "@/lib/types";
import { mergePreferLoadedImageFields } from "./catalog-title-merge.ts";

/**
 * A title as the side panel loads it: the movie side-panel selection is much
 * wider than the catalog list projection, so every panel-only field is set.
 */
const hydratedPanelTitle: TitleRecord = {
  id: "title-1",
  name: "Placeholder Feature",
  facet: "MOVIE",
  libraryId: "library-1",
  libraryName: "Placeholder Library",
  librarySlug: "placeholder-library",
  monitored: true,
  tags: ["placeholder-tag"],
  createdAt: "2026-01-01T00:00:00Z",
  year: 2026,
  overview: "A placeholder synopsis.",
  sortTitle: "placeholder feature",
  slug: "placeholder-feature",
  imdbId: "tt0000001",
  externalIds: [{ source: "tmdb", value: "1" }],
  sizeBytes: 1024,
  contentStatus: "RELEASED",
  posterUrl: "https://media.example.test/poster.jpg",
  posterSourceUrl: "https://media.example.test/poster-source.jpg",
  backgroundUrl: "https://media.example.test/background.jpg",
  backgroundSourceUrl: "https://media.example.test/background-source.jpg",
  runtimeMinutes: 101,
  ratings: { rating: 7.5, ratingSources: [], externalRatings: [] },
  canonicalTags: [],
  language: "en",
  firstAired: "2026-01-01",
  network: "Placeholder Network",
  studio: "Placeholder Studio",
  country: "US",
  aliases: ["Placeholder Feature (2026)"],
  metadataLanguage: "en",
  requiredAudioLanguagesOverride: ["en"],
  effectiveRequiredAudioLanguages: ["en"],
  inheritsRequiredAudioLanguages: false,
  metadataFetchedAt: "2026-01-02T00:00:00Z",
  qualityProfileId: "profile-1",
  rootFolderId: "root-1",
  rootFolderPath: "/placeholder/movies",
  monitorType: "ALL",
  collections: [],
  mediaFiles: [],
  credits: [],
  playbackLinks: [
    {
      connectionId: "connection-1",
      displayName: "Placeholder Server",
      provider: "JELLYFIN",
      href: "https://watch.example.test/web/index.html#!/details?id=item-1",
    },
  ],
};

/**
 * The same title as a reactive catalog list refresh returns it: the list
 * projection carries no panel-only fields at all.
 */
const listRefreshTitle: TitleRecord = {
  id: "title-1",
  name: "Placeholder Feature",
  facet: "MOVIE",
  libraryId: "library-1",
  libraryName: "Placeholder Library",
  librarySlug: "placeholder-library",
  monitored: true,
  tags: ["placeholder-tag"],
  createdAt: "2026-01-01T00:00:00Z",
  year: 2026,
  slug: "placeholder-feature",
  contentStatus: "RELEASED",
  posterUrl: "https://media.example.test/poster.jpg",
  posterSourceUrl: "https://media.example.test/poster-source.jpg",
  backgroundUrl: "https://media.example.test/background.jpg",
  backgroundSourceUrl: "https://media.example.test/background-source.jpg",
  metadataLanguage: "en",
  metadataFetchedAt: "2026-01-02T00:00:00Z",
  qualityProfileId: "profile-1",
  rootFolderId: "root-1",
  monitorType: "ALL",
};

test("a catalog list refresh keeps the media server playback links the panel loaded", () => {
  const merged = mergePreferLoadedImageFields(
    hydratedPanelTitle,
    listRefreshTitle,
  );

  assert.deepEqual(merged.playbackLinks, hydratedPanelTitle.playbackLinks);
});

test("a catalog list refresh blanks no field the selected panel already loaded", () => {
  const merged = mergePreferLoadedImageFields(
    hydratedPanelTitle,
    listRefreshTitle,
  ) as Record<string, unknown>;

  for (const [field, value] of Object.entries(hydratedPanelTitle)) {
    if (value === undefined) {
      continue;
    }
    assert.notEqual(
      merged[field],
      undefined,
      `${field} was blanked by a catalog list refresh`,
    );
  }
});

test("a refresh that does carry panel fields still wins over the loaded copy", () => {
  const merged = mergePreferLoadedImageFields(hydratedPanelTitle, {
    ...listRefreshTitle,
    playbackLinks: [],
    sortTitle: "renamed placeholder feature",
    aliases: [],
  });

  // An explicit empty list means the links really are gone (the connection was
  // unlinked or disabled), which must not be confused with an omitted field.
  assert.deepEqual(merged.playbackLinks, []);
  assert.equal(merged.sortTitle, "renamed placeholder feature");
  assert.deepEqual(merged.aliases, []);
});

test("a null from the server still clears a value the panel had loaded", () => {
  const merged = mergePreferLoadedImageFields(hydratedPanelTitle, {
    ...listRefreshTitle,
    overview: null,
    network: null,
  });

  // GraphQL answers an unselected field with undefined and a cleared one with
  // null, so null must survive the merge where undefined does not.
  assert.equal(merged.overview, null);
  assert.equal(merged.network, null);
});

test("a narrower column projection does not blank the columns it left out", () => {
  // Poster view asks for no optional columns at all, so a refresh taken there
  // carries neither rootFolderPath nor the quality columns.
  const posterViewRefresh: TitleRecord = {
    ...listRefreshTitle,
    rootFolderPath: undefined,
    sizeBytes: undefined,
  };

  const merged = mergePreferLoadedImageFields(
    { ...hydratedPanelTitle, qualityTier: "HD-1080p" },
    posterViewRefresh,
  );

  assert.equal(merged.rootFolderPath, hydratedPanelTitle.rootFolderPath);
  assert.equal(merged.sizeBytes, hydratedPanelTitle.sizeBytes);
  assert.equal(merged.qualityTier, "HD-1080p");
});
