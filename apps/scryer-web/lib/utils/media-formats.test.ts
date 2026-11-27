import assert from "node:assert/strict";
import test from "node:test";
import {
  STANDALONE_VIDEO_EXTENSIONS,
  VIDEO_EXTENSIONS,
} from "./media-formats.generated.ts";
import { canonicalVideoExtension, videoFileStem } from "./media-formats.ts";

test("every registered suffix normalizes case and trailing junk without accepting deceptive suffixes", () => {
  for (const extension of VIDEO_EXTENSIONS) {
    for (const suffix of [extension, extension.toUpperCase()]) {
      for (const junk of ["", "_", '"', ". ", "'`_\t"]) {
        assert.equal(
          canonicalVideoExtension(`/media/Release.${suffix}${junk}`),
          extension,
        );
        const standalone = (
          STANDALONE_VIDEO_EXTENSIONS as readonly string[]
        ).includes(extension);
        assert.equal(
          videoFileStem(`D:\\media\\Release.${suffix}${junk}`),
          standalone ? "Release" : `Release.${suffix}${junk}`,
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

test("only standalone library suffixes are stripped from a file stem", () => {
  for (const name of [
    "Movie.Release.ifo",
    "Movie.Release.ogg",
    "Movie.Release.mts",
  ]) {
    assert.ok(canonicalVideoExtension(name));
    assert.equal(videoFileStem(name), name);
  }
  assert.equal(videoFileStem("Movie.Release.mkv"), "Movie.Release");
  assert.ok(
    STANDALONE_VIDEO_EXTENSIONS.length < VIDEO_EXTENSIONS.length,
    "probe-only suffixes stay outside the standalone list",
  );
});
