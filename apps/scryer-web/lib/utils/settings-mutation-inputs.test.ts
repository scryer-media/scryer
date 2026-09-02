import assert from "node:assert/strict";
import test from "node:test";
import {
  buildCreateProxyInput,
  buildDownloadClientConnectionTestInput,
  buildDownloadClientProxyCreateInput,
  buildDownloadClientProxyUpdateInput,
  buildUpdateProxyInput,
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
  clearPassword: false,
  privateKey: "",
  privateKeyPassphrase: "",
  hasStoredPrivateKey: false,
  clearPrivateKey: false,
  peerPublicKey: "",
  presharedKey: "",
  hasStoredPresharedKey: false,
  clearPresharedKey: false,
  tunnelAddresses: "",
  tunnelDnsServers: "",
  tunnelMtu: "",
  hasStoredTunnelMtu: false,
  tunnelKeepaliveSeconds: "",
  hasStoredTunnelKeepaliveSeconds: false,
  remoteDns: false,
  isEnabled: true,
};

const tunnelProxyDraft = {
  ...proxyDraft,
  providerType: "ssh_tunnel" as const,
  name: "Seedbox",
  baseUrl: "ssh://seedbox:22",
};

/// Key-shaped values: 32 bytes of base64 is always 43 characters and a `=`.
const WG_PEER_PUBLIC_KEY = `${"p".repeat(43)}=`;
const WG_PRESHARED_KEY = `${"s".repeat(43)}=`;

const wireguardProxyDraft = {
  ...proxyDraft,
  providerType: "wireguard" as const,
  name: "Home",
  baseUrl: "wireguard://vpn.example:51820",
  peerPublicKey: WG_PEER_PUBLIC_KEY,
  tunnelAddresses: "10.6.0.2/32",
};

const socksProxyDraft = {
  ...proxyDraft,
  providerType: "socks5" as const,
  name: "Gateway",
  baseUrl: "socks5://gateway:1080",
};

test("proxy updates omit immutable provider type", () => {
  const input = buildUpdateProxyInput("proxy-1", proxyDraft);

  assert.deepEqual(input, {
    id: "proxy-1",
    name: "Trawl",
    baseUrl: "http://proxy:8191",
    requestTimeoutSeconds: 60,
    isEnabled: true,
  });
  assert.equal("providerType" in input, false);
});

test("proxy creates include provider type", () => {
  assert.equal(buildCreateProxyInput(proxyDraft).providerType, "trawl");
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

  const created = buildCreateProxyInput(stale);
  assert.equal("username" in created, false);
  assert.equal("password" in created, false);
  assert.equal("remoteDns" in created, false);

  const updated = buildUpdateProxyInput("proxy-1", stale);
  assert.equal("username" in updated, false);
  assert.equal("password" in updated, false);
  assert.equal("remoteDns" in updated, false);
});

test("socks5 creates carry trimmed credentials and the remote-DNS choice", () => {
  const input = buildCreateProxyInput({
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
  const input = buildCreateProxyInput({
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
  const input = buildCreateProxyInput({
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
  const input = buildUpdateProxyInput("proxy-1", {
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
  const input = buildUpdateProxyInput("proxy-1", {
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
  const input = buildUpdateProxyInput("proxy-1", {
    ...socksProxyDraft,
    hasStoredCredentials: true,
    password: "rotated",
  });

  assert.equal(input.password, "rotated");
  assert.equal("username" in input, false);
});

test("clearing credentials sends explicit nulls and ignores typed values", () => {
  const input = buildUpdateProxyInput("proxy-1", {
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
  const input = buildCreateProxyInput({
    ...socksProxyDraft,
    clearCredentials: true,
  });

  assert.equal("username" in input, false);
  assert.equal("password" in input, false);
});

test("tunnels send their SSH credentials and key material", () => {
  const input = buildCreateProxyInput({
    ...tunnelProxyDraft,
    username: "  operator  ",
    password: "  hunter2  ",
    privateKey: "  -----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----  ",
  });

  assert.equal(input.username, "operator");
  assert.equal(input.password, "hunter2");
  assert.equal(
    input.privateKey,
    "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----",
  );
  // A tunnel resolves on the far side, so remote DNS is not its choice to make.
  assert.equal("remoteDns" in input, false);
});

test("a passphrase without a key is withheld, because the API rejects it", () => {
  const withoutKey = buildCreateProxyInput({
    ...tunnelProxyDraft,
    username: "operator",
    password: "hunter2",
    privateKeyPassphrase: "secret",
  });
  assert.equal("privateKeyPassphrase" in withoutKey, false);

  const withPastedKey = buildCreateProxyInput({
    ...tunnelProxyDraft,
    username: "operator",
    privateKey: "-----BEGIN OPENSSH PRIVATE KEY-----",
    privateKeyPassphrase: "secret",
  });
  assert.equal(withPastedKey.privateKeyPassphrase, "secret");

  // A stored key counts: rotating only the passphrase must be possible.
  const withStoredKey = buildUpdateProxyInput("proxy-1", {
    ...tunnelProxyDraft,
    hasStoredPrivateKey: true,
    privateKeyPassphrase: "rotated",
  });
  assert.equal(withStoredKey.privateKeyPassphrase, "rotated");
  assert.equal("privateKey" in withStoredKey, false);
});

test("a tunnel drops its password alone, never its mandatory username", () => {
  const input = buildUpdateProxyInput("proxy-1", {
    ...tunnelProxyDraft,
    hasStoredCredentials: true,
    hasStoredPrivateKey: true,
    username: "operator",
    password: "typed",
    clearPassword: true,
  });

  assert.equal(input.username, "operator");
  assert.equal(input.password, null);
});

test("clearing a tunnel key sends explicit nulls for the key and its passphrase", () => {
  const input = buildUpdateProxyInput("proxy-1", {
    ...tunnelProxyDraft,
    hasStoredPrivateKey: true,
    clearPrivateKey: true,
    privateKey: "ignored",
    privateKeyPassphrase: "ignored",
  });

  assert.equal(input.privateKey, null);
  assert.equal(input.privateKeyPassphrase, null);
});

test("non-tunnel providers never carry key material", () => {
  const stale = {
    ...socksProxyDraft,
    privateKey: "-----BEGIN OPENSSH PRIVATE KEY-----",
    privateKeyPassphrase: "secret",
    hasStoredPrivateKey: true,
    clearPrivateKey: true,
  };

  const created = buildCreateProxyInput(stale);
  assert.equal("privateKey" in created, false);
  assert.equal("privateKeyPassphrase" in created, false);

  const updated = buildUpdateProxyInput("proxy-1", stale);
  assert.equal("privateKey" in updated, false);
  assert.equal("privateKeyPassphrase" in updated, false);
});

test("a WireGuard create sends its keys, addresses and numbers", () => {
  const input = buildCreateProxyInput({
    ...wireguardProxyDraft,
    privateKey: "  key-as-pasted  ",
    presharedKey: `  ${WG_PRESHARED_KEY}  `,
    // One line, one comma-separated line, and blanks that must be dropped.
    tunnelAddresses: "10.6.0.2/32\n fd00::2/128 ,\n",
    tunnelDnsServers: "10.6.0.1, 10.6.0.53",
    tunnelMtu: "1420",
    tunnelKeepaliveSeconds: "0",
  });

  assert.equal(input.privateKey, "key-as-pasted");
  assert.equal(input.peerPublicKey, WG_PEER_PUBLIC_KEY);
  assert.equal(input.presharedKey, WG_PRESHARED_KEY);
  assert.deepEqual(input.tunnelAddresses, ["10.6.0.2/32", "fd00::2/128"]);
  assert.deepEqual(input.tunnelDnsServers, ["10.6.0.1", "10.6.0.53"]);
  assert.equal(input.tunnelMtu, 1420);
  // Zero is a real setting — keepalive off — not an absent one.
  assert.equal(input.tunnelKeepaliveSeconds, 0);
  // WireGuard authenticates with keys, and the API rejects the SSH pair.
  assert.equal("username" in input, false);
  assert.equal("password" in input, false);
  assert.equal("privateKeyPassphrase" in input, false);
  assert.equal("remoteDns" in input, false);
});

test("a WireGuard create omits what it has nothing to say about", () => {
  const input = buildCreateProxyInput(wireguardProxyDraft);

  // Nothing is stored yet, so a blank MTU or keepalive is an omission rather
  // than an explicit "restore the default".
  assert.equal("tunnelMtu" in input, false);
  assert.equal("tunnelKeepaliveSeconds" in input, false);
  assert.equal("presharedKey" in input, false);
  // An empty DNS list is nothing to clear on a create.
  assert.equal("tunnelDnsServers" in input, false);
  assert.deepEqual(input.tunnelAddresses, ["10.6.0.2/32"]);
});

test("a WireGuard update states its lists and keeps its untouched secrets", () => {
  const input = buildUpdateProxyInput("proxy-1", {
    ...wireguardProxyDraft,
    hasStoredPrivateKey: true,
    hasStoredPresharedKey: true,
    // Nothing typed: the stored key and preshared key are kept.
    peerPublicKey: "",
    tunnelDnsServers: "",
  });

  assert.equal("privateKey" in input, false);
  assert.equal("presharedKey" in input, false);
  // The peer's key has no cleared state, so an untouched field is an omission.
  assert.equal("peerPublicKey" in input, false);
  // The lists are always stated, so an emptied DNS field clears them.
  assert.deepEqual(input.tunnelAddresses, ["10.6.0.2/32"]);
  assert.deepEqual(input.tunnelDnsServers, []);
});

test("clearing a WireGuard preshared key sends an explicit null", () => {
  const input = buildUpdateProxyInput("proxy-1", {
    ...wireguardProxyDraft,
    hasStoredPresharedKey: true,
    clearPresharedKey: true,
    presharedKey: "ignored",
  });

  assert.equal(input.presharedKey, null);
});

test("blanking a stored WireGuard number restores the default, once", () => {
  const restored = buildUpdateProxyInput("proxy-1", {
    ...wireguardProxyDraft,
    hasStoredPrivateKey: true,
    hasStoredTunnelMtu: true,
    hasStoredTunnelKeepaliveSeconds: true,
    tunnelMtu: "",
    tunnelKeepaliveSeconds: "",
  });
  assert.equal(restored.tunnelMtu, null);
  assert.equal(restored.tunnelKeepaliveSeconds, null);

  // A field that never held a value has no default to restore, so sending a
  // null would be an intent the operator never expressed.
  const untouched = buildUpdateProxyInput("proxy-1", {
    ...wireguardProxyDraft,
    hasStoredPrivateKey: true,
  });
  assert.equal("tunnelMtu" in untouched, false);
  assert.equal("tunnelKeepaliveSeconds" in untouched, false);
});

test("a WireGuard key never carries an SSH passphrase, even when cleared", () => {
  const typed = buildCreateProxyInput({
    ...wireguardProxyDraft,
    privateKey: "pasted",
    privateKeyPassphrase: "stale",
  });
  assert.equal("privateKeyPassphrase" in typed, false);

  const cleared = buildUpdateProxyInput("proxy-1", {
    ...wireguardProxyDraft,
    hasStoredPrivateKey: true,
    clearPrivateKey: true,
    privateKeyPassphrase: "stale",
  });
  assert.equal(cleared.privateKey, null);
  assert.equal("privateKeyPassphrase" in cleared, false);
});

test("non-WireGuard providers never carry WireGuard fields", () => {
  // A stale draft: the operator filled a WireGuard form, then switched.
  const stale = {
    ...tunnelProxyDraft,
    username: "operator",
    password: "hunter2",
    peerPublicKey: WG_PEER_PUBLIC_KEY,
    presharedKey: WG_PRESHARED_KEY,
    hasStoredPresharedKey: true,
    clearPresharedKey: true,
    tunnelAddresses: "10.6.0.2/32",
    tunnelDnsServers: "10.6.0.1",
    tunnelMtu: "1420",
    hasStoredTunnelMtu: true,
    tunnelKeepaliveSeconds: "25",
    hasStoredTunnelKeepaliveSeconds: true,
  };

  for (const input of [
    buildCreateProxyInput(stale),
    buildUpdateProxyInput("proxy-1", stale),
    buildCreateProxyInput({ ...stale, providerType: "socks5" as const }),
    buildUpdateProxyInput("proxy-1", {
      ...stale,
      providerType: "socks5" as const,
    }),
  ]) {
    for (const field of [
      "peerPublicKey",
      "presharedKey",
      "tunnelAddresses",
      "tunnelDnsServers",
      "tunnelMtu",
      "tunnelKeepaliveSeconds",
    ]) {
      assert.equal(field in input, false, field);
    }
  }
});

test("download client proxy assignments omit on create and clear on update", () => {
  // Nothing is stored yet, so "direct" is an omission rather than a clear.
  assert.deepEqual(buildDownloadClientProxyCreateInput(null), {});
  assert.deepEqual(buildDownloadClientProxyCreateInput("proxy-1"), {
    proxyConfigId: "proxy-1",
  });

  // The editor always knows the intent, so an update states it outright.
  assert.deepEqual(buildDownloadClientProxyUpdateInput("proxy-1"), {
    proxyConfigId: "proxy-1",
  });
  assert.deepEqual(buildDownloadClientProxyUpdateInput(null), {
    proxyConfigId: null,
  });
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

test("a download client test dials through the proxy the draft assigns", () => {
  const config = [{ key: "password", value: "secret" }];

  assert.deepEqual(
    buildDownloadClientConnectionTestInput(null, "sabnzbd", config, "proxy-1"),
    { clientType: "sabnzbd", config, proxyConfigId: "proxy-1" },
  );
  // No assignment means the test goes direct, the same as live traffic would.
  assert.deepEqual(
    buildDownloadClientConnectionTestInput(null, "sabnzbd", config, null),
    { clientType: "sabnzbd", config },
  );
});
