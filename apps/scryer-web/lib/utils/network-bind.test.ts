import test from "node:test";
import assert from "node:assert/strict";

import { t } from "../i18n/index.ts";
import {
  NETWORK_FALLBACK_KEYS,
  hostForReach,
  reachForHost,
  socketHost,
} from "./network-bind.ts";

test("socket addresses give back their host in every spelling", () => {
  assert.equal(socketHost("127.0.0.1:8080"), "127.0.0.1");
  assert.equal(socketHost("0.0.0.0:8080"), "0.0.0.0");
  assert.equal(socketHost("[::1]:8080"), "::1");
  assert.equal(socketHost("[::]:18080"), "::");
  assert.equal(socketHost("::1"), "::1");
  assert.equal(socketHost(" 192.0.2.10:9000 "), "192.0.2.10");
});

test("the page opens on the choice the host already is", () => {
  for (const host of ["127.0.0.1", "127.0.0.2", "::1", "0:0:0:0:0:0:0:1"]) {
    assert.equal(reachForHost(host), "local", host);
  }
  assert.equal(reachForHost("0.0.0.0"), "network");
  assert.equal(reachForHost("192.0.2.10"), "custom");
  assert.equal(reachForHost("::"), "custom");
});

test("each choice saves its host and an empty custom field saves nothing", () => {
  assert.equal(hostForReach("local", "192.0.2.10"), "127.0.0.1");
  assert.equal(hostForReach("network", ""), "0.0.0.0");
  assert.equal(hostForReach("custom", " 192.0.2.10 "), "192.0.2.10");
  assert.equal(hostForReach("custom", "  "), null);
});

test("every fallback code has base copy", () => {
  for (const [code, key] of Object.entries(NETWORK_FALLBACK_KEYS)) {
    assert.notEqual(t(key, "eng"), key, `${code} has no base copy`);
  }
});
