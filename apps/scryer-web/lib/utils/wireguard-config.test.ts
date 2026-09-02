import assert from "node:assert/strict";
import test from "node:test";

import { parseWireguardConfig } from "./wireguard-config.ts";

const CONFIG = `# provided by the VPN
[Interface]
PrivateKey = cHJpdmF0ZQ==
Address = 10.6.0.2/32, fd00::2/128
DNS = 10.6.0.1
MTU = 1420
ListenPort = 51820

[Peer]
PublicKey = cGVlcg==
PresharedKey = cHNr
AllowedIPs = 0.0.0.0/0, ::/0
Endpoint = vpn.example.com:51820   # main
PersistentKeepalive = 25
`;

test("a whole configuration fills every field the form has", () => {
  const parsed = parseWireguardConfig(CONFIG);
  assert.ok(parsed);
  assert.equal(parsed.privateKey, "cHJpdmF0ZQ==");
  assert.equal(parsed.peerPublicKey, "cGVlcg==");
  assert.equal(parsed.presharedKey, "cHNr");
  assert.equal(parsed.endpoint, "vpn.example.com:51820");
  assert.equal(parsed.tunnelAddresses, "10.6.0.2/32\nfd00::2/128");
  assert.equal(parsed.tunnelDnsServers, "10.6.0.1");
  assert.equal(parsed.tunnelMtu, "1420");
  assert.equal(parsed.tunnelKeepaliveSeconds, "25");
  assert.equal(parsed.peerCount, 1);
  // Reported rather than silently dropped: an operator who wrote them deserves
  // to know a tunnel proxy does not use them.
  assert.deepEqual(parsed.ignored, ["ListenPort", "AllowedIPs"]);
});

test("a fragment with no section headers still reads", () => {
  const parsed = parseWireguardConfig(
    "PrivateKey = cHJpdmF0ZQ==\nPublicKey = cGVlcg==\nEndpoint = vpn.test:51820",
  );
  assert.ok(parsed);
  assert.equal(parsed.privateKey, "cHJpdmF0ZQ==");
  assert.equal(parsed.peerPublicKey, "cGVlcg==");
  assert.equal(parsed.endpoint, "vpn.test:51820");
  assert.equal(parsed.peerCount, 0);
});

test("only the first peer is imported, and the file says how many there were", () => {
  const parsed = parseWireguardConfig(`[Interface]
PrivateKey = cHJpdmF0ZQ==
Address = 10.6.0.2/32

[Peer]
PublicKey = Zmlyc3Q=
Endpoint = first.test:51820

[Peer]
PublicKey = c2Vjb25k
Endpoint = second.test:51820
`);
  assert.ok(parsed);
  assert.equal(parsed.peerCount, 2);
  assert.equal(parsed.peerPublicKey, "Zmlyc3Q=");
  assert.equal(parsed.endpoint, "first.test:51820");
});

test("repeated address lines and a keepalive of zero survive", () => {
  const parsed = parseWireguardConfig(`[Interface]
Address = 10.6.0.2/32
Address = fd00::2/128
PrivateKey = cHJpdmF0ZQ==

[Peer]
PublicKey = cGVlcg==
PersistentKeepalive = 0
`);
  assert.ok(parsed);
  assert.equal(parsed.tunnelAddresses, "10.6.0.2/32\nfd00::2/128");
  // Zero is a value: it switches keepalive off, which is not the same as
  // leaving the engine's default in place.
  assert.equal(parsed.tunnelKeepaliveSeconds, "0");
});

test("text that is not a configuration is refused rather than half-read", () => {
  assert.equal(parseWireguardConfig(""), null);
  assert.equal(parseWireguardConfig("the quick brown fox"), null);
  assert.equal(parseWireguardConfig("[Interface]\n# nothing else"), null);
  // A key with no value tells us nothing, so it does not count as recognised.
  assert.equal(parseWireguardConfig("PrivateKey ="), null);
});
