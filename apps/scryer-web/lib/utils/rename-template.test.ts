import test from "node:test";
import assert from "node:assert/strict";

import {
  applyRenameTemplatePreview,
  splitRenameTemplateSegments,
  validateFolderTemplateSyntax,
  validateRenameTemplateSyntax,
} from "./rename-template.ts";

const VALID_TOKENS = new Set(["title", "season_order", "edition", "ext"]);
const VALID_FOLDER_TOKENS = new Set(["title", "season"]);
const SAMPLE_VALUES = {
  title: "The Grey Harbor",
  edition: "IMAX",
  ext: "mkv",
};

test("validateRenameTemplateSyntax accepts truncate filters", () => {
  assert.equal(
    validateRenameTemplateSyntax("{title|truncate:8|space:_}.{ext}", VALID_TOKENS),
    null,
  );
});

test("validateRenameTemplateSyntax accepts optional groups and rejects unsupported branches", () => {
  const episodeTokens = new Set([
    "title", "season_order", "episode", "absolute_episode", "episode_title", "quality", "ext",
  ]);
  assert.equal(
    validateRenameTemplateSyntax(
      "{title} - S{season_order:2}E{episode:2}{?absolute_episode: ({absolute_episode})}{?episode_title: - {episode_title|truncate:64}} - {quality}.{ext}",
      episodeTokens,
    ),
    null,
  );
  assert.equal(
    validateRenameTemplateSyntax("{?edition:{{literal|else:edition}}}", VALID_TOKENS),
    null,
  );
  assert.deepEqual(
    validateRenameTemplateSyntax("{?title: {?edition: ({edition})}}", VALID_TOKENS),
    { kind: "nestedOptionalGroup" },
  );
  assert.deepEqual(
    validateRenameTemplateSyntax("{?title: {title}|else: fallback}", VALID_TOKENS),
    { kind: "unsupportedOptionalFallback" },
  );
  assert.deepEqual(
    validateRenameTemplateSyntax("{?title|truncate:8: ({title})}", VALID_TOKENS),
    { kind: "invalidOptionalGroup" },
  );
});

test("validateRenameTemplateSyntax rejects invalid truncate filters", () => {
  assert.deepEqual(validateRenameTemplateSyntax("{title|truncate:0}", VALID_TOKENS), {
    kind: "invalidFilter",
    filter: "truncate:0",
  });
  assert.deepEqual(validateRenameTemplateSyntax("{title|truncate:abc}", VALID_TOKENS), {
    kind: "invalidFilter",
    filter: "truncate:abc",
  });
});

test("validateFolderTemplateSyntax accepts season padding and escaped braces", () => {
  for (const template of [
    "Season {season}",
    "Season {season:0}",
    "Season {season:2}",
    "{{S{season}}}",
  ]) {
    assert.equal(validateFolderTemplateSyntax(template, VALID_FOLDER_TOKENS, "season"), null);
  }
  assert.equal(
    applyRenameTemplatePreview("Season {season:2}", VALID_FOLDER_TOKENS, { season: "3" }),
    "Season 03",
  );
  assert.equal(
    applyRenameTemplatePreview("{{S{season}}}", VALID_FOLDER_TOKENS, { season: "3" }),
    "{S3}",
  );
});

test("validateFolderTemplateSyntax and preview support optional groups", () => {
  const folderTokens = new Set(["title", "year", "season"]);
  assert.equal(
    validateFolderTemplateSyntax("{title}{?year: ({year})}", folderTokens),
    null,
  );
  assert.equal(
    validateFolderTemplateSyntax("{?season:Season {season}}", folderTokens, "season"),
    null,
  );
  assert.equal(
    applyRenameTemplatePreview("{title}{?year: ({year})}", folderTokens, { title: "Movie" }),
    "Movie",
  );
  assert.equal(
    applyRenameTemplatePreview("{title}{?year: ({year})}", folderTokens, { title: "Movie", year: "2004" }),
    "Movie (2004)",
  );
});

test("validateFolderTemplateSyntax rejects malformed or excessive padding", () => {
  for (const [template, padding] of [
    ["Season {season:}", ""],
    ["Season {season:abc}", "abc"],
    ["Season {season:2x}", "2x"],
    ["Season {season:241}", "241"],
    ["Season {season:999999999999999999999999999999999999999}", "999999999999999999999999999999999999999"],
  ]) {
    assert.deepEqual(validateFolderTemplateSyntax(template, VALID_FOLDER_TOKENS, "season"), {
      kind: "invalidPadding",
      padding,
    });
  }
});

test("validateFolderTemplateSyntax rejects illegal literal characters", () => {
  for (const character of ["<", ">", ":", "\"", "/", "\\", "|", "?", "*", "\n"]) {
    assert.deepEqual(
      validateFolderTemplateSyntax(`Season${character} {season}`, VALID_FOLDER_TOKENS, "season"),
      { kind: "illegalCharacter", character },
    );
  }
});

test("validateRenameTemplateSyntax accepts literal brace escapes", () => {
  assert.equal(
    validateRenameTemplateSyntax("{{edition-{edition}}}", VALID_TOKENS),
    null,
  );
});

test("validateRenameTemplateSyntax rejects unmatched single braces", () => {
  assert.deepEqual(validateRenameTemplateSyntax("prefix {", VALID_TOKENS), {
    kind: "unmatchedOpen",
  });
  assert.deepEqual(validateRenameTemplateSyntax("prefix }", VALID_TOKENS), {
    kind: "unmatchedClose",
  });
});

test("applyRenameTemplatePreview applies truncate before later filters", () => {
  assert.equal(
    applyRenameTemplatePreview(
      "{title|truncate:8|space:_}.{ext}",
      VALID_TOKENS,
      SAMPLE_VALUES,
    ),
    "The_Grey.mkv",
  );
});

test("applyRenameTemplatePreview renders literal brace escapes", () => {
  assert.equal(
    applyRenameTemplatePreview(
      "{{edition-{edition}}}",
      VALID_TOKENS,
      SAMPLE_VALUES,
    ),
    "{edition-IMAX}",
  );
});

test("applyRenameTemplatePreview renders literal brace escapes inside optional groups", () => {
  assert.equal(
    applyRenameTemplatePreview(
      "{?edition:{{cut-{edition}}}}",
      VALID_TOKENS,
      SAMPLE_VALUES,
    ),
    "{cut-IMAX}",
  );
});

test("applyRenameTemplatePreview preserves escaped folder tokens as literals", () => {
  assert.equal(
    applyRenameTemplatePreview(
      "{title} ({{year}})",
      new Set(["title", "year"]),
      { title: "The Grey Harbor", year: "2008" },
    ),
    "The Grey Harbor ({year})",
  );
});

test("applyRenameTemplatePreview renders missing sample values as empty strings", () => {
  assert.equal(
    applyRenameTemplatePreview("{title} - {season_order}.{ext}", VALID_TOKENS, SAMPLE_VALUES),
    "The Grey Harbor - .mkv",
  );
});

test("applyRenameTemplatePreview omits missing optional values and truncates present values", () => {
  const episodeTokens = new Set([
    "title", "season_order", "episode", "absolute_episode", "episode_title", "quality", "ext",
  ]);
  const template = "{title} - S{season_order:2}E{episode:2}{?absolute_episode: ({absolute_episode})}{?episode_title: - {episode_title|truncate:8}} - {quality}.{ext}";
  assert.equal(
    applyRenameTemplatePreview(template, episodeTokens, {
      title: "The Grey Harbor", season_order: "0", episode: "4", absolute_episode: "",
      episode_title: "Harbor Signal Uprising", quality: "2160p", ext: "mkv",
    }),
    "The Grey Harbor - S00E04 - Harbor S - 2160p.mkv",
  );
});

test("splitRenameTemplateSegments highlights filtered token specs", () => {
  assert.deepEqual(
    splitRenameTemplateSegments("{title|truncate:8|space:_}.{ext}", VALID_TOKENS),
    [
      { text: "{title|truncate:8|space:_}", isToken: true },
      { text: ".", isToken: false },
      { text: "{ext}", isToken: true },
    ],
  );
});

test("splitRenameTemplateSegments highlights valid optional groups", () => {
  assert.deepEqual(
    splitRenameTemplateSegments("{title}{?edition: ({edition})}.{ext}", VALID_TOKENS),
    [
      { text: "{title}", isToken: true },
      { text: "{?edition: ({edition})}", isToken: true },
      { text: ".", isToken: false },
      { text: "{ext}", isToken: true },
    ],
  );
});

test("splitRenameTemplateSegments leaves escaped literal braces unhighlighted", () => {
  assert.deepEqual(
    splitRenameTemplateSegments("{title} ({{year}})", new Set(["title", "year"])),
    [
      { text: "{title}", isToken: true },
      { text: " ({{year}})", isToken: false },
    ],
  );
});
