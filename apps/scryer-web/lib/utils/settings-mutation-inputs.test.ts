import assert from "node:assert/strict";
import test from "node:test";
import {
  buildCreateIndexerProxyInput,
  buildDownloadClientConnectionTestInput,
  buildUpdateIndexerProxyInput,
  parseUiDateTimeFormat,
  parseVerificationDepth,
} from "./settings-mutation-inputs.ts";

const proxyDraft = {
  providerType: "trawl" as const,
  name: "  Trawl  ",
  baseUrl: "  http://proxy:8191  ",
  requestTimeoutSeconds: 60,
  username: "",
  password: "",
  hasStoredCredentials: false,
  clearCredentials: false,
  remoteDns: false,
  isEnabled: true,
};

const socksProxyDraft = {
  ...proxyDraft,
  providerType: "socks5" as const,
  name: "Gateway",
  baseUrl: "socks5://gateway:1080",
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

test("challenge solvers never carry transport-only fields", () => {
  // The API rejects credentials and remote DNS on a solver, so the client must
  // not send them even when a stale draft still holds values.
  const stale = {
    ...proxyDraft,
    username: "operator",
    password: "hunter2",
    remoteDns: true,
  };

  const created = buildCreateIndexerProxyInput(stale);
  assert.equal("username" in created, false);
  assert.equal("password" in created, false);
  assert.equal("remoteDns" in created, false);

  const updated = buildUpdateIndexerProxyInput("proxy-1", stale);
  assert.equal("username" in updated, false);
  assert.equal("password" in updated, false);
  assert.equal("remoteDns" in updated, false);
});

test("socks5 creates carry trimmed credentials and the remote-DNS choice", () => {
  const input = buildCreateIndexerProxyInput({
    ...socksProxyDraft,
    username: "  operator  ",
    password: "  hunter2  ",
    remoteDns: true,
  });

  assert.deepEqual(input, {
    providerType: "socks5",
    name: "Gateway",
    baseUrl: "socks5://gateway:1080",
    requestTimeoutSeconds: 60,
    isEnabled: true,
    remoteDns: true,
    username: "operator",
    password: "hunter2",
  });
});

test("http proxies send credentials but never a remote-DNS flag", () => {
  // An HTTP CONNECT proxy always resolves the destination itself, so the API
  // rejects the flag; credentials are still accepted.
  const input = buildCreateIndexerProxyInput({
    ...socksProxyDraft,
    providerType: "http",
    baseUrl: "http://gateway:3128",
    username: "operator",
    remoteDns: true,
  });

  assert.equal(input.username, "operator");
  assert.equal("remoteDns" in input, false);
});

test("socks4 takes remote DNS but never credentials", () => {
  // The HTTP client builds its SOCKS4 connector without auth, so the API
  // rejects credentials rather than dropping them silently on the wire.
  const input = buildCreateIndexerProxyInput({
    ...socksProxyDraft,
    providerType: "socks4",
    baseUrl: "socks4://gateway:1080",
    username: "operator",
    password: "hunter2",
    remoteDns: true,
  });

  assert.equal(input.remoteDns, true);
  assert.equal("username" in input, false);
  assert.equal("password" in input, false);
});

test("socks4 updates never clear credentials it cannot hold", () => {
  const input = buildUpdateIndexerProxyInput("proxy-1", {
    ...socksProxyDraft,
    providerType: "socks4",
    baseUrl: "socks4://gateway:1080",
    hasStoredCredentials: true,
    clearCredentials: true,
  });

  assert.equal("username" in input, false);
  assert.equal("password" in input, false);
});

test("blank credential fields leave a stored secret unchanged", () => {
  const input = buildUpdateIndexerProxyInput("proxy-1", {
    ...socksProxyDraft,
    hasStoredCredentials: true,
    username: "",
    password: "   ",
  });

  // Omission is the "unchanged" signal; an explicit null would clear it.
  assert.equal("username" in input, false);
  assert.equal("password" in input, false);
});

test("a password may be replaced on its own", () => {
  const input = buildUpdateIndexerProxyInput("proxy-1", {
    ...socksProxyDraft,
    hasStoredCredentials: true,
    password: "rotated",
  });

  assert.equal(input.password, "rotated");
  assert.equal("username" in input, false);
});

test("clearing credentials sends explicit nulls and ignores typed values", () => {
  const input = buildUpdateIndexerProxyInput("proxy-1", {
    ...socksProxyDraft,
    hasStoredCredentials: true,
    clearCredentials: true,
    username: "operator",
    password: "hunter2",
  });

  assert.equal(input.username, null);
  assert.equal(input.password, null);
});

test("creates never send a credential clear", () => {
  const input = buildCreateIndexerProxyInput({
    ...socksProxyDraft,
    clearCredentials: true,
  });

  assert.equal("username" in input, false);
  assert.equal("password" in input, false);
});

test("time format values preserve GraphQL enum casing", () => {
  assert.equal(parseUiDateTimeFormat("LOCALE"), "LOCALE");
  assert.equal(parseUiDateTimeFormat("ISO24H"), "ISO24H");
  assert.equal(parseUiDateTimeFormat("locale"), null);
});

test("verification depth values preserve GraphQL enum casing", () => {
  assert.equal(parseVerificationDepth("FULL"), "FULL");
  assert.equal(parseVerificationDepth("QUICK"), "QUICK");
  assert.equal(parseVerificationDepth("full"), null);
  assert.equal(parseVerificationDepth(""), null);
  assert.equal(parseVerificationDepth("SAMPLED"), null);
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
