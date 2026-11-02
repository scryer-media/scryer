import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { VIDEO_EXTENSIONS } from "./media-formats.generated.ts";
import { canonicalVideoExtension, videoFileStem } from "./media-formats.ts";

test("generated media registry matches the authoritative Rust declaration", () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(
        new URL("../../scripts/generate-media-formats.mjs", import.meta.url),
      ),
      "--check",
    ],
    { encoding: "utf8", timeout: 30_000 },
  );
  assert.equal(result.status, 0, result.stderr);
});

test("every registered suffix normalizes case and trailing junk without accepting deceptive suffixes", () => {
  for (const extension of VIDEO_EXTENSIONS) {
    for (const suffix of [extension, extension.toUpperCase()]) {
      for (const junk of ["", "_", '"', ". ", "'`_\t"]) {
        assert.equal(
          canonicalVideoExtension(`/media/Release.${suffix}${junk}`),
          extension,
        );
        assert.equal(
          videoFileStem(`D:\\media\\Release.${suffix}${junk}`),
          "Release",
        );
      }
      for (const name of [
        `Release.${suffix}.exe`,
        `Release.${suffix}x`,
        `.${suffix}`,
      ]) {
        assert.equal(canonicalVideoExtension(name), undefined);
        assert.equal(videoFileStem(name), name);
      }
    }
  }
});

test("normalization uses Unicode whitespace and ASCII case like the backend", () => {
  assert.equal(canonicalVideoExtension("\u0085Release.MKV\u0085"), "mkv");
  assert.equal(canonicalVideoExtension("Release.m\u212Av"), undefined);
  assert.equal(canonicalVideoExtension("Release.mkv\ufeff"), undefined);
});
