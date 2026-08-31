import assert from "node:assert/strict";
import test from "node:test";
import {
  applyDelayProfileProtocolMode,
  buildDelayProfileTemplate,
  delayProfileProtocolMode,
} from "./delay-profiles.ts";

test("delay-profile protocol modes map eligibility and preference", () => {
  const profile = buildDelayProfileTemplate([]);

  assert.deepEqual(applyDelayProfileProtocolMode(profile, "preferUsenet"), {
    ...profile,
    enable_usenet: true,
    enable_torrent: true,
    preferred_protocol: "USENET",
  });
  assert.deepEqual(applyDelayProfileProtocolMode(profile, "preferTorrent"), {
    ...profile,
    enable_usenet: true,
    enable_torrent: true,
    preferred_protocol: "TORRENT",
  });
  assert.deepEqual(applyDelayProfileProtocolMode(profile, "onlyUsenet"), {
    ...profile,
    enable_usenet: true,
    enable_torrent: false,
    preferred_protocol: "USENET",
  });
  assert.deepEqual(applyDelayProfileProtocolMode(profile, "onlyTorrent"), {
    ...profile,
    enable_usenet: false,
    enable_torrent: true,
    preferred_protocol: "TORRENT",
  });
});

test("delay-profile protocol mode derives legacy defaults", () => {
  const profile = buildDelayProfileTemplate([]);

  assert.equal(delayProfileProtocolMode(profile), "preferUsenet");
  assert.equal(
    delayProfileProtocolMode({ ...profile, preferred_protocol: "TORRENT" }),
    "preferTorrent",
  );
  assert.equal(
    delayProfileProtocolMode({ ...profile, enable_torrent: false }),
    "onlyUsenet",
  );
  assert.equal(
    delayProfileProtocolMode({ ...profile, enable_usenet: false }),
    "onlyTorrent",
  );
});
