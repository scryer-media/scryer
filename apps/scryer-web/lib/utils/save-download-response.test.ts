import assert from "node:assert/strict";
import test from "node:test";

import { filenameFromContentDisposition } from "./save-download-response.ts";

const FALLBACK = "scryer-release.nzb";

test("the attachment filename comes from the header, preferring filename*", () => {
  assert.equal(
    filenameFromContentDisposition(
      `attachment; filename="Paperman.2012.1080p.WEB-DL.nzb"`,
      FALLBACK,
    ),
    "Paperman.2012.1080p.WEB-DL.nzb",
  );

  // The ASCII fallback is lossy; the encoded form carries the real name.
  assert.equal(
    filenameFromContentDisposition(
      `attachment; filename="__.nzb"; filename*=UTF-8''%E6%9D%B1%E4%BA%AC%2Enzb`,
      FALLBACK,
    ),
    "東京.nzb",
  );

  assert.equal(
    filenameFromContentDisposition(
      String.raw`attachment; filename="Say \"Hi\".nzb"`,
      FALLBACK,
    ),
    `Say "Hi".nzb`,
  );

  assert.equal(
    filenameFromContentDisposition("attachment; filename=bundle.tar.gz", FALLBACK),
    "bundle.tar.gz",
  );
});

test("a missing or unusable disposition header falls back", () => {
  assert.equal(filenameFromContentDisposition(null, FALLBACK), FALLBACK);
  assert.equal(filenameFromContentDisposition("attachment", FALLBACK), FALLBACK);
  assert.equal(
    filenameFromContentDisposition(`attachment; filename=""`, FALLBACK),
    FALLBACK,
  );
  // A truncated percent escape must not throw the whole download away.
  assert.equal(
    filenameFromContentDisposition(
      `attachment; filename="ok.nzb"; filename*=UTF-8''%E6%9D`,
      FALLBACK,
    ),
    "ok.nzb",
  );
});
