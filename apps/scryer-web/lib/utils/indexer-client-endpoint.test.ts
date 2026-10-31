import assert from "node:assert/strict";
import test from "node:test";
import { indexerClientEndpointUrl } from "./indexer-client-endpoint.ts";

test("endpoint uses the public URL and base path, falling back to the browser origin", () => {
  const origin = "http://localhost:3000";
  assert.equal(
    indexerClientEndpointUrl("NEWZNAB", null, "/", origin),
    "http://localhost:3000/newznab",
  );
  assert.equal(
    indexerClientEndpointUrl("TORZNAB", null, "/scryer/", origin),
    "http://localhost:3000/scryer/torznab",
  );
  assert.equal(
    indexerClientEndpointUrl(
      "NEWZNAB",
      "https://media.example.test",
      "/scryer",
      origin,
    ),
    "https://media.example.test/scryer/newznab",
  );
  assert.equal(
    indexerClientEndpointUrl(
      "NEWZNAB",
      "https://media.example.test/scryer/",
      "/scryer",
      origin,
    ),
    "https://media.example.test/scryer/newznab",
  );
  assert.equal(
    indexerClientEndpointUrl(
      "TORZNAB",
      "https://media.example.test/",
      "/",
      origin,
    ),
    "https://media.example.test/torznab",
  );
  assert.equal(
    indexerClientEndpointUrl("NEWZNAB", "not a url", "/", origin),
    "http://localhost:3000/newznab",
  );
});
