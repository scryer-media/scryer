import assert from "node:assert/strict";
import test from "node:test";

import de from "../i18n/locales/de.ts";
import en from "../i18n/locales/en.ts";
import es from "../i18n/locales/es.ts";
import fr from "../i18n/locales/fr.ts";
import it from "../i18n/locales/it.ts";
import ja from "../i18n/locales/ja.ts";
import ko from "../i18n/locales/ko.ts";
import pt_BR from "../i18n/locales/pt_BR.ts";
import ru from "../i18n/locales/ru.ts";
import zh_CN from "../i18n/locales/zh_CN.ts";
import type { LocaleDictionary } from "../i18n/types.ts";
import {
  PROXY_DEFAULT_BASE_URLS,
  PROXY_FAMILIES,
  PROXY_FAMILY_LABEL_KEYS,
  PROXY_INITIAL_DRAFT,
  PROXY_PROVIDER_TYPES,
  PROXY_PROVIDER_TYPES_BY_FAMILY,
  formatProxyProvider,
  formatTunnelList,
  groupProxiesByFamily,
  isProxyProviderType,
  isSolverProxyProvider,
  isSshTunnelProxyProvider,
  isTransportProxyProvider,
  isTunnelProxyProvider,
  isWireguardProxyProvider,
  looksLikeWireguardKey,
  normalizeProxyDraft,
  normalizeProxyEndpoint,
  proxyProviderFamily,
  splitTunnelList,
  stripConfigAssignment,
  supportsProxyCredentials,
  supportsProxyHostKey,
  supportsProxyPrivateKey,
  supportsProxyPrivateKeyPassphrase,
  supportsProxyRemoteDns,
  supportsProxyWireguardFields,
} from "./proxies.ts";

const LOCALES: Array<[string, LocaleDictionary]> = [
  ["de", de],
  ["en", en],
  ["es", es],
  ["fr", fr],
  ["it", it],
  ["ja", ja],
  ["ko", ko],
  ["pt_BR", pt_BR],
  ["ru", ru],
  ["zh_CN", zh_CN],
];

/// Every key the proxies page, the assignment selects and the tunnel editor
/// read. A locale missing one of these renders a raw key at the operator.
const PROXY_LOCALE_KEYS = [
  "settings.proxies",
  "settings.proxiesHelp",
  "settings.proxyFamilySolver",
  "settings.proxyFamilyStandard",
  "settings.proxyFamilyTunnel",
  "settings.proxyFamilyOther",
  "settings.proxyHealth",
  "settings.proxyLastError",
  "settings.proxyHealthHealthy",
  "settings.proxyHealthUnhealthy",
  "settings.proxyEmpty",
  "settings.proxyTest",
  "settings.proxyTimeout",
  "settings.proxyCreateNew",
  "settings.proxyCreate",
  "settings.proxyUpdate",
  "settings.proxyDeleteConfirmDescription",
  "settings.proxyValidation",
  "settings.proxyEndpoint",
  "settings.proxyEndpointHelp",
  "settings.proxyAssignment",
  "settings.proxyDirect",
  "settings.proxyMissing",
  "settings.proxyMissingHelp",
  "settings.proxyDisabledHelp",
  "settings.proxyDisabledSuffix",
  "settings.proxyUsername",
  "settings.proxyPassword",
  "settings.proxyCredentialsStored",
  "settings.proxyCredentialUnchanged",
  "settings.proxyCredentialsHelp",
  "settings.proxyCredentialsStoredHelp",
  "settings.proxyClearCredentials",
  "settings.proxyClearPassword",
  "settings.proxyRemoteDns",
  "settings.proxyRemoteDnsHelp",
  "settings.proxyTunnelAuthHelp",
  "settings.proxyPrivateKey",
  "settings.proxyPrivateKeyHelp",
  "settings.proxyPrivateKeyStored",
  "settings.proxyPrivateKeyUnchanged",
  "settings.proxyClearPrivateKey",
  "settings.proxyPrivateKeyPassphrase",
  "settings.proxyPrivateKeyPassphraseHelp",
  "settings.proxyHostKey",
  "settings.proxyHostKeyPinnedAt",
  "settings.proxyHostKeyUnpinned",
  "settings.proxyHostKeyReset",
  "settings.proxyHostKeyResetDescription",
  "settings.proxyValidationTunnelUsername",
  "settings.proxyValidationTunnelAuth",
  "settings.proxyEndpointHelpWireguard",
  "settings.proxyPrivateKeyHelpWireguard",
  "settings.proxyPeerPublicKey",
  "settings.proxyPeerPublicKeyHelp",
  "settings.proxyPresharedKey",
  "settings.proxyPresharedKeyHelp",
  "settings.proxyPresharedKeyStored",
  "settings.proxyPresharedKeyUnchanged",
  "settings.proxyClearPresharedKey",
  "settings.proxyTunnelAddresses",
  "settings.proxyTunnelAddressesHelp",
  "settings.proxyTunnelDnsServers",
  "settings.proxyTunnelDnsServersHelp",
  "settings.proxyTunnelMtu",
  "settings.proxyTunnelMtuHelp",
  "settings.proxyTunnelKeepalive",
  "settings.proxyTunnelKeepaliveHelp",
  "settings.proxyTunnelPublicKey",
  "settings.proxyTunnelPublicKeyHelp",
  "settings.proxyTunnelPublicKeyPending",
  "settings.proxyTunnelPublicKeyCopy",
  "settings.proxyImportConfig",
  "settings.proxyImportConfigHelp",
  "settings.proxyImportConfigApply",
  "settings.proxyImportConfigFile",
  "status.proxyImportConfigFilled",
  "status.proxyImportConfigUnreadable",
  "status.proxyImportConfigFirstPeer",
  "status.proxyImportConfigIgnored",
  "settings.proxyValidationWireguardPrivateKey",
  "settings.proxyValidationWireguardPeerPublicKey",
  "settings.proxyValidationWireguardAddresses",
  "settings.proxyValidationWireguardKeyShape",
  "settings.proxyValidationWireguardMtu",
  "settings.proxyValidationWireguardKeepalive",
  "settings.downloadClientProxyHelp",
  "status.proxyCreated",
  "status.proxyUpdated",
  "status.proxyDeleted",
  "status.proxyTestPassed",
  "status.proxyTestFailed",
  "status.editingProxy",
  "status.proxyHostKeyReset",
  "status.proxyTunnelPublicKeyCopied",
  "status.proxyTunnelPublicKeyCopyFailed",
];

test("every provider belongs to exactly one family", () => {
  const grouped = PROXY_FAMILIES.flatMap(
    (family) => PROXY_PROVIDER_TYPES_BY_FAMILY[family],
  );
  assert.deepEqual([...grouped].sort(), [...PROXY_PROVIDER_TYPES].sort());
  assert.equal(new Set(grouped).size, grouped.length);

  assert.equal(proxyProviderFamily("byparr"), "solver");
  assert.equal(proxyProviderFamily("socks5"), "standard");
  assert.equal(proxyProviderFamily("ssh_tunnel"), "tunnel");
  // A second tunnel technology slots in beside the first rather than moving
  // anything: the family list and the dropdown order come from one map.
  assert.equal(proxyProviderFamily("wireguard"), "tunnel");
  assert.equal(isProxyProviderType("wireguard"), true);
  assert.deepEqual(PROXY_PROVIDER_TYPES_BY_FAMILY.tunnel, [
    "ssh_tunnel",
    "wireguard",
  ]);
  // A provider from a newer server still has no family rather than a wrong one.
  assert.equal(proxyProviderFamily("hypothetical"), null);
  assert.equal(isProxyProviderType("hypothetical"), false);
});

test("provider labels are product names, and unknown values render verbatim", () => {
  assert.equal(formatProxyProvider("byparr"), "Byparr");
  assert.equal(formatProxyProvider("socks4"), "SOCKS4");
  assert.equal(formatProxyProvider("ssh_tunnel"), "SSH tunnel");
  assert.equal(formatProxyProvider("wireguard"), "WireGuard");
  assert.equal(formatProxyProvider("hypothetical"), "hypothetical");
});

test("the field predicates match the rules the API enforces", () => {
  // Credentials: not solvers, and not SOCKS4, whose connector drops them.
  // WireGuard authenticates with keys and rejects the pair outright.
  assert.equal(supportsProxyCredentials("http"), true);
  assert.equal(supportsProxyCredentials("socks5"), true);
  assert.equal(supportsProxyCredentials("ssh_tunnel"), true);
  assert.equal(supportsProxyCredentials("socks4"), false);
  assert.equal(supportsProxyCredentials("byparr"), false);
  assert.equal(supportsProxyCredentials("wireguard"), false);

  // Remote DNS is the SOCKS-only socks4a / socks5h behaviour.
  assert.equal(supportsProxyRemoteDns("socks4"), true);
  assert.equal(supportsProxyRemoteDns("socks5"), true);
  assert.equal(supportsProxyRemoteDns("http"), false);
  assert.equal(supportsProxyRemoteDns("ssh_tunnel"), false);

  assert.equal(supportsProxyRemoteDns("wireguard"), false);

  // Key material is tunnels only, in each one's own encoding.
  assert.equal(supportsProxyPrivateKey("ssh_tunnel"), true);
  assert.equal(supportsProxyPrivateKey("wireguard"), true);
  assert.equal(supportsProxyPrivateKey("socks5"), false);

  // The SSH-shaped extras stop at the SSH tunnel: a WireGuard key has nothing
  // to unlock and its peer is authenticated by the key the operator typed.
  assert.equal(supportsProxyPrivateKeyPassphrase("ssh_tunnel"), true);
  assert.equal(supportsProxyPrivateKeyPassphrase("wireguard"), false);
  assert.equal(supportsProxyHostKey("ssh_tunnel"), true);
  assert.equal(supportsProxyHostKey("wireguard"), false);

  // ...and the WireGuard-shaped ones stop at WireGuard.
  assert.equal(supportsProxyWireguardFields("wireguard"), true);
  assert.equal(supportsProxyWireguardFields("ssh_tunnel"), false);
  assert.equal(supportsProxyWireguardFields("socks5"), false);

  assert.equal(isSolverProxyProvider("trawl"), true);
  assert.equal(isTransportProxyProvider("http"), true);
  assert.equal(isTransportProxyProvider("ssh_tunnel"), false);
  assert.equal(isTunnelProxyProvider("ssh_tunnel"), true);
  assert.equal(isTunnelProxyProvider("wireguard"), true);
  assert.equal(isSshTunnelProxyProvider("wireguard"), false);
  assert.equal(isWireguardProxyProvider("wireguard"), true);
});

test("a WireGuard key is 32 bytes of base64, and the check says so cheaply", () => {
  assert.equal(looksLikeWireguardKey(`${"a".repeat(43)}=`), true);
  // Trimmed, because it arrives pasted out of a `wg` config.
  assert.equal(looksLikeWireguardKey(`  ${"a".repeat(43)}=  `), true);
  // 43 characters and no `=`, 44 without the `=`, and a PEM banner.
  assert.equal(looksLikeWireguardKey("a".repeat(43)), false);
  assert.equal(looksLikeWireguardKey("a".repeat(44)), false);
  assert.equal(looksLikeWireguardKey(`${"a".repeat(43)}=extra`), false);
  assert.equal(looksLikeWireguardKey("-----BEGIN OPENSSH PRIVATE KEY-----"), false);
  assert.equal(looksLikeWireguardKey(""), false);
});

test("tunnel lists split on newlines and commas alike, dropping blanks", () => {
  assert.deepEqual(splitTunnelList("10.6.0.2/32"), ["10.6.0.2/32"]);
  assert.deepEqual(splitTunnelList("10.6.0.2/32, fd00::2/128"), [
    "10.6.0.2/32",
    "fd00::2/128",
  ]);
  assert.deepEqual(splitTunnelList(" 10.6.0.2/32 \n\n fd00::2/128 ,\n"), [
    "10.6.0.2/32",
    "fd00::2/128",
  ]);
  assert.deepEqual(splitTunnelList("   "), []);
  assert.deepEqual(splitTunnelList(""), []);

  // Round trip: what the editor shows for a stored list splits back to it.
  const stored = ["10.6.0.2/32", "fd00::2/128"];
  assert.deepEqual(splitTunnelList(formatTunnelList(stored)), stored);
  assert.equal(formatTunnelList([]), "");
});

test("grouping keeps family order and never drops an unknown provider", () => {
  const groups = groupProxiesByFamily([
    { id: "a", providerType: "socks5" },
    { id: "b", providerType: "byparr" },
    { id: "c", providerType: "hypothetical" },
    { id: "d", providerType: "ssh_tunnel" },
    { id: "e", providerType: "wireguard" },
  ]);

  assert.deepEqual(
    groups.map((group) => group.family),
    ["solver", "standard", "tunnel", null],
  );
  // Both tunnels land in the one tunnel group, in list order.
  assert.deepEqual(
    groups.flatMap((group) => group.proxies.map((proxy) => proxy.id)),
    ["b", "a", "d", "e", "c"],
  );
  // Empty families are not rendered as headings over nothing.
  assert.deepEqual(
    groupProxiesByFamily([{ id: "a", providerType: "http" }]).map(
      (group) => group.family,
    ),
    ["standard"],
  );
});

test("every provider has a default endpoint matching its own scheme", () => {
  for (const providerType of PROXY_PROVIDER_TYPES) {
    const defaultUrl = PROXY_DEFAULT_BASE_URLS[providerType];
    assert.equal(typeof defaultUrl, "string", providerType);
    assert.ok(defaultUrl.includes("://"), `${providerType}: ${defaultUrl}`);
  }
  assert.ok(PROXY_DEFAULT_BASE_URLS.ssh_tunnel.startsWith("ssh://"));
  // WireGuard's own listen port, and what every `wg-quick` config uses.
  assert.equal(
    PROXY_DEFAULT_BASE_URLS.wireguard,
    "wireguard://localhost:51820",
  );
  assert.equal(PROXY_INITIAL_DRAFT.baseUrl, PROXY_DEFAULT_BASE_URLS.byparr);
  // A fresh draft opts into nothing that has to be cleared later.
  assert.equal(PROXY_INITIAL_DRAFT.clearCredentials, false);
  assert.equal(PROXY_INITIAL_DRAFT.clearPassword, false);
  assert.equal(PROXY_INITIAL_DRAFT.clearPrivateKey, false);
  assert.equal(PROXY_INITIAL_DRAFT.hasStoredPrivateKey, false);
  assert.equal(PROXY_INITIAL_DRAFT.clearPresharedKey, false);
  assert.equal(PROXY_INITIAL_DRAFT.hasStoredPresharedKey, false);
  // ...and nothing that would be sent as an explicit "restore the default".
  assert.equal(PROXY_INITIAL_DRAFT.tunnelMtu, "");
  assert.equal(PROXY_INITIAL_DRAFT.hasStoredTunnelMtu, false);
  assert.equal(PROXY_INITIAL_DRAFT.tunnelKeepaliveSeconds, "");
  assert.equal(PROXY_INITIAL_DRAFT.hasStoredTunnelKeepaliveSeconds, false);
});

test("every locale carries every proxy string", () => {
  const missing: string[] = [];
  for (const [name, dictionary] of LOCALES) {
    for (const key of [
      ...PROXY_LOCALE_KEYS,
      ...PROXY_FAMILIES.map((family) => PROXY_FAMILY_LABEL_KEYS[family]),
    ]) {
      if (typeof dictionary[key] !== "string" || dictionary[key].length === 0) {
        missing.push(`${name} -> ${key}`);
      }
    }
  }
  assert.deepEqual(missing, []);
});

test("interpolated proxy strings keep their placeholders in every locale", () => {
  for (const [name, dictionary] of LOCALES) {
    for (const key of [
      "settings.proxyDeleteConfirmDescription",
      "status.proxyDeleted",
      "status.editingProxy",
      "status.proxyHostKeyReset",
    ]) {
      assert.match(dictionary[key], /\{\{name\}\}/, `${name} -> ${key}`);
    }
    assert.match(
      dictionary["settings.proxyHostKeyPinnedAt"],
      /\{\{time\}\}/,
      `${name} -> settings.proxyHostKeyPinnedAt`,
    );
    // The WireGuard bounds are interpolated from the constants rather than
    // written into the copy, so no locale can drift from the engine.
    for (const key of [
      "settings.proxyTunnelMtuHelp",
      "settings.proxyValidationWireguardMtu",
    ]) {
      assert.match(dictionary[key], /\{\{min\}\}/, `${name} -> ${key}`);
      assert.match(dictionary[key], /\{\{max\}\}/, `${name} -> ${key}`);
    }
    for (const key of [
      "settings.proxyTunnelMtuHelp",
      "settings.proxyTunnelKeepaliveHelp",
    ]) {
      assert.match(dictionary[key], /\{\{default\}\}/, `${name} -> ${key}`);
    }
    assert.match(
      dictionary["settings.proxyValidationWireguardKeyShape"],
      /\{\{field\}\}/,
      `${name} -> settings.proxyValidationWireguardKeyShape`,
    );
    assert.match(
      dictionary["status.proxyImportConfigFirstPeer"],
      /\{\{count\}\}/,
      `${name} -> status.proxyImportConfigFirstPeer`,
    );
    assert.match(
      dictionary["status.proxyImportConfigIgnored"],
      /\{\{keys\}\}/,
      `${name} -> status.proxyImportConfigIgnored`,
    );
  }
});

test("the private-key help is the backend's own sentence in English", () => {
  // The engine rejects anything else at connect time with exactly this text,
  // so the form must not paraphrase it.
  assert.equal(
    en["settings.proxyPrivateKeyHelp"],
    "only Ed25519 private keys are supported; generate one with " +
      "`ssh-keygen -t ed25519` and paste the OpenSSH private key",
  );
  // Every locale names the algorithm and the command, whatever the wording.
  for (const [name, dictionary] of LOCALES) {
    const help = dictionary["settings.proxyPrivateKeyHelp"];
    assert.match(help, /Ed25519/, name);
    assert.match(help, /ssh-keygen -t ed25519/, name);
  }
});

test("the WireGuard key help is the backend's own sentence in English", () => {
  // `WIREGUARD_KEY_MESSAGE`, crates/scryer-tunnel/src/wireguard/spec.rs. The
  // save path, the connect path and the health probe all say this, so the form
  // must not paraphrase it.
  assert.equal(
    en["settings.proxyPrivateKeyHelpWireguard"],
    "WireGuard keys are 32 bytes of base64, exactly as `wg genkey` prints " +
      "them (44 characters ending in `=`)",
  );
  // Every locale keeps the command that produces one, whatever the wording.
  for (const [name, dictionary] of LOCALES) {
    assert.match(
      dictionary["settings.proxyPrivateKeyHelpWireguard"],
      /wg genkey/,
      name,
    );
    assert.match(
      dictionary["settings.proxyValidationWireguardKeyShape"],
      /wg genkey/,
      name,
    );
  }
});

test("every field takes a pasted configuration line", () => {
  // An operator has the file open in front of them, so the whole line is at
  // least as likely as the value alone.
  assert.equal(
    stripConfigAssignment("PublicKey = cGVlcg==", ["publickey"]),
    "cGVlcg==",
  );
  assert.equal(
    stripConfigAssignment("  privatekey=c2VjcmV0  ", ["privatekey"]),
    "c2VjcmV0",
  );
  // The key has to be a name this field answers to: a base64 value merely ends
  // in `=` and is kept whole, and so is any other assignment.
  assert.equal(stripConfigAssignment("cGVlcg==", ["publickey"]), "cGVlcg==");
  assert.equal(
    stripConfigAssignment("Name = value", ["publickey"]),
    "Name = value",
  );
  // Comments go, but only where one can legitimately start.
  assert.equal(
    stripConfigAssignment("Endpoint = vpn.test:51820 # main", ["endpoint"]),
    "vpn.test:51820",
  );
  // A PEM block is multi-line and is never read as an assignment.
  const pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
  assert.equal(stripConfigAssignment(pem, ["privatekey"]), pem);
});

test("an endpoint without a scheme takes its provider's own", () => {
  assert.equal(
    normalizeProxyEndpoint("wireguard", "Endpoint = vpn.test:51820"),
    "wireguard://vpn.test:51820",
  );
  assert.equal(
    normalizeProxyEndpoint("ssh_tunnel", "seedbox.test:2222"),
    "ssh://seedbox.test:2222",
  );
  assert.equal(normalizeProxyEndpoint("socks5", "127.0.0.1:1080"), "socks5://127.0.0.1:1080");
  assert.equal(normalizeProxyEndpoint("byparr", "localhost:8191"), "http://localhost:8191");
  // A bare IPv6 literal is a host, so it is bracketed rather than read as a
  // host and a nonsense port.
  assert.equal(normalizeProxyEndpoint("wireguard", "fd00::1"), "wireguard://[fd00::1]");
  assert.equal(
    normalizeProxyEndpoint("wireguard", "[fd00::1]:51820"),
    "wireguard://[fd00::1]:51820",
  );
  // A scheme the operator actually wrote is never rewritten: that would hide a
  // real mistake rather than forgive a paste.
  assert.equal(normalizeProxyEndpoint("wireguard", "https://vpn.test"), "https://vpn.test");
  // A provider only a newer server knows has no scheme of ours to supply.
  assert.equal(normalizeProxyEndpoint("something-new", "vpn.test"), "vpn.test");
  assert.equal(normalizeProxyEndpoint("wireguard", "   "), "");
});

test("a list field takes lines, commas and whole configuration lines", () => {
  assert.deepEqual(
    splitTunnelList("Address = 10.6.0.2/32, fd00::2/128\n# spare\nDNS = 10.6.0.1"),
    ["10.6.0.2/32", "fd00::2/128", "10.6.0.1"],
  );
  assert.deepEqual(splitTunnelList("10.6.0.2/32,\n\n"), ["10.6.0.2/32"]);
});

test("a draft is parsed on save so what is stored is what is shown", () => {
  const draft = normalizeProxyDraft({
    ...PROXY_INITIAL_DRAFT,
    providerType: "wireguard",
    baseUrl: "Endpoint = vpn.test:51820",
    privateKey: "PrivateKey = c2VjcmV0",
    peerPublicKey: "PublicKey = cGVlcg==",
    presharedKey: "PresharedKey = cHNr",
    tunnelAddresses: "Address = 10.6.0.2/32, fd00::2/128",
    tunnelDnsServers: "DNS = 10.6.0.1",
    tunnelMtu: "MTU = 1420",
    tunnelKeepaliveSeconds: "PersistentKeepalive = 25",
  });
  assert.equal(draft.baseUrl, "wireguard://vpn.test:51820");
  assert.equal(draft.privateKey, "c2VjcmV0");
  assert.equal(draft.peerPublicKey, "cGVlcg==");
  assert.equal(draft.presharedKey, "cHNr");
  assert.equal(draft.tunnelAddresses, "10.6.0.2/32\nfd00::2/128");
  assert.equal(draft.tunnelDnsServers, "10.6.0.1");
  assert.equal(draft.tunnelMtu, "1420");
  assert.equal(draft.tunnelKeepaliveSeconds, "25");
  // Normalizing again changes nothing: the operator can save twice.
  assert.deepEqual(normalizeProxyDraft(draft), draft);

  // Another provider's endpoint is cleaned too, and its own fields are left
  // exactly as typed — a password can contain anything.
  const ssh = normalizeProxyDraft({
    ...PROXY_INITIAL_DRAFT,
    providerType: "ssh_tunnel",
    baseUrl: "seedbox.test:2222",
    password: "hunter2 # not a comment",
  });
  assert.equal(ssh.baseUrl, "ssh://seedbox.test:2222");
  assert.equal(ssh.password, "hunter2 # not a comment");
});
