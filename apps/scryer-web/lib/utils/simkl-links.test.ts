import assert from "node:assert/strict";
import test from "node:test";
import { simklItemUrl } from "./simkl-links.ts";

test("SIMKL attribution uses stable IDs and the provider library", () => {
  assert.equal(
    simklItemUrl("simkl:movie:123"),
    "https://simkl.com/movies/123/",
  );
  assert.equal(simklItemUrl("simkl:show:456"), "https://simkl.com/tv/456/");
  assert.equal(simklItemUrl("simkl:anime:789"), "https://simkl.com/anime/789/");
  assert.equal(
    simklItemUrl("simkl:anime:9007199254740993"),
    "https://simkl.com/anime/9007199254740993/",
  );
});

test("SIMKL attribution rejects unknown providers, malformed keys, and URL input", () => {
  for (const key of [
    "",
    "trakt:movie:123",
    "simkl:unknown:123",
    "simkl:movie:0",
    "simkl:movie:-1",
    "simkl:movie:12/redirect",
    "simkl:movie:123:slug",
    "https://example.invalid",
  ]) {
    assert.equal(simklItemUrl(key), null);
  }
});
