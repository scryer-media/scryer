import assert from "node:assert/strict";
import test from "node:test";

import {
  fetchProtectedMediaServerAvatar,
  isProtectedMediaServerAvatarUrl,
} from "./authenticated-avatar.ts";

test("only same-origin media-server avatar routes receive authentication", () => {
  const origin = "https://scryer.example.test";
  assert.equal(
    isProtectedMediaServerAvatarUrl(
      "/api/media-server-avatars/connection/user/tag",
      origin,
    ),
    true,
  );
  assert.equal(
    isProtectedMediaServerAvatarUrl(
      "https://scryer.example.test/api/media-server-avatars/connection/user/tag",
      origin,
    ),
    true,
  );
  assert.equal(
    isProtectedMediaServerAvatarUrl(
      "https://emby.example.test/api/media-server-avatars/connection/user/tag",
      origin,
    ),
    false,
  );
  assert.equal(
    isProtectedMediaServerAvatarUrl("/ordinary-avatar.png", origin),
    false,
  );
});

test("authenticated avatar fetch sends bearer in a header and returns an image blob", async () => {
  const calls: Array<{ input: RequestInfo | URL; init?: RequestInit }> = [];
  const fetchImage: typeof fetch = async (input, init) => {
    calls.push({ input, init });
    return new Response(
      new Blob([new Uint8Array([1, 2, 3])], { type: "image/png" }),
    );
  };
  const url = "/api/media-server-avatars/connection/user/tag";

  const blob = await fetchProtectedMediaServerAvatar(
    url,
    "https://scryer.example.test",
    "secret-token",
    new AbortController().signal,
    fetchImage,
  );

  assert.equal(blob.type, "image/png");
  assert.equal(calls.length, 1);
  assert.equal(calls[0]?.input, url);
  assert.equal(
    new Headers(calls[0]?.init?.headers).get("authorization"),
    "Bearer secret-token",
  );
  assert.equal(String(calls[0]?.input).includes("secret-token"), false);
});

test("authenticated avatar fetch refuses cross-origin URLs before calling fetch", async () => {
  let called = false;
  const fetchImage: typeof fetch = async () => {
    called = true;
    return new Response();
  };

  await assert.rejects(() =>
    fetchProtectedMediaServerAvatar(
      "https://emby.example.test/avatar.png",
      "https://scryer.example.test",
      "secret-token",
      new AbortController().signal,
      fetchImage,
    ),
  );
  assert.equal(called, false);
});

test("authenticated avatar fetch rejects a successful non-image response", async () => {
  const fetchImage: typeof fetch = async () =>
    new Response("not an image", {
      headers: { "Content-Type": "text/plain" },
    });

  await assert.rejects(
    () =>
      fetchProtectedMediaServerAvatar(
        "/api/media-server-avatars/connection/user/tag",
        "https://scryer.example.test",
        "secret-token",
        new AbortController().signal,
        fetchImage,
      ),
    /not an image/,
  );
});
