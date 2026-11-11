import assert from "node:assert/strict";
import test from "node:test";
import {
  type ArrDiagnostic,
  type ReviewedCustomFormat,
  arrImportErrorMessage,
  candidateScores,
  collectFormatScores,
  defaultScore,
  isCurrentTranslationRequest,
  recommendScore,
  reviewedFormat,
  sourceFacets,
} from "./arr-custom-format-import-state.ts";

function format(
  overrides: Partial<ReviewedCustomFormat> = {},
): ReviewedCustomFormat {
  return {
    id: "format-id",
    name: "Format",
    specifications: [{ negate: false }],
    suggestedScores: {},
    ...overrides,
  };
}

test("exported scores take priority, with default preferred among profile choices", () => {
  const source = format({ suggestedScores: { default: 10 } });
  assert.deepEqual(candidateScores(source), [10]);
  assert.equal(defaultScore(source, []), "10");
  assert.equal(
    defaultScore(format({ suggestedScores: { default: 10, anime: 15 } }), []),
    "10",
  );
  const conflict: ArrDiagnostic = {
    code: "arr.score.conflict",
    severity: "ERROR",
    message: "Choose explicitly.",
    formatId: source.id,
  };
  assert.equal(defaultScore(source, [conflict]), "");
  assert.equal(
    defaultScore(
      format({ name: "Extras", suggestedScores: { default: 300 } }),
      [],
    ),
    "300",
  );
  assert.equal(
    recommendScore(format({ suggestedScores: { default: 10, anime: 15 } }), [])
      ?.reasonKey,
    "settings.arrImportScoreReasonDefault",
  );
});

test("clear preference and penalty names get conservative editable starting scores", () => {
  for (const [name, expected] of [
    ["Prefer HEVC", 100],
    ["Preferred WEB-DL", 100],
    ["Boost lossless audio", 100],
    ["LQ", -100],
    ["Low-quality groups", -100],
    ["Bad Dual Groups", -100],
    ["Avoid CAM", -100],
    ["Block unwanted releases", -100],
    ["Extras", -1000],
    ["Samples", -1000],
    ["Block extras", -1000],
    ["Prefer extras", 100],
  ] as const) {
    const recommendation = recommendScore(format({ name }), []);
    assert.equal(recommendation?.value, expected, name);
    assert.equal(defaultScore(format({ name }), []), String(expected), name);
    assert.ok(recommendation.value > -9000, name);
  }
  const imported = format({ name: "Prefer HEVC" });
  assert.deepEqual(collectFormatScores([imported], { [imported.id]: "250" }), {
    scores: [{ formatId: imported.id, score: 250 }],
  });
});

test("clearly named surround audio formats get an editable preference recommendation", () => {
  for (const name of ["5.1 Surround", "7.1 Surround"] as const) {
    const recommendation = recommendScore(format({ name }), []);
    assert.deepEqual(
      recommendation,
      { value: 100, reasonKey: "settings.arrImportScoreReasonPreference" },
      name,
    );
    assert.equal(defaultScore(format({ name }), []), "100", name);
  }
  assert.deepEqual(
    recommendScore(
      format({ name: "5.1 Surround", suggestedScores: { default: 250 } }),
      [],
    ),
    { value: 250, reasonKey: "settings.arrImportScoreReasonExported" },
  );
  const [positive] = format().specifications;
  const mixedSurroundConditions = [
    { ...positive!, negate: false },
    { ...positive!, negate: true },
    { ...positive!, negate: true },
  ];
  assert.equal(
    recommendScore(
      format({ name: "5.1 Surround", specifications: mixedSurroundConditions }),
      [],
    )?.value,
    100,
  );
  const negated = format().specifications.map((spec) => ({
    ...spec,
    negate: true,
  }));
  assert.equal(
    recommendScore(
      format({ name: "5.1 Surround", specifications: negated }),
      [],
    ),
    null,
  );
  assert.equal(recommendScore(format({ name: "5.1 Audio" }), []), null);
});

test("neutral names, ambiguous exports, and malformed conditions do not invent scores", () => {
  for (const name of [
    "HEVC",
    "HDR",
    "1080p",
    "Release group",
    "No extras",
    "Not low quality",
    "LQless",
  ]) {
    assert.equal(recommendScore(format({ name }), []), null, name);
  }
  assert.equal(
    recommendScore(
      format({
        name: "Prefer HEVC",
        suggestedScores: { anime: 10, movies: 20 },
      }),
      [],
    ),
    null,
  );
  assert.equal(
    recommendScore(
      format({ name: "Prefer HEVC", suggestedScores: { default: Infinity } }),
      [],
    ),
    null,
  );
  assert.equal(
    recommendScore(format({ name: "Prefer HEVC", specifications: [] }), []),
    null,
  );
  const malformed = format({
    name: "Prefer HEVC",
    inspectionDiagnostics: [
      {
        code: "arr.specification.invalid",
        severity: "ERROR",
        message: "Invalid",
        formatId: "format-id",
      },
    ],
  });
  assert.equal(recommendScore(malformed, []), null);
});

test("negate alone never establishes a preference or an extras penalty", () => {
  const negated = format().specifications.map((spec) => ({
    ...spec,
    negate: true,
  }));
  assert.equal(
    recommendScore(format({ name: "HEVC", specifications: negated }), []),
    null,
  );
  assert.equal(
    recommendScore(format({ name: "Extras", specifications: negated }), []),
    null,
  );
});

test("hard-block scores require an explicit valid exported weight or user entry", () => {
  assert.equal(defaultScore(format({ name: "Block everything" }), []), "-100");
  assert.equal(
    defaultScore(format({ suggestedScores: { default: -10000 } }), []),
    "-10000",
  );
  assert.deepEqual(
    candidateScores(
      format({
        suggestedScores: { bad: 2147483648, fraction: 1.5, valid: 10 },
      }),
    ),
    [10],
  );
});

test("requires one signed 32-bit integer score per custom format", () => {
  const first = format({ id: "first" });
  const second = format({ id: "second", name: "Second" });
  assert.deepEqual(
    collectFormatScores([first, second], { first: "-9000", second: "25" }),
    {
      scores: [
        { formatId: "first", score: -9000 },
        { formatId: "second", score: 25 },
      ],
    },
  );
  for (const invalid of [
    "1.5",
    "Infinity",
    "9007199254740992",
    "2147483648",
    "-2147483649",
  ]) {
    assert.deepEqual(collectFormatScores([first], { first: invalid }), {
      missingFormat: first,
    });
  }
});

test("uses Arr-specific default facets and rejects stale responses", () => {
  assert.deepEqual(sourceFacets("radarr"), ["movie"]);
  assert.deepEqual(sourceFacets("sonarr"), ["series", "anime"]);
  assert.equal(isCurrentTranslationRequest(4, 4), true);
  assert.equal(isCurrentTranslationRequest(5, 4), false);
});

test("reviewed formats key suggested scores by export label in export order", () => {
  const reviewed = reviewedFormat({
    id: "imported_format_1",
    name: "Synthetic",
    status: "DISABLED",
    diagnostics: [],
    suggestedScores: [
      { label: "default", score: 10 },
      { label: "anime", score: 15 },
    ],
    specifications: [{ index: 0, negate: true }],
    inspectionDiagnostics: [],
  });
  assert.deepEqual(reviewed.suggestedScores, { default: 10, anime: 15 });
  assert.deepEqual(Object.keys(reviewed.suggestedScores), ["default", "anime"]);
  assert.deepEqual(reviewed.specifications, [{ index: 0, negate: true }]);
});

function refusal(extensions: Record<string, unknown>) {
  return {
    graphQLErrors: [
      {
        message: "Server sentence.",
        extensions: { code: "VALIDATION_ERROR", ...extensions },
      },
    ],
  };
}

test("import refusals translate their reason and limit", () => {
  const t = (key: string, values?: Record<string, unknown>) =>
    values ? `${key}:${JSON.stringify(values)}` : key;
  assert.equal(
    arrImportErrorMessage(
      refusal({
        reason: "ARR_CUSTOM_FORMAT_JSON_TOO_LARGE",
        maxBytes: "16777216",
      }),
      t,
      "fallback",
    ),
    'settings.arrImportRefusalTooLarge:{"maxBytes":"16777216"}',
  );
  assert.equal(
    arrImportErrorMessage(
      refusal({
        reason: "ARR_CUSTOM_FORMAT_FORMATS_TOO_MANY",
        maxFormats: "1000",
      }),
      t,
      "fallback",
    ),
    'settings.arrImportRefusalTooMany:{"maxFormats":"1000"}',
  );
  assert.equal(
    arrImportErrorMessage(
      refusal({ reason: "ARR_CUSTOM_FORMAT_JSON_INVALID" }),
      t,
      "fallback",
    ),
    "settings.arrImportInvalidJson",
  );
  assert.equal(
    arrImportErrorMessage(
      refusal({ reason: "ARR_CUSTOM_FORMAT_FORMATS_EMPTY" }),
      t,
      "fallback",
    ),
    "settings.arrImportRefusalEmpty",
  );
  assert.equal(
    arrImportErrorMessage(
      refusal({ reason: "ARR_CUSTOM_FORMAT_SOMETHING_NEW" }),
      t,
      "fallback",
    ),
    "Server sentence.",
  );
});
