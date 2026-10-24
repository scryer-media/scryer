import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import en from "../i18n/locales/en.ts";
import {
  PLUGIN_FAILURE_KEYS,
  humanisePluginReason,
  pluginErrorMessage,
  pluginFailureDetail,
  pluginFailureKey,
  unknownPluginFailureReason,
} from "./plugin-error-message.ts";

const dictionary = en as Record<string, string>;
const t = (key: string, values?: Record<string, unknown>) => {
  let result = dictionary[key] ?? key;
  for (const [name, value] of Object.entries(values ?? {})) {
    result = result.replaceAll(`{{${name}}}`, String(value ?? ""));
  }
  return result;
};

function failure(message: string, extensions: Record<string, unknown>) {
  return { graphQLErrors: [{ message, extensions }] };
}

test("a named plugin failure is shown in translation, not as the server's sentence", () => {
  const error = failure(
    "indexer connection test failed: indexer indexer_search component: plugin error Temporary: indexer component failed",
    { code: "PLUGIN_FAILED", reason: "PLUGIN_TEMPORARY_FAILURE" },
  );

  assert.equal(pluginFailureKey(error), "plugins.failure.temporary");
  assert.equal(pluginFailureDetail(error), null);
  assert.equal(
    pluginErrorMessage(error, t, "Test failed"),
    "The plugin ran into a temporary problem. Try again in a little while.",
  );
});

test("the plugin's own sentence follows the translated reason", () => {
  const error = failure("download test_connection: plugin error AuthFailed: Login refused", {
    code: "PLUGIN_FAILED",
    reason: "PLUGIN_AUTH_FAILED",
    detail: "Login refused",
  });

  assert.equal(pluginFailureDetail(error), "Login refused");
  assert.equal(
    pluginErrorMessage(error, t, "Test failed"),
    "The service rejected the sign-in details. Check the API key, username, or password. Details: Login refused",
  );
});

test("a reason this client does not know shows the plugin's own sentence", () => {
  const error = failure("plugin error quota_exhausted: the dial is set to eleven", {
    code: "PLUGIN_FAILED",
    reason: "PLUGIN_QUOTA_EXHAUSTED",
    detail: "the dial is set to eleven",
  });

  assert.equal(pluginFailureKey(error), null);
  assert.equal(unknownPluginFailureReason(error), "PLUGIN_QUOTA_EXHAUSTED");
  assert.equal(pluginErrorMessage(error, t, "Test failed"), "the dial is set to eleven");
});

test("a reason this client does not know, with no sentence, names the code in translation", () => {
  const error = failure("plugin error quota_exhausted: indexer component failed", {
    code: "PLUGIN_FAILED",
    reason: "PLUGIN_QUOTA_EXHAUSTED",
  });

  assert.equal(
    pluginErrorMessage(error, t, "Test failed"),
    "The plugin reported a problem: quota exhausted.",
  );
});

test("a new plugin reason translates once it is registered, with no other change", () => {
  const reason = "PLUGIN_INVALID_USERNAME_PASSWORD";
  const error = failure("plugin error invalid_username_password: Login refused", {
    code: "PLUGIN_FAILED",
    reason,
    detail: "Login refused",
  });
  assert.equal(pluginFailureKey(error), "plugins.failure.invalidUsernamePassword");
  assert.equal(unknownPluginFailureReason(error), null);
  assert.equal(
    pluginErrorMessage(error, t, "Test failed"),
    "The service rejected the username or password. Check both and try again. Details: Login refused",
  );

  // The same failure before the client knew the reason: the plugin's sentence.
  const unregistered = failure("plugin error invalid_username_password: Login refused", {
    code: "PLUGIN_FAILED",
    reason: "PLUGIN_INVALID_USERNAME_PASSWORDS",
    detail: "Login refused",
  });
  assert.equal(pluginErrorMessage(unregistered, t, "Test failed"), "Login refused");
});

test("a reason without the plugin prefix is not read as a plugin reason", () => {
  for (const reason of ["SOMETHING_ELSE", "PLUGIN_"]) {
    const error = failure("plugin error: boom", { code: "PLUGIN_FAILED", reason, detail: "boom" });
    assert.equal(unknownPluginFailureReason(error), null, reason);
    assert.equal(pluginErrorMessage(error, t, "Test failed"), "plugin error: boom", reason);
  }
});

test("a plugin reason reads as words", () => {
  assert.equal(humanisePluginReason("PLUGIN_INVALID_USERNAME_PASSWORD"), "invalid username password");
  assert.equal(humanisePluginReason("PLUGIN_HTTP_429"), "http 429");
});

test("a reason on another kind of error is not read as a plugin failure", () => {
  const error = failure("validation: The indexer rejected the API key.", {
    code: "VALIDATION_ERROR",
    reason: "PLUGIN_AUTH_FAILED",
  });

  assert.equal(pluginFailureKey(error), null);
  assert.equal(pluginErrorMessage(error, t, "Test failed"), "The indexer rejected the API key.");
});

test("a failure with no sentence at all shows the fallback", () => {
  assert.equal(pluginErrorMessage(new Error(""), t, "Test failed"), "Test failed");
});

test("every reason the server can name has a translation in every locale", () => {
  const reasons = [
    "PLUGIN_INVALID_CONFIG",
    "PLUGIN_AUTH_FAILED",
    "PLUGIN_RATE_LIMITED",
    "PLUGIN_UPSTREAM_UNAVAILABLE",
    "PLUGIN_UNSUPPORTED",
    "PLUGIN_TEMPORARY_FAILURE",
    "PLUGIN_PERMANENT_FAILURE",
    "PLUGIN_HOST_FAILURE",
    "PLUGIN_TIMED_OUT",
    "PLUGIN_RESOURCE_LIMIT",
    "PLUGIN_INVALID_USERNAME_PASSWORD",
    "PLUGIN_INVALID_API_KEY",
    "PLUGIN_INVALID_PASSKEY",
    "PLUGIN_INVALID_COOKIE",
    "PLUGIN_CAPTCHA_REQUIRED",
    "PLUGIN_TWO_FACTOR_REQUIRED",
    "PLUGIN_ACCOUNT_BANNED",
    "PLUGIN_ACCOUNT_DISABLED",
    "PLUGIN_ENDPOINT_NOT_FOUND",
    "PLUGIN_TLS_ERROR",
    "PLUGIN_INVALID_RESPONSE",
    "PLUGIN_QUOTA_EXCEEDED",
    "PLUGIN_ITEM_NOT_FOUND",
    "PLUGIN_DOWNLOAD_REJECTED",
    "PLUGIN_PATH_NOT_WRITABLE",
  ];
  assert.deepEqual(Object.keys(PLUGIN_FAILURE_KEYS).sort(), [...reasons].sort());

  const localesDir = path.join(path.dirname(fileURLToPath(import.meta.url)), "../i18n/locales");
  const keys = [
    ...Object.values(PLUGIN_FAILURE_KEYS),
    "plugins.failure.withDetail",
    "plugins.failure.unknown",
  ];
  for (const file of readdirSync(localesDir).filter((name) => name.endsWith(".ts"))) {
    const source = readFileSync(path.join(localesDir, file), "utf8");
    for (const key of keys) {
      assert.ok(source.includes(`"${key}":`), `${file} is missing ${key}`);
    }
    const withDetail = source.match(/"plugins\.failure\.withDetail":\s*"([^"]*)"/)?.[1] ?? "";
    assert.ok(
      withDetail.includes("{{reason}}") && withDetail.includes("{{detail}}"),
      `${file} withDetail must keep both placeholders`,
    );
    const unknown = source.match(/"plugins\.failure\.unknown":\s*"([^"]*)"/)?.[1] ?? "";
    assert.ok(unknown.includes("{{code}}"), `${file} unknown must keep its placeholder`);
  }
});

test("every plugin reason resolves to a sentence of its own in every locale", async () => {
  const localesDir = path.join(path.dirname(fileURLToPath(import.meta.url)), "../i18n/locales");
  const files = readdirSync(localesDir).filter((name) => name.endsWith(".ts"));
  assert.equal(files.length, 13);
  for (const file of files) {
    const locale = (await import(path.join(localesDir, file))).default as Record<string, string>;
    const sentences = new Set<string>();
    for (const [reason, key] of Object.entries(PLUGIN_FAILURE_KEYS)) {
      const sentence = locale[key];
      assert.equal(typeof sentence, "string", `${file} has no ${key} for ${reason}`);
      assert.ok(sentence.trim().length > 0, `${file} ${key} is empty`);
      assert.ok(!sentences.has(sentence), `${file} ${key} repeats another reason's sentence`);
      sentences.add(sentence);
    }
  }
});

test("each shared plugin code is shown in translation, with the plugin's sentence after it", () => {
  const error = failure("plugin error invalid_api_key: Key refused", {
    code: "PLUGIN_FAILED",
    reason: "PLUGIN_INVALID_API_KEY",
    detail: "Key refused",
  });
  assert.equal(pluginFailureKey(error), "plugins.failure.invalidApiKey");
  assert.equal(unknownPluginFailureReason(error), null);
  assert.equal(
    pluginErrorMessage(error, t, "Test failed"),
    "The service rejected the API key. Check the key and try again. Details: Key refused",
  );
});
