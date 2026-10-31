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
    for (const identity of [
      { id: `${provider}-list` },
      { providerType: provider },
      { name },
    ]) {
      const logo = getPluginLogoSources(identity);
      assert.equal(logo?.src, expected);
      assert.ok(
        existsSync(new URL(`../../public${expected}`, import.meta.url)),
      );
    }
  }
});

test("list plugin catalog IDs resolve the same bundled logo as their provider", () => {
  for (const provider of [
    "anilist",
    "mal",
    "trakt",
    "simkl",
    "plex",
    "tmdb",
    "imdb",
    "tvdb",
    "mdblist",
    "letterboxd",
  ]) {
    const logo = getPluginLogoSources({ providerType: provider });
    assert.ok(logo, provider);
    assert.equal(
      getPluginLogoSources({ id: `${provider}-list` })?.src,
      logo.src,
      provider,
    );
    assert.ok(
      existsSync(new URL(`../../public${logo.src}`, import.meta.url)),
      logo.src,
    );
  }
});

test("existing plugin logos and unknown-provider fallback are preserved", () => {
  assert.equal(
    getPluginLogoSources({ name: "Simkl" })?.src,
    "/plugin-logos/svg/simkl.svg",
  );
  assert.equal(
    getPluginLogoSources({ name: "Plex" })?.src,
    "/auth-providers/plex.svg",
  );
  assert.equal(
    getPluginLogoSources({ name: "Trakt" })?.src,
    "/plugin-logos/svg/trakt.svg",
  );
  assert.equal(
    getPluginLogoSources({ name: "My Anime List" })?.src,
    "/media-sites/mal.svg",
  );
  assert.equal(
    getPluginLogoSources({ providerType: "filelist" })?.src,
    "/plugin-logos/avif/filelist.avif",
  );
  assert.equal(
    getPluginLogoSources({ id: "custom-list", name: "Custom" }),
    null,
  );
});
