import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  DIAGNOSTICS_REFUSAL_KEYS,
  DiagnosticsBundleError,
  diagnosticsBundleErrorFromResponse,
  diagnosticsBundleErrorMessage,
} from "./diagnostics-bundle-error.ts";

const dictionary = en as Record<string, string>;
const t = (key: string) => dictionary[key] ?? key;

test("every diagnostics reason has English copy", () => {
  for (const key of Object.values(DIAGNOSTICS_REFUSAL_KEYS)) {
    assert.ok(dictionary[key], `${key} is missing from en.ts`);
  }
});

test("a refusal is explained from its reason, not its sentence", async () => {
  const response = new Response(
    JSON.stringify({
      error: "some other wording",
      reason: "HTTP_DIAGNOSTICS_PERMISSION_DENIED",
    }),
    { status: 403, headers: { "content-type": "application/json" } },
  );
  const error = await diagnosticsBundleErrorFromResponse(response);

  assert.equal(error.reason, "HTTP_DIAGNOSTICS_PERMISSION_DENIED");
  assert.equal(
    diagnosticsBundleErrorMessage(error, t, "Failed"),
    dictionary["system.diagnosticsBundle.refusal.permissionDenied"],
  );
});

test("an unknown reason shows the server's sentence", () => {
  const error = new DiagnosticsBundleError(
    "synthetic sentence",
    "HTTP_DIAGNOSTICS_SYNTHETIC_UNKNOWN",
  );

  assert.equal(
    diagnosticsBundleErrorMessage(error, t, "Failed"),
    "synthetic sentence",
  );
});

test("a failure with neither sentence nor known reason shows the fallback", async () => {
  const response = new Response("", { status: 502 });
  const error = await diagnosticsBundleErrorFromResponse(response);

  assert.equal(diagnosticsBundleErrorMessage(error, t, "Failed"), "Failed");
  assert.equal(
    diagnosticsBundleErrorMessage(new Error("network"), t, "Failed"),
    "Failed",
  );
});
