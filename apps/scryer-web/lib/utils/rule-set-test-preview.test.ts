import assert from "node:assert/strict";
import test from "node:test";
import {
  canTestRuleSet,
  buildRuleSetTestInput,
  RuleSetTestRequestController,
  EMPTY_RULE_SET_TEST_LISTING,
  listingInputFromDraft,
  ruleSetTestFingerprint,
  sizeBytesFromGib,
  shouldApplyRuleSetTestResponse,
  storedFileOptions,
} from "./rule-set-test-preview.ts";
import { testRuleSetMutation } from "../graphql/mutations.ts";

const draft = {
  name: "rule",
  description: "",
  regoSource: "package test",
  enabled: true,
  priority: 0,
  appliedFacets: [],
};

test("preview requires a title, release name, and an episode for episodic titles", () => {
  const movie = {
    titleId: "title",
    episodeId: null,
    releaseName: "x",
    sizeGib: "",
  };
  assert.equal(canTestRuleSet(movie, false), true);
  assert.equal(canTestRuleSet(movie, true), false);
  assert.equal(
    canTestRuleSet({ ...movie, episodeId: "episode", releaseName: "" }, true),
    false,
  );
});

test("preview becomes stale for either draft or input changes", () => {
  const selection = {
    titleId: "title",
    episodeId: null,
    releaseName: "release",
    sizeGib: "",
  };
  const baseline = ruleSetTestFingerprint(draft, selection, null, null, null);
  assert.notEqual(
    baseline,
    ruleSetTestFingerprint(
      { ...draft, enabled: false },
      selection,
      null,
      null,
      null,
    ),
  );
  assert.notEqual(
    baseline,
    ruleSetTestFingerprint(
      draft,
      { ...selection, sizeGib: "2" },
      null,
      null,
      null,
    ),
  );
});

test("saved previews are distinct from draft previews and retain the selected rule identity", () => {
  const selection = {
    titleId: "title",
    episodeId: null,
    releaseName: "release",
    sizeGib: "",
  };
  const saved = ruleSetTestFingerprint(
    null,
    selection,
    null,
    null,
    "installed-rule",
  );
  assert.notEqual(
    saved,
    ruleSetTestFingerprint(draft, selection, null, null, null),
  );
  assert.notEqual(
    saved,
    ruleSetTestFingerprint(null, selection, null, null, "other-rule"),
  );
});

test("saved preview requests contain only the installed rule identity and selection", () => {
  const input = buildRuleSetTestInput({
    draft,
    editRuleSetId: "edit-rule",
    copySourceRuleSetId: "copy-source",
    testRuleSetId: "installed-rule",
    titleId: "title",
    episodeId: "episode",
    releaseName: "release",
    sizeBytes: 42,
  });
  assert.deepEqual(input, {
    testRuleSetId: "installed-rule",
    titleId: "title",
    episodeId: "episode",
    releaseName: "release",
    sizeBytes: 42,
  });
});

test("delayed preview response is discarded after committed inputs change", async () => {
  let release!: () => void;
  const delayed = new Promise<void>((resolve) => {
    release = resolve;
  });
  const requestFingerprint = "before";
  let committedFingerprint = requestFingerprint;
  const response = delayed.then(() =>
    shouldApplyRuleSetTestResponse(
      1,
      1,
      requestFingerprint,
      committedFingerprint,
    ),
  );
  committedFingerprint = "after";
  release();
  assert.equal(await response, false);
});

test("request controller blocks duplicate submission and releases its owner", async () => {
  let release!: () => void;
  const delayed = new Promise<void>((resolve) => {
    release = resolve;
  });
  const controller = new RuleSetTestRequestController();
  const request = controller.begin();
  assert.equal(request, 1);
  assert.equal(controller.begin(), null);
  const completion = delayed.then(() => controller.finish(request!));
  release();
  assert.equal(await completion, true);
  assert.equal(controller.begin(), 2);
});

test("disposed preview controller ignores delayed completion", async () => {
  let release!: () => void;
  const delayed = new Promise<void>((resolve) => {
    release = resolve;
  });
  const controller = new RuleSetTestRequestController();
  const request = controller.begin();
  controller.dispose();
  const completion = delayed.then(() => controller.finish(request!));
  release();
  assert.equal(await completion, false);
});

test("controller reactivation permits a new request without reviving the old one", () => {
  const controller = new RuleSetTestRequestController();
  controller.activate();
  const oldRequest = controller.begin();
  controller.dispose();
  controller.activate();
  const newRequest = controller.begin();
  assert.notEqual(oldRequest, newRequest);
  assert.equal(controller.isCurrent(oldRequest!), false);
  assert.equal(controller.isCurrent(newRequest!), true);
});

test("protocol selections reach the tester and invalidate previous results", () => {
  const selection = {
    titleId: "title",
    episodeId: null,
    releaseName: "release",
    sizeGib: "",
    listing: EMPTY_RULE_SET_TEST_LISTING,
  };
  const before = ruleSetTestFingerprint(draft, selection, null, null, null);
  for (const protocol of ["usenet", "torrent", "http"]) {
    const listing = { ...EMPTY_RULE_SET_TEST_LISTING, protocol };
    assert.deepEqual(listingInputFromDraft(listing), { value: { protocol } });
    assert.notEqual(
      ruleSetTestFingerprint(
        draft,
        { ...selection, listing },
        null,
        null,
        null,
      ),
      before,
    );
  }
  for (const protocol of ["", "unknown", "ftp"]) {
    assert.deepEqual(
      listingInputFromDraft({ ...EMPTY_RULE_SET_TEST_LISTING, protocol }),
      { value: undefined },
    );
  }
});

test("size conversion preserves unknown values and rejects unsafe bytes", () => {
  assert.deepEqual(sizeBytesFromGib(""), { value: undefined });
  assert.deepEqual(sizeBytesFromGib("1"), { value: 1024 ** 3 });
  assert.equal("error" in sizeBytesFromGib("1e20"), true);
});

test("preview operation asks for structured evaluation errors", () => {
  assert.match(
    testRuleSetMutation,
    /errors \{\s+code\s+message\s+ruleSetId\s+\}/,
  );
  assert.match(
    testRuleSetMutation,
    /entries \{\s+code\s+delta\s+blocked\s+kind\s+\}/,
  );
  assert.match(
    testRuleSetMutation,
    /releaseGroup[\s\S]*videoCodec[\s\S]*audioLanguages/,
  );
});

test("a stored file needs only a title and a file, never an episode", () => {
  const stored = {
    titleId: "title",
    episodeId: null,
    releaseName: "",
    sizeGib: "",
    mode: "storedFile" as const,
    mediaFileId: null,
  };
  assert.equal(canTestRuleSet(stored, true), false);
  assert.equal(canTestRuleSet({ ...stored, mediaFileId: "file" }, true), true);
  assert.equal(
    canTestRuleSet({ ...stored, titleId: null, mediaFileId: "file" }, false),
    false,
  );
});

test("stored-file requests send the file, never a release name, size, or listing", () => {
  const input = buildRuleSetTestInput({
    draft,
    editRuleSetId: null,
    copySourceRuleSetId: null,
    testRuleSetId: "installed-rule",
    titleId: "title",
    releaseName: "ignored",
    sizeBytes: 42,
    listing: { thumbsUp: 3 },
    mediaFileId: "file",
  });
  assert.deepEqual(input, {
    testRuleSetId: "installed-rule",
    titleId: "title",
    episodeId: undefined,
    mediaFileId: "file",
  });
});

test("release requests carry the typed listing facts", () => {
  const input = buildRuleSetTestInput({
    draft: null,
    editRuleSetId: null,
    copySourceRuleSetId: null,
    testRuleSetId: "installed-rule",
    titleId: "title",
    releaseName: "release",
    listing: { thumbsUp: 3, extra: { freeleech: true } },
  });
  assert.deepEqual(input.listing, { thumbsUp: 3, extra: { freeleech: true } });
  assert.equal(input.mediaFileId, undefined);
});

test("empty listing facts stay unknown", () => {
  assert.deepEqual(listingInputFromDraft(EMPTY_RULE_SET_TEST_LISTING), {
    value: undefined,
  });
  assert.deepEqual(
    listingInputFromDraft({
      ...EMPTY_RULE_SET_TEST_LISTING,
      indexerLanguages: " , ",
      isPasswordProtected: "",
    }),
    { value: undefined },
  );
});

test("listing facts parse into mutation input", () => {
  assert.deepEqual(
    listingInputFromDraft({
      publishedAt: "2024-01-31",
      protocol: "torrent",
      thumbsUp: "12",
      thumbsDown: " 0 ",
      isPasswordProtected: "false",
      indexerLanguages: "en, de ,",
      extra: '{"freeleech": true, "grabs": 5}',
    }),
    {
      value: {
        publishedAt: "2024-01-31T00:00:00Z",
        protocol: "torrent",
        thumbsUp: 12,
        thumbsDown: 0,
        isPasswordProtected: false,
        indexerLanguages: ["en", "de"],
        extra: { freeleech: true, grabs: 5 },
      },
    },
  );
  assert.deepEqual(
    listingInputFromDraft({
      ...EMPTY_RULE_SET_TEST_LISTING,
      publishedAt: "2024-01-31 12:30:00+02:00",
    }),
    { value: { publishedAt: "2024-01-31 12:30:00+02:00" } },
    "an RFC 3339 time is kept as written",
  );
  assert.deepEqual(
    listingInputFromDraft({
      ...EMPTY_RULE_SET_TEST_LISTING,
      publishedAt: " Wed, 01 Jan 2025 12:00:00 +0000 ",
    }),
    { value: { publishedAt: "Wed, 01 Jan 2025 12:00:00 +0000" } },
    "a newznab pubDate is accepted",
  );
});

test("malformed listing facts report their i18n error", () => {
  const bad = (
    field: keyof typeof EMPTY_RULE_SET_TEST_LISTING,
    value: string,
  ) =>
    listingInputFromDraft({ ...EMPTY_RULE_SET_TEST_LISTING, [field]: value });
  for (const value of [
    "yesterday",
    "2024-01-31T12:00:00",
    "2024-01-31T12:30Z",
    "2024-13-45T00:00:00Z",
    "Wed, 45 Foo 2025 12:00:00 +0000",
  ]) {
    assert.deepEqual(bad("publishedAt", value), {
      error: "settings.ruleTestListingPublishedAtInvalid",
    });
  }
  assert.deepEqual(bad("thumbsUp", "1.5"), {
    error: "settings.ruleTestListingVotesInvalid",
  });
  assert.deepEqual(bad("thumbsUp", "-1"), {
    error: "settings.ruleTestListingVotesInvalid",
  });
  assert.deepEqual(bad("thumbsDown", "many"), {
    error: "settings.ruleTestListingVotesInvalid",
  });
  for (const value of ["[1, 2]", "null", "7", "{not json"]) {
    assert.deepEqual(bad("extra", value), {
      error: "settings.ruleTestListingExtraInvalid",
    });
  }
});

test("listing changes make a previous result stale", () => {
  const selection = {
    titleId: "title",
    episodeId: null,
    releaseName: "release",
    sizeGib: "",
    listing: EMPTY_RULE_SET_TEST_LISTING,
  };
  assert.notEqual(
    ruleSetTestFingerprint(draft, selection, null, null, null),
    ruleSetTestFingerprint(
      draft,
      {
        ...selection,
        listing: { ...EMPTY_RULE_SET_TEST_LISTING, thumbsUp: "1" },
      },
      null,
      null,
      null,
    ),
  );
});

test("preview operation echoes the scored release and its listing facts", () => {
  assert.match(testRuleSetMutation, /releaseName\s+mediaFileId/);
  assert.match(
    testRuleSetMutation,
    /listing \{\s+protocol\s+publishedAt\s+ageDays\s+thumbsUp\s+thumbsDown\s+isPasswordProtected\s+indexerLanguages\s+extra\s+capturedAt\s+\}/,
  );
});

test("stored-file options list each file once and follow the selected episode", () => {
  const rows = [
    {
      id: "disc",
      episodeId: "ep-1",
      filePath: "/library/Serial/disc.iso",
      grabbedReleaseTitle: null,
    },
    {
      id: "disc",
      episodeId: "ep-2",
      filePath: "/library/Serial/disc.iso",
      grabbedReleaseTitle: null,
    },
    {
      id: "single",
      episodeId: "ep-2",
      filePath: "C:\\library\\Serial\\e2.mkv",
      grabbedReleaseTitle: "Synthetic.Serial.S01E02",
    },
    { id: "unbound", episodeId: null, filePath: "/library/Serial/extra.mkv" },
  ];
  assert.deepEqual(storedFileOptions(rows, null), [
    { id: "disc", label: "disc.iso", episodeIds: ["ep-1", "ep-2"] },
    { id: "single", label: "Synthetic.Serial.S01E02", episodeIds: ["ep-2"] },
    { id: "unbound", label: "extra.mkv", episodeIds: [] },
  ]);
  assert.deepEqual(
    storedFileOptions(rows, "ep-1").map((option) => option.id),
    ["disc"],
  );
  assert.deepEqual(
    storedFileOptions(rows, "ep-2").map((option) => option.id),
    ["disc", "single"],
  );
});
