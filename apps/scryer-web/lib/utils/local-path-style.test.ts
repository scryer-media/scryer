import test from "node:test";
import assert from "node:assert/strict";

import {
  isAbsoluteLocalPathForStyle,
  isLocalPathFormatValidForStyle,
  localPathStyleFromRuntimeValue,
} from "./local-path-style.ts";

test("Unix runtime local path validation accepts only Unix absolute paths", () => {
  assert.equal(isAbsoluteLocalPathForStyle("/data/downloads", "unix"), true);
  assert.equal(
    isAbsoluteLocalPathForStyle("/srv/media/files/downloads", "unix"),
    true,
  );
  assert.equal(isAbsoluteLocalPathForStyle("C:\\Downloads", "unix"), false);
  assert.equal(isAbsoluteLocalPathForStyle("C:/Downloads", "unix"), false);
  assert.equal(isAbsoluteLocalPathForStyle("\\\\server\\share", "unix"), false);
  assert.equal(isAbsoluteLocalPathForStyle("downloads", "unix"), false);
  assert.equal(isAbsoluteLocalPathForStyle("../downloads", "unix"), false);
  assert.equal(isAbsoluteLocalPathForStyle("", "unix"), false);
});

test("Windows runtime local path validation accepts Windows absolute paths", () => {
  assert.equal(isAbsoluteLocalPathForStyle("C:\\Downloads", "windows"), true);
  assert.equal(isAbsoluteLocalPathForStyle("C:/Downloads", "windows"), true);
  assert.equal(
    isAbsoluteLocalPathForStyle("\\\\server\\share", "windows"),
    true,
  );
  assert.equal(
    isAbsoluteLocalPathForStyle("/data/downloads", "windows"),
    false,
  );
  assert.equal(isAbsoluteLocalPathForStyle("downloads", "windows"), false);
  assert.equal(isAbsoluteLocalPathForStyle("../downloads", "windows"), false);
  assert.equal(isAbsoluteLocalPathForStyle("", "windows"), false);
});

test("GraphQL runtime path style maps to frontend local path style", () => {
  assert.equal(localPathStyleFromRuntimeValue("UNIX"), "unix");
  assert.equal(localPathStyleFromRuntimeValue("WINDOWS"), "windows");
  assert.equal(localPathStyleFromRuntimeValue(null), undefined);
  assert.equal(localPathStyleFromRuntimeValue(undefined), undefined);
  assert.equal(localPathStyleFromRuntimeValue("MYSTERY"), undefined);
});

test("Unknown runtime path style skips local path format validation", () => {
  assert.equal(
    isLocalPathFormatValidForStyle("C:\\Downloads", undefined),
    true,
  );
  assert.equal(
    isLocalPathFormatValidForStyle("/data/downloads", undefined),
    true,
  );
  assert.equal(
    isLocalPathFormatValidForStyle("relative/path", undefined),
    true,
  );
});

test("Known runtime path style enforces local path format validation", () => {
  assert.equal(isLocalPathFormatValidForStyle("/data/downloads", "unix"), true);
  assert.equal(isLocalPathFormatValidForStyle("C:\\Downloads", "unix"), false);
  assert.equal(
    isLocalPathFormatValidForStyle("C:\\Downloads", "windows"),
    true,
  );
  assert.equal(
    isLocalPathFormatValidForStyle("/data/downloads", "windows"),
    false,
  );
});
