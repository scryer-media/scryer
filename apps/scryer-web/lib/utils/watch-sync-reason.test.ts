import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  WATCH_SYNC_REASON_KEYS,
  watchSyncReasonText,
} from "./watch-sync-reason.ts";

const dictionary = en as Record<string, string>;
const t = (key: string) => dictionary[key] ?? key;

test("a known code is said in translation, whatever the server's sentence says", () => {
  assert.equal(
    watchSyncReasonText(t, "WATCH_SYNC_ACCESS_DENIED", "status 401"),
    dictionary["settings.watchSyncReason.accessDenied"],
  );
});

test("an unknown code falls back to the server's sentence", () => {
  assert.equal(
    watchSyncReasonText(t, "WATCH_SYNC_SOMETHING_NEWER", "a newer condition"),
    "a newer condition",
  );
});

test("a sweep recorded before codes were stored shows its sentence", () => {
  assert.equal(watchSyncReasonText(t, null, "status 401"), "status 401");
  assert.equal(watchSyncReasonText(t, null, null), null);
});

test("every watch-sync reason has an English translation", () => {
  for (const [code, key] of Object.entries(WATCH_SYNC_REASON_KEYS)) {
    assert.ok(code.startsWith("WATCH_SYNC_"), code);
    assert.equal(typeof dictionary[key], "string", `${code} → ${key}`);
  }
});

// Mirrors `ALL` in crates/scryer-application/src/media_server_signals/reasons.rs,
// whose test pins the same count.
test("the client knows exactly the server's watch-sync reasons", () => {
  assert.deepEqual(Object.keys(WATCH_SYNC_REASON_KEYS).sort(), [
    "WATCH_SYNC_ACCESS_DENIED",
    "WATCH_SYNC_BASE_URL_INVALID",
    "WATCH_SYNC_CREDENTIAL_MISSING",
    "WATCH_SYNC_FILTER_NOT_HONOURED",
    "WATCH_SYNC_ITEMS_UNREADABLE",
    "WATCH_SYNC_PAGING_NOT_HONOURED",
    "WATCH_SYNC_PARTICIPANT_ID_MISSING",
    "WATCH_SYNC_PLEX_ACCOUNT_ID_INVALID",
    "WATCH_SYNC_PLEX_SERVER_NOT_SELECTED",
    "WATCH_SYNC_PLEX_SERVER_UNREACHABLE",
    "WATCH_SYNC_PROVIDER_UNSUPPORTED",
    "WATCH_SYNC_REQUEST_FAILED",
    "WATCH_SYNC_REQUEST_REJECTED",
    "WATCH_SYNC_RESPONSE_UNREADABLE",
    "WATCH_SYNC_SOURCE_UNAVAILABLE",
    "WATCH_SYNC_STORE_FAILED",
    "WATCH_SYNC_UNMAPPED_PLAYED_MOVIES",
  ]);
});
