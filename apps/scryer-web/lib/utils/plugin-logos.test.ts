import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import test from "node:test";

import { getPluginLogoSources } from "./plugin-logos.ts";

test("uninstalled list providers reuse shipped metadata logos", () => {
  for (const [provider, name, expected] of [
    ["anilist", "AniList", "/media-sites/anilist.svg"],
    ["mal", "MyAnimeList", "/media-sites/mal.svg"],
    ["mdblist", "MDBList", "/rating-sources/mdblist.avif"],
    ["tmdb", "TMDb", "/rating-sources/tmdb.svg"],
  ]) {
    for (const identity of [{ id: `${provider}-list` }, { providerType: provider }, { name }]) {
      const logo = getPluginLogoSources(identity);
      assert.equal(logo?.src, expected);
      assert.ok(existsSync(new URL(`../../public${expected}`, import.meta.url)));
    }
  }
});

test("existing plugin logos and unknown-provider fallback are preserved", () => {
  assert.equal(getPluginLogoSources({ name: "Simkl" })?.src, "/plugin-logos/svg/simkl.svg");
  assert.equal(getPluginLogoSources({ name: "Plex" })?.src, "/auth-providers/plex.svg");
  assert.equal(getPluginLogoSources({ name: "Trakt" })?.src, "/plugin-logos/svg/trakt.svg");
  assert.equal(getPluginLogoSources({ id: "custom-list", name: "Custom" }), null);
});
