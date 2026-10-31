import assert from "node:assert/strict";
import test from "node:test";
import { Kind, parse } from "graphql";
import { mediaSettingsInitQuery } from "../graphql/queries.ts";
import { updateMediaSettingsMutation } from "../graphql/mutations.ts";

import {
  facetScopedMediaSettingsScopeId,
  normalizeAnimeMediaSettings,
  normalizeFillerPolicy,
  normalizeRecapPolicy,
  normalizeRenameCollisionPolicy,
  normalizeRenameMissingMetadataPolicy,
  updateFacetScopedStringArrayRecord,
  updateFacetScopedStringRecord,
} from "./media-settings-scope.ts";

test("rename policies preserve every supported selection through loading and resaving", () => {
  for (const [normalize, values, fallback] of [
    [
      normalizeRenameCollisionPolicy,
      ["SKIP", "ERROR", "REPLACE_IF_BETTER"],
      "SKIP",
    ],
    [
      normalizeRenameMissingMetadataPolicy,
      ["SKIP", "FALLBACK_TITLE"],
      "FALLBACK_TITLE",
    ],
  ] as const) {
    for (const value of values) {
      assert.equal(normalize(value), value);
      assert.equal(normalize(` ${value.toLowerCase()} `), value);
      for (const scope of ["MOVIE", "SERIES", "ANIME"] as const) {
        const hydrated = updateFacetScopedStringRecord(
          { MOVIE: fallback, SERIES: fallback, ANIME: fallback },
          scope,
          normalize(value),
        );
        assert.equal(normalize(hydrated[scope]), value);
      }
    }
    for (const invalid of [undefined, null, "", "invalid"]) {
      assert.equal(normalize(invalid), fallback);
    }
  }
});

test("media settings loading and saving both request the persisted season-folder flag", () => {
  for (const [document, fieldName] of [
    [mediaSettingsInitQuery, "mediaSettings"],
    [updateMediaSettingsMutation, "updateMediaSettings"],
  ]) {
    const operation = parse(document).definitions.find(
      (definition) => definition.kind === Kind.OPERATION_DEFINITION,
    );
    assert.ok(operation);
    const settings = operation.selectionSet.selections.find(
      (selection) =>
        selection.kind === Kind.FIELD && selection.name.value === fieldName,
    );
    assert.ok(settings?.kind === Kind.FIELD);
    assert.ok(
      settings.selectionSet?.selections.some(
        (selection) =>
          selection.kind === Kind.FIELD &&
          selection.name.value === "useSeasonFolders",
      ),
      `${fieldName} must return an explicitly disabled season-folder setting`,
    );
  }
});

for (const enabled of [true, false]) {
  test(`reloading anime policies and toggles (${enabled}) updates only ANIME`, () => {
    const scope = facetScopedMediaSettingsScopeId({ scope: "ANIME" });
    const settings = normalizeAnimeMediaSettings({
      fillerPolicy: "SKIP_FILLER",
      recapPolicy: "SKIP_RECAP",
      monitorSpecials: enabled,
      interSeasonMovies: enabled,
      monitorFillerMovies: enabled,
    });
    assert.deepEqual(settings, {
      fillerPolicy: "SKIP_FILLER",
      recapPolicy: "SKIP_RECAP",
      monitorSpecials: String(enabled),
      interSeasonMovies: String(enabled),
      monitorFillerMovies: String(enabled),
    });
    for (const value of Object.values(settings)) {
      const previous = {
        MOVIE: "movie value",
        SERIES: "series value",
        ANIME: "old value",
      };
      const next = updateFacetScopedStringRecord(previous, scope, value);
      assert.deepEqual(next, { ...previous, ANIME: value });
      assert.equal(Object.hasOwn(next, "anime"), false);
      assert.equal(previous.ANIME, "old value");
      assert.strictEqual(
        updateFacetScopedStringRecord(next, scope, value),
        next,
      );
    }
  });
}

test("anime policies normalize valid wire values without losing non-default selections", () => {
  for (const [normalize, skip] of [
    [normalizeFillerPolicy, "SKIP_FILLER"],
    [normalizeRecapPolicy, "SKIP_RECAP"],
  ] as const) {
    assert.equal(normalize(skip), skip);
    assert.equal(normalize(` ${skip.toLowerCase()} `), skip);
    assert.equal(normalize("DOWNLOAD_ALL"), "DOWNLOAD_ALL");
    for (const missing of [undefined, null, "", "invalid"]) {
      assert.equal(normalize(missing), "DOWNLOAD_ALL");
    }
  }
});

test("missing anime settings retain established boolean and policy defaults", () => {
  const defaults = {
    fillerPolicy: "DOWNLOAD_ALL",
    recapPolicy: "DOWNLOAD_ALL",
    monitorSpecials: "false",
    interSeasonMovies: "true",
    monitorFillerMovies: "false",
  };
  assert.deepEqual(normalizeAnimeMediaSettings({}), defaults);
  assert.deepEqual(
    normalizeAnimeMediaSettings({
      fillerPolicy: null,
      recapPolicy: null,
      monitorSpecials: null,
      interSeasonMovies: null,
      monitorFillerMovies: null,
    }),
    defaults,
  );
});

test("hydrated anime selections survive normalization for a subsequent settings save", () => {
  const loaded = normalizeAnimeMediaSettings({
    fillerPolicy: "SKIP_FILLER",
    recapPolicy: "SKIP_RECAP",
  });
  const filler = updateFacetScopedStringRecord(
    { MOVIE: "DOWNLOAD_ALL", SERIES: "DOWNLOAD_ALL", ANIME: "DOWNLOAD_ALL" },
    "ANIME",
    loaded.fillerPolicy,
  );
  const recap = updateFacetScopedStringRecord(
    { MOVIE: "DOWNLOAD_ALL", SERIES: "DOWNLOAD_ALL", ANIME: "DOWNLOAD_ALL" },
    "ANIME",
    loaded.recapPolicy,
  );
  assert.deepEqual(
    {
      fillerPolicy: normalizeFillerPolicy(filler.ANIME),
      recapPolicy: normalizeRecapPolicy(recap.ANIME),
    },
    { fillerPolicy: "SKIP_FILLER", recapPolicy: "SKIP_RECAP" },
  );
});

test("saving an anime rename template updates the anime bucket", () => {
  const scopeId = facetScopedMediaSettingsScopeId({ scope: "ANIME" });
  const templates = {
    MOVIE: "{title} ({year}) - {quality}.{ext}",
    SERIES: "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
    ANIME:
      "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}",
  };

  const next = updateFacetScopedStringRecord(
    templates,
    scopeId,
    "{title} - {episode_title} - {source} - {group} - {quality}.{ext}",
  );

  assert.equal(
    next.ANIME,
    "{title} - {episode_title} - {source} - {group} - {quality}.{ext}",
  );
  assert.equal(next.SERIES, templates.SERIES);
  assert.equal(next.MOVIE, templates.MOVIE);
});

test("reloading anime media settings restores values into anime scope rather than the current series bucket", () => {
  const scopeId = facetScopedMediaSettingsScopeId({ scope: "ANIME" });
  const renameTemplates = {
    MOVIE: "{title} ({year}) - {quality}.{ext}",
    SERIES: "SERIES CURRENT TEMPLATE",
    ANIME: "ANIME CURRENT TEMPLATE",
  };
  const audioLanguages = {
    MOVIE: [] as string[],
    SERIES: ["eng"],
    ANIME: ["jpn"],
  };

  const nextTemplates = updateFacetScopedStringRecord(
    renameTemplates,
    scopeId,
    "{title} - {episode_title} [{quality}].{ext}",
  );
  const nextAudioLanguages = updateFacetScopedStringArrayRecord(
    audioLanguages,
    scopeId,
    ["eng", "jpn"],
  );

  assert.equal(nextTemplates.SERIES, "SERIES CURRENT TEMPLATE");
  assert.equal(
    nextTemplates.ANIME,
    "{title} - {episode_title} [{quality}].{ext}",
  );
  assert.deepEqual(nextAudioLanguages.SERIES, ["eng"]);
  assert.deepEqual(nextAudioLanguages.ANIME, ["eng", "jpn"]);
});
