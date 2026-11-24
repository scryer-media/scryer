import assert from "node:assert/strict";
import test from "node:test";
import en from "../i18n/locales/en.ts";
import ru from "../i18n/locales/ru.ts";
import { ERROR_REASON_KEYS } from "../graphql/error-message.ts";
import {
  LOCATION_TITLE_REFUSAL_REASONS,
  locationTitleRefusalMessage,
} from "./location-title-refusal.ts";

const translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => {
  let template = en[key] ?? key;
  for (const [name, value] of Object.entries(values ?? {})) {
    template = template.replaceAll(`{{${name}}}`, String(value));
  }
  return template;
};

test("every translated title refusal has copy in English and Russian", () => {
  for (const reason of LOCATION_TITLE_REFUSAL_REASONS) {
    const key = ERROR_REASON_KEYS[reason];
    assert.ok(key, `${reason} has a key`);
    assert.ok(en[key], `${reason} has English copy`);
    assert.ok(ru[key], `${reason} has Russian copy`);
  }
});

test("a known reason without facts is its copy, whatever the server said", () => {
  assert.deepEqual(
    locationTitleRefusalMessage(
      "LOCATION_DESTINATION_NO_SPACE",
      null,
      translate,
    ),
    [
      {
        kind: "text",
        text: en["error.reason.LOCATION_DESTINATION_NO_SPACE"],
      },
    ],
  );
});

test("an unknown or absent reason leaves the server sentence to the caller", () => {
  assert.equal(
    locationTitleRefusalMessage("SYNTHETIC_UNKNOWN_REASON", {}, translate),
    null,
  );
  assert.equal(locationTitleRefusalMessage(null, null, translate), null);
  assert.equal(
    locationTitleRefusalMessage("LOCATION_DESTINATION_OCCUPIED", {}, translate),
    null,
    "a reason whose sentence carries a path stays the sentence",
  );
});

test("a link left behind names its path as a path segment", () => {
  const path = "/synthetic/library/Synthetic Title/file.mkv";
  const segments = locationTitleRefusalMessage(
    "LOCATION_UNPROVEN_LINK_NOT_REMOVED",
    { leftBehindPath: path, syntheticList: ["a", "b"] },
    translate,
  );
  assert.ok(segments);
  assert.deepEqual(
    segments.filter((segment) => segment.kind === "path"),
    [{ kind: "path", path }],
  );
  const text = segments
    .map((segment) => (segment.kind === "text" ? segment.text : segment.path))
    .join("");
  assert.equal(
    text,
    en["error.unprovenLinkLeftBehindNamed"].replace("{{leftBehindPath}}", path),
  );
});

test("a link left behind without its path gets the base copy", () => {
  assert.deepEqual(
    locationTitleRefusalMessage(
      "LOCATION_UNPROVEN_LINK_NOT_REMOVED",
      { leftBehindPath: 7 },
      translate,
    ),
    [
      {
        kind: "text",
        text: en["error.reason.LOCATION_UNPROVEN_LINK_NOT_REMOVED"],
      },
    ],
  );
});

test("a missing drive names both paths through the shared reason copy", () => {
  const segments = locationTitleRefusalMessage(
    "FOLDER_OWNERSHIP_TITLE_FOLDER_NO_EXISTING_ANCESTOR",
    {
      titleFolder: "/synthetic/volume/Synthetic Title",
      missingDrive: "/synthetic/volume",
    },
    translate,
  );
  assert.ok(segments);
  assert.deepEqual(
    segments.filter((segment) => segment.kind === "path"),
    [
      { kind: "path", path: "/synthetic/volume/Synthetic Title" },
      { kind: "path", path: "/synthetic/volume" },
    ],
  );
});

test("a source kept where a cleanup set it aside names that path", () => {
  const path = "/synthetic/library/Synthetic Title/.synthetic-aside";
  const segments = locationTitleRefusalMessage(
    "LOCATION_SET_ASIDE_SOURCE_NOT_PUT_BACK",
    { leftBehindPath: path },
    translate,
  );
  assert.ok(segments);
  assert.deepEqual(
    segments.filter((segment) => segment.kind === "path"),
    [{ kind: "path", path }],
  );
  assert.deepEqual(
    locationTitleRefusalMessage(
      "LOCATION_SET_ASIDE_SOURCE_NOT_PUT_BACK",
      {},
      translate,
    ),
    [
      {
        kind: "text",
        text: en["error.reason.LOCATION_SET_ASIDE_SOURCE_NOT_PUT_BACK"],
      },
    ],
  );
});
