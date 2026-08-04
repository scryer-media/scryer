import assert from "node:assert/strict";
import test from "node:test";
import {
  buildCreateIndexerProxyInput,
  buildDownloadClientConnectionTestInput,
  buildUpdateIndexerProxyInput,
  parseUiDateTimeFormat,
} from "./settings-mutation-inputs.ts";

const proxyDraft = {
  providerType: "trawl" as const,
  name: "  Trawl  ",
  baseUrl: "  http://proxy:8191  ",
  requestTimeoutSeconds: 60,
  isEnabled: true,
};

test("indexer proxy updates omit immutable provider type", () => {
  const input = buildUpdateIndexerProxyInput("proxy-1", proxyDraft);

  assert.deepEqual(input, {
    id: "proxy-1",
    name: "Trawl",
    baseUrl: "http://proxy:8191",
    requestTimeoutSeconds: 60,
    isEnabled: true,
  });
  assert.equal("providerType" in input, false);
});

test("indexer proxy creates include provider type", () => {
  assert.equal(buildCreateIndexerProxyInput(proxyDraft).providerType, "trawl");
});

test("time format values preserve GraphQL enum casing", () => {
  assert.equal(parseUiDateTimeFormat("LOCALE"), "LOCALE");
  assert.equal(parseUiDateTimeFormat("ISO24H"), "ISO24H");
  assert.equal(parseUiDateTimeFormat("locale"), null);
});

test("download client tests include only an editing client id", () => {
  const config = [{ key: "password", value: "secret" }];

  assert.deepEqual(
    buildDownloadClientConnectionTestInput("client-1", "qbittorrent", config),
    { id: "client-1", clientType: "qbittorrent", config },
  );
  assert.deepEqual(
    buildDownloadClientConnectionTestInput(null, "qbittorrent", config),
    { clientType: "qbittorrent", config },
  );
});
