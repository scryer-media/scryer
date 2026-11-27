import test from "node:test";
import assert from "node:assert/strict";

import { userAccountBadges } from "./user-account-origin.ts";
import type { ExternalAccountProvider } from "../types/settings.ts";

const translations: Record<string, string> = {
  "settings.accountOrigin.scryer": "Scryer account",
  "settings.accountOrigin.mediaServerLogin": "Created by {{provider}} login",
  "settings.accountOrigin.mediaServerLoginUnknown":
    "Created by media server login",
  "settings.linkedMediaServer": "Linked to {{provider}}",
};
const translate = (key: string, values: Record<string, string> = {}) =>
  (translations[key] ?? key).replace(
    /\{\{(\w+)\}\}/g,
    (_, name: string) => values[name] ?? "",
  );
const providerLabel = (provider: ExternalAccountProvider) =>
  provider.charAt(0) + provider.slice(1).toLowerCase();

test("an account made in Scryer says so and lists its media-server links", () => {
  assert.deepEqual(
    userAccountBadges(
      { accountKind: "LOCAL", linkedMediaServers: ["JELLYFIN", "PLEX"] },
      translate,
      providerLabel,
    ),
    [
      { kind: "origin", label: "Scryer account" },
      { kind: "linked", label: "Linked to Jellyfin" },
      { kind: "linked", label: "Linked to Plex" },
    ],
  );
  assert.deepEqual(
    userAccountBadges(
      { accountKind: "LOCAL", linkedMediaServers: [] },
      translate,
      providerLabel,
    ),
    [{ kind: "origin", label: "Scryer account" }],
  );
});

test("an account a media-server login created names that server", () => {
  assert.deepEqual(
    userAccountBadges(
      {
        accountKind: "EXTERNAL_AUTO_PROVISIONED",
        linkedMediaServers: ["JELLYFIN"],
      },
      translate,
      providerLabel,
    ),
    [{ kind: "origin", label: "Created by Jellyfin login" }],
  );
});

test("a media-server account whose link is gone still shows its origin", () => {
  assert.deepEqual(
    userAccountBadges(
      { accountKind: "EXTERNAL_AUTO_PROVISIONED", linkedMediaServers: [] },
      translate,
      providerLabel,
    ),
    [{ kind: "origin", label: "Created by media server login" }],
  );
});
