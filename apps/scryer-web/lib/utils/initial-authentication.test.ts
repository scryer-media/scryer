import test from "node:test";
import assert from "node:assert/strict";
import {
  initialAuthenticationView,
  initialAuthenticationReloadPath,
  canCompleteInitialAuthentication,
} from "./initial-authentication.ts";

test("initial authentication never treats missing or unknown state as completion", () => {
  for (const value of [undefined, "", "future-stage", false, {}])
    assert.equal(initialAuthenticationView(value), "error");
  for (const value of [null, "complete"])
    assert.equal(initialAuthenticationView(value), "application");
  assert.equal(initialAuthenticationView("choice"), "choice");
  assert.equal(
    initialAuthenticationView("password_replacement"),
    "password_replacement",
  );
});

test("local completion refreshes the session without a document reload", () => {
  assert.equal(
    initialAuthenticationReloadPath("choice", "application", true),
    null,
  );
  assert.equal(
    initialAuthenticationReloadPath("choice", "application", false),
    "/setup",
  );
});

test("setup transitions rebootstrap before exposing cached authentication", () => {
  for (const completed of ["complete", null]) {
    for (const pending of ["choice", "password_replacement"] as const) {
      assert.equal(
        initialAuthenticationReloadPath(
          initialAuthenticationView(pending),
          initialAuthenticationView(completed),
        ),
        "/setup",
      );
    }
  }
  assert.equal(
    initialAuthenticationReloadPath("choice", "password_replacement"),
    "/login?redirect=%2Fsetup",
  );
  assert.equal(
    initialAuthenticationReloadPath("application", "choice"),
    "/setup",
  );
});

test("initial reads, failed refreshes, and unchanged setup need no navigation", () => {
  for (const stage of [
    "error",
    "choice",
    "password_replacement",
    "application",
  ] as const) {
    assert.equal(initialAuthenticationReloadPath("error", stage), null);
    assert.equal(initialAuthenticationReloadPath(stage, "error"), null);
    assert.equal(initialAuthenticationReloadPath(stage, stage), null);
  }
  // A failed refresh must not erase the previous pending observation: a
  // successful retry still needs to discard the cached signed-out snapshot.
  assert.equal(initialAuthenticationReloadPath("choice", "error"), null);
  assert.equal(
    initialAuthenticationReloadPath("choice", "application"),
    "/setup",
  );
});

test("initial authentication requires an explicit matching choice and operator proof", () => {
  const input = {
    mode: "PASSWORD" as const,
    password: "long-password",
    confirmation: "long-password",
    minimum: 12,
    acknowledged: false,
    codeRequired: true,
    code: "ABC-234",
    environmentMode: null,
  };
  assert.equal(canCompleteInitialAuthentication(input), true);
  for (const change of [
    { mode: null },
    { confirmation: "different" },
    { minimum: 24 },
    { code: " " },
    { code: "ACD" },
    { code: "AC D34" },
    { code: "ACD3479" },
    { environmentMode: false },
  ]) {
    assert.equal(
      canCompleteInitialAuthentication({ ...input, ...change }),
      false,
    );
  }
  assert.equal(
    canCompleteInitialAuthentication({ ...input, mode: "NO_AUTHENTICATION" }),
    false,
  );
  assert.equal(
    canCompleteInitialAuthentication({
      ...input,
      mode: "NO_AUTHENTICATION",
      acknowledged: true,
    }),
    true,
  );
  assert.equal(
    canCompleteInitialAuthentication({
      ...input,
      mode: "NO_AUTHENTICATION",
      acknowledged: true,
      environmentMode: true,
    }),
    false,
  );
  assert.equal(
    canCompleteInitialAuthentication({
      ...input,
      code: "",
      codeRequired: false,
    }),
    true,
  );
});
