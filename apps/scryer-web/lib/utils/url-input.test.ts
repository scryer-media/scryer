import assert from "node:assert/strict";
import test from "node:test";

import type { ConfigFieldDef } from "../types/index.ts";
import {
  defaultSchemeFor,
  normalizeIndexerConfigValues,
  normalizeUrlInput,
  parseHostInput,
} from "./url-input.ts";

function configField(
  overrides: Partial<ConfigFieldDef> & Pick<ConfigFieldDef, "key">,
): ConfigFieldDef {
  return {
    key: overrides.key,
    label: overrides.label ?? overrides.key,
    fieldType: overrides.fieldType ?? "STRING",
    required: overrides.required ?? false,
    defaultValue: overrides.defaultValue ?? null,
    valueSource: overrides.valueSource ?? "USER",
    role: overrides.role ?? null,
    hostBinding: overrides.hostBinding ?? null,
    options: overrides.options ?? [],
    helpText: overrides.helpText ?? null,
  };
}

test("a typed address is taken apart however it was written", () => {
  assert.deepEqual(parseHostInput("http://192.168.1.5:8080/qbt/"), {
    scheme: "http",
    host: "192.168.1.5",
    port: "8080",
    path: "/qbt",
  });
  assert.deepEqual(parseHostInput("192.168.1.5:8080"), {
    scheme: null,
    host: "192.168.1.5",
    port: "8080",
    path: "",
  });
  assert.deepEqual(parseHostInput("  https://qbit.example.com  "), {
    scheme: "https",
    host: "qbit.example.com",
    port: "",
    path: "",
  });
  // A bare IPv6 literal is a host, not a host and a port.
  assert.deepEqual(parseHostInput("fd00::1"), {
    scheme: null,
    host: "[fd00::1]",
    port: "",
    path: "",
  });
  assert.deepEqual(parseHostInput("[fd00::1]:8080"), {
    scheme: null,
    host: "[fd00::1]",
    port: "8080",
    path: "",
  });
});

test("anything we should not touch is left alone rather than half-read", () => {
  // Credentials in the URL would have to be dropped or moved, and doing either
  // silently is worse than handing the value to the server as typed.
  assert.equal(parseHostInput("http://admin:pw@nzbget.lan:6789"), null);
  // A scheme that is not http(s) is a real mistake, not a paste to forgive.
  assert.equal(parseHostInput("ftp://files.example.com"), null);
  assert.equal(parseHostInput("   "), null);
  assert.equal(normalizeUrlInput("  ftp://files.example.com "), "ftp://files.example.com");
});

test("the scheme is only guessed when none was written", () => {
  // A service someone stood up themselves, on a port they chose.
  assert.equal(normalizeUrlInput("192.168.1.5:8080"), "http://192.168.1.5:8080");
  assert.equal(normalizeUrlInput("10.0.0.5:9117"), "http://10.0.0.5:9117");
  assert.equal(normalizeUrlInput("100.64.0.3:8080"), "http://100.64.0.3:8080");
  assert.equal(normalizeUrlInput("localhost:6789"), "http://localhost:6789");
  // A single label is a container or a LAN name, never a public site.
  assert.equal(normalizeUrlInput("jackett"), "http://jackett");
  assert.equal(normalizeUrlInput("nzbget.lan"), "http://nzbget.lan");
  // A name on the public internet.
  assert.equal(normalizeUrlInput("nzbgeek.info"), "https://nzbgeek.info");
  assert.equal(normalizeUrlInput("api.nzbgeek.info/api"), "https://api.nzbgeek.info/api");
  assert.equal(normalizeUrlInput("example.com:443"), "https://example.com:443");
  // What the operator wrote always wins.
  assert.equal(normalizeUrlInput("http://nzbgeek.info"), "http://nzbgeek.info");
  assert.equal(normalizeUrlInput("https://192.168.1.5:8080/"), "https://192.168.1.5:8080");
  assert.equal(defaultSchemeFor("fd00::1", ""), "http");
  assert.equal(defaultSchemeFor("seedbox.example.com", ""), "https");
});

test("only an indexer's connection URL field is rewritten", () => {
  const fields = [
    configField({ key: "base_url", role: "CONNECTION_URL" }),
    configField({ key: "api_key" }),
  ];
  const values = normalizeIndexerConfigValues(fields, {
    base_url: "nzbgeek.info/",
    // An API key is not an address and must survive untouched, dots and all.
    api_key: "abc.def",
  });
  assert.equal(values.base_url, "https://nzbgeek.info");
  assert.equal(values.api_key, "abc.def");

  // Nothing to do means the same object back, so no needless re-render.
  const unchanged = { base_url: "https://nzbgeek.info", api_key: "abc" };
  assert.equal(normalizeIndexerConfigValues(fields, unchanged), unchanged);
  // A provider with no connection URL declared is left entirely alone.
  const other = { api_key: "abc" };
  assert.equal(
    normalizeIndexerConfigValues([configField({ key: "api_key" })], other),
    other,
  );
});
