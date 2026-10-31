import { PUBLIC_ERROR_REASON_CODES } from "./public-error-reason-codes.ts";
import { t, loadLocaleDictionary } from "../i18n/index.ts";
import test from "node:test";
import assert from "node:assert/strict";

import {
  ERROR_REASON_KEYS,
  isLostWarmupSessionError,
  hasGraphQlErrorReason,
  graphQlErrorNumberExtension,
  isMfaStepUpRequiredError,
  normalizeGraphQlErrorMessage,
  userFacingGraphQlErrorMessage,
} from "./error-message.ts";

function coded(
  reason: string,
  code = "VALIDATION_ERROR",
  message = "This sentence was reworded",
) {
  return { graphQLErrors: [{ message, extensions: { code, reason } }] };
}

test("metadata gateway clock skew uses its translated reason, not server text", async () => {
  const error = coded("METADATA_GATEWAY_CLOCK_SKEW");
  assert.equal(
    userFacingGraphQlErrorMessage(error, "Failed"),
    "Scryer cannot connect to the metadata gateway because your system clock is off by more than 5 minutes. Synchronize your system clock and restart Scryer.",
  );
  for (const language of [
    "fra",
    "deu",
    "spa",
    "ita",
    "por",
    "kor",
    "zho",
    "zh-HK",
    "zh-TW",
    "jpn",
    "rus",
    "nld",
    "ces",
  ]) {
    await loadLocaleDictionary(language);
    const translated = userFacingGraphQlErrorMessage(
      error,
      "Failed",
      (key, values) => t(key, language, values),
    );
    assert.notEqual(translated, "This sentence was reworded");
    assert.notEqual(translated, userFacingGraphQlErrorMessage(error, "Failed"));
    assert.ok(translated.includes("Scryer"));
  }
  assert.equal(
    hasGraphQlErrorReason(
      error,
      "METADATA_GATEWAY_CLOCK_SKEW",
      "VALIDATION_ERROR",
    ),
    true,
  );
  assert.equal(
    hasGraphQlErrorReason(
      coded("OTHER_AUTH_FAILURE"),
      "METADATA_GATEWAY_CLOCK_SKEW",
      "VALIDATION_ERROR",
    ),
    false,
  );
});

test("initial authentication refusals use localized codes rather than server wording", async () => {
  await loadLocaleDictionary("fr");
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("INITIAL_AUTH_CODE_INVALID"),
      "Failed",
      (key, values) => t(key, "fr", values),
    ),
    "Saisissez le code de configuration affiché dans la console du serveur ou, sur l’application de bureau, dans logs/console.log du dossier de données de Scryer.",
  );
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("INITIAL_AUTH_ALREADY_COMPLETED"),
      "Failed",
    ),
    "Authentication setup has already been completed. Refresh to continue.",
  );
});

test("password refusals interpolate validated minima and localize missing or malformed facts", async () => {
  await loadLocaleDictionary("fr");
  for (const minLength of [8, 24, "16"]) {
    const error = {
      graphQLErrors: [
        {
          message: "server requirement",
          extensions: {
            code: "VALIDATION_ERROR",
            reason: "SECURITY_PASSWORD_OUT_OF_RANGE",
            minLength,
          },
        },
      ],
    };
    assert.equal(
      userFacingGraphQlErrorMessage(error, "Failed"),
      `Password must be at least ${minLength} characters.`,
    );
    assert.equal(
      userFacingGraphQlErrorMessage(
        error,
        "Failed",
        (_key, facts) => `Mindestens ${facts?.minLength} Zeichen`,
      ),
      `Mindestens ${minLength} Zeichen`,
    );
  }
  for (const minLength of [
    undefined,
    null,
    true,
    "",
    "eight",
    0,
    -1,
    1.5,
    Infinity,
    "8e2",
  ]) {
    const error = {
      graphQLErrors: [
        {
          message: "Password must have the configured minimum",
          extensions: {
            code: "VALIDATION_ERROR",
            reason: "SECURITY_PASSWORD_OUT_OF_RANGE",
            minLength,
          },
        },
      ],
    };
    assert.equal(
      userFacingGraphQlErrorMessage(error, "Failed"),
      "Password does not meet the configured length requirement.",
    );
    assert.equal(
      userFacingGraphQlErrorMessage(error, "Failed", (key, values) =>
        t(key, "fr", values),
      ),
      "Le mot de passe ne respecte pas la longueur requise par la configuration.",
    );
  }
});

test("missing titles use their reason even after the sentence changes", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("TITLE_NOT_FOUND", "NOT_FOUND"),
      "Failed",
    ),
    "Title not found.",
  );
  assert.equal(
    userFacingGraphQlErrorMessage(
      new Error("not found: title missing-id"),
      "Failed",
    ),
    "not found: title missing-id",
  );
});

test("normalizeGraphQlErrorMessage strips GraphQL validation prefixes", () => {
  assert.equal(
    normalizeGraphQlErrorMessage(
      "[GraphQL] validation: no download client enabled for library movie_default_library",
    ),
    "no download client enabled for library movie_default_library",
  );
});

test("uncoded prose is display cleanup only", () => {
  assert.equal(
    normalizeGraphQlErrorMessage("[GraphQL] repository: validation: old words"),
    "repository: validation: old words",
  );
});

test("normalizeGraphQlErrorMessage strips plain GraphQL prefixes", () => {
  assert.equal(
    normalizeGraphQlErrorMessage(
      "[GraphQL] enable TOTP for your account before requiring TOTP for system configuration",
    ),
    "enable TOTP for your account before requiring TOTP for system configuration",
  );
});

test("expired release candidates use a reason rather than JWT diagnostic text", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("RELEASE_TOKEN_EXPIRED", "UNAUTHORIZED"),
      "queue failed",
    ),
    "Search results expired, please search again",
  );
});

test("other token errors are not described as expired search results", () => {
  for (const message of [
    "unauthorized: invalid release candidate token: InvalidSignature",
    "unauthorized: invalid token: ExpiredSignature",
  ]) {
    assert.equal(normalizeGraphQlErrorMessage(message), message);
  }
});

test("configuration verification uses the existing MFA code", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(coded("", "MFA_STEP_UP_REQUIRED"), "Failed"),
    "Settings verification expired. Enter an authenticator code to continue.",
  );
});

test("known reasons use the current translator and unknown reasons keep their sentence", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("TAG_ALREADY_EXISTS"),
      "Failed",
      (key) => `translated:${key}`,
    ),
    "translated:error.tagAlreadyExists",
  );
  assert.equal(
    userFacingGraphQlErrorMessage(coded("FUTURE_UNKNOWN"), "Failed"),
    "This sentence was reworded",
  );
});

test("network failures are identified by the network field", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(
      { networkError: new Error("different browser wording") },
      "Failed",
    ),
    "Disconnected",
  );
  assert.equal(
    userFacingGraphQlErrorMessage(
      new Error("[Network] Failed to fetch"),
      "Failed",
    ),
    "[Network] Failed to fetch",
  );
});

test("isMfaStepUpRequiredError detects GraphQL extension codes", () => {
  assert.equal(
    isMfaStepUpRequiredError({
      graphQLErrors: [
        {
          message:
            "MFA verification is required before changing system configuration",
          extensions: { code: "MFA_STEP_UP_REQUIRED" },
        },
      ],
    }),
    true,
  );
});

test("userFacingGraphQlErrorMessage passes plain Error.message through", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(new Error("queue failed"), "fallback"),
    "queue failed",
  );
});

test("userFacingGraphQlErrorMessage normalizes prefixed Error.message values", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(
      new Error(
        "[GraphQL] enable TOTP for your account before requiring TOTP for system configuration",
      ),
      "fallback",
    ),
    "enable TOTP for your account before requiring TOTP for system configuration",
  );
});

test("userFacingGraphQlErrorMessage uses fallback when no message exists", () => {
  assert.equal(
    userFacingGraphQlErrorMessage({ graphQLErrors: [] }, "queue failed"),
    "queue failed",
  );
});

test("queue validation failure normalizes into the global status text", () => {
  const message = userFacingGraphQlErrorMessage(
    {
      graphQLErrors: [
        {
          message:
            "[GraphQL] validation: no download client enabled for library movie_default_library",
        },
      ],
    },
    "queue failed",
  );

  assert.equal(
    message,
    "no download client enabled for library movie_default_library",
  );
});

test("GraphQL internal errors mask repository details and include the reference id", () => {
  const message = userFacingGraphQlErrorMessage(
    {
      graphQLErrors: [
        {
          message:
            "[GraphQL] repository: metadata gateway request failed (502 Bad Gateway): <!DOCTYPE html><html><body>Bad gateway</body></html>",
          extensions: { code: "INTERNAL_ERROR", errorId: "err-123" },
        },
      ],
    },
    "metadata search failed",
  );

  assert.equal(message, "Internal server error. Reference ID: err-123");
});

test("every known public reason has a real base localization entry", () => {
  for (const [reason, key] of Object.entries(ERROR_REASON_KEYS)) {
    assert.notEqual(t(key, "eng"), key, `${reason} has no base copy`);
    assert.ok(
      PUBLIC_ERROR_REASON_CODES.get(reason)?.length,
      `${reason} has no coarse-code contract`,
    );
  }
});

test("reworded conditions drive scan, import, tag, and rule decisions", () => {
  for (const reason of [
    "LIBRARY_SCAN_ALREADY_RUNNING",
    "EXTERNAL_IMPORT_SESSION_GONE",
    "TAG_ALREADY_EXISTS",
    "RULE_PACK_UP_TO_DATE",
    "RULE_TARGETS_PERSON",
  ]) {
    const code =
      reason === "EXTERNAL_IMPORT_SESSION_GONE"
        ? "NOT_FOUND"
        : "VALIDATION_ERROR";
    assert.equal(
      hasGraphQlErrorReason(coded(reason, code), reason, code),
      true,
    );
    assert.equal(
      hasGraphQlErrorReason(coded(reason, "PLUGIN_FAILED"), reason, code),
      false,
    );
    assert.equal(hasGraphQlErrorReason(new Error(reason), reason, code), false);
  }
  assert.equal(
    isLostWarmupSessionError(
      coded("EXTERNAL_IMPORT_SESSION_GONE", "NOT_FOUND"),
    ),
    true,
  );
  assert.equal(isLostWarmupSessionError(new Error("no warmup session")), false);
});

test("public account-link failures never recover private relationship detail", () => {
  const error = coded(
    "EXTERNAL_ACCOUNT_LINK_FAILED",
    "REQUEST_CONFLICT",
    "Jellyfin account could not be linked",
  );
  assert.equal(
    userFacingGraphQlErrorMessage(error, "Failed"),
    "The external account could not be linked.",
  );
});

test("a reason never changes its broad error family", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("TAG_ALREADY_EXISTS", "REPOSITORY_ERROR"),
      "Failed",
    ),
    "This sentence was reworded",
  );
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("PLUGIN_AUTH_FAILED", "VALIDATION_ERROR"),
      "Failed",
    ),
    "This sentence was reworded",
  );
  assert.notEqual(
    userFacingGraphQlErrorMessage(
      coded("PLUGIN_AUTH_FAILED", "PLUGIN_FAILED"),
      "Failed",
    ),
    "This sentence was reworded",
  );
});

test("behavior helpers require the expected broad code and reason together", () => {
  assert.equal(
    isLostWarmupSessionError(
      coded("EXTERNAL_IMPORT_SESSION_GONE", "PLUGIN_FAILED"),
    ),
    false,
  );
  assert.equal(
    isLostWarmupSessionError(
      coded("EXTERNAL_IMPORT_SESSION_GONE", "VALIDATION_ERROR"),
    ),
    false,
  );
  const countError = (code: string) => ({
    graphQLErrors: [
      {
        message: "arbitrary",
        extensions: {
          code,
          reason: "MAINTENANCE_ARM_COUNT_MISMATCH",
          currentCount: 12,
        },
      },
    ],
  });
  assert.equal(
    graphQlErrorNumberExtension(
      countError("PLUGIN_FAILED"),
      "VALIDATION_ERROR",
      "MAINTENANCE_ARM_COUNT_MISMATCH",
      "currentCount",
    ),
    null,
  );
  assert.equal(
    graphQlErrorNumberExtension(
      countError("VALIDATION_ERROR"),
      "VALIDATION_ERROR",
      "MAINTENANCE_ARM_COUNT_MISMATCH",
      "currentCount",
    ),
    12,
  );
  for (const code of ["VALIDATION_ERROR", "PLUGIN_FAILED", "UNKNOWN_FAMILY"])
    assert.equal(
      userFacingGraphQlErrorMessage(coded("TITLE_NOT_FOUND", code), "Failed"),
      "This sentence was reworded",
    );
  assert.equal(
    userFacingGraphQlErrorMessage(
      coded("RELEASE_TOKEN_EXPIRED", "NOT_FOUND"),
      "Failed",
    ),
    "This sentence was reworded",
  );
});
