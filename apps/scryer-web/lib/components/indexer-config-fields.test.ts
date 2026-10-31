import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentType, type Context } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router";
import { createServer } from "vite";

import type { Translate } from "@/components/root/types";
import type {
  ConfigFieldDef,
  IndexerDraft,
  LoginChallengeStatus,
} from "@/lib/types";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));
const translate: Translate = (key) => key;

function field(overrides: Partial<ConfigFieldDef>): ConfigFieldDef {
  return {
    key: "field",
    label: "Field",
    fieldType: "STRING",
    required: false,
    defaultValue: null,
    valueSource: "USER",
    role: null,
    hostBinding: null,
    options: [],
    helpText: null,
    visibleWhen: null,
    requiredWhen: null,
    advanced: false,
    ...overrides,
  };
}

const draft: IndexerDraft = {
  name: "Sample Tracker",
  providerType: "cardigann",
  proxyConfigId: null,
  downloadClientId: null,
  seedingProfileId: null,
  storedSecretKeys: [],
  rateLimitSeconds: "",
  maxQueriesPerMinute: "",
  isEnabled: true,
  enableInteractiveSearch: true,
  enableAutoSearch: true,
  configValues: { username: "sample-user" },
};

const fields: ConfigFieldDef[] = [
  field({ key: "username", label: "Username" }),
  field({
    key: "info_cookie",
    label: "About cookies",
    fieldType: "INFO",
    helpText: "Sign in with a browser first.\n- Copy the cookie",
  }),
  field({
    key: "cardigannCaptcha",
    label: "CAPTCHA",
    role: "LOGIN_CHALLENGE_ANSWER",
  }),
];

async function renderFieldGroup(loginChallenge?: LoginChallengeStatus) {
  const server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });
  try {
    const [sectionModule, contextModule] = await Promise.all([
      server.ssrLoadModule(
        "/components/views/settings/settings-indexers-section.tsx",
      ),
      server.ssrLoadModule("/lib/context/translate-context.tsx"),
    ]);
    const Group = sectionModule.ProviderConfigFieldGroup as ComponentType<
      Record<string, unknown>
    >;
    const TranslateContext =
      contextModule.TranslateContext as Context<Translate | null>;
    return renderToStaticMarkup(
      createElement(
        MemoryRouter,
        null,
        createElement(
          TranslateContext.Provider,
          { value: translate },
          createElement(Group, {
            fields,
            draft,
            onChange: () => {},
            loginChallenge,
            onRefreshLoginChallenge: () => {},
          }),
        ),
      ),
    );
  } finally {
    await server.close();
  }
}

test("an informational field is shown as text and never as an input", async () => {
  const html = await renderFieldGroup();

  const note = html.match(
    /<div id="settings-indexer-field-info-cookie" role="note"[^>]*>(.*?)<\/div>/s,
  );
  assert.ok(note, "the note renders");
  assert.match(note[1], /About cookies/);
  assert.match(note[1], /Sign in with a browser first\./);
  assert.doesNotMatch(note[1], /<input|<textarea|<select/);
  assert.doesNotMatch(
    html,
    /name="info_cookie"|<input[^>]*id="settings-indexer-field-info-cookie"/,
  );
});

test("the sign-in image is shown beside the field that takes its answer", async () => {
  const html = await renderFieldGroup({
    state: "ready",
    contentType: "image/png",
    imageData: "iVBORw==",
  });

  const answerInput = html.indexOf(
    'id="settings-indexer-field-cardiganncaptcha"',
  );
  const image = html.indexOf('id="settings-indexer-login-challenge-image"');
  assert.ok(answerInput >= 0, "the answer field renders");
  assert.ok(image > answerInput, "the image follows the answer field");
  assert.match(html, /src="data:image\/png;base64,iVBORw=="/);
  assert.match(html, /alt="settings\.indexerLoginChallengeImageAlt"/);
  assert.match(html, /id="settings-indexer-login-challenge-refresh"/);
});

test("a sign-in image that could not be fetched says why and offers another", async () => {
  const html = await renderFieldGroup({
    state: "error",
    message: "The sign-in image is too large to show.",
  });

  assert.match(
    html,
    /id="settings-indexer-login-challenge-error" role="alert"/,
  );
  assert.match(html, /settings\.indexerLoginChallengeError/);
  assert.match(html, /id="settings-indexer-login-challenge-refresh"/);
  assert.doesNotMatch(html, /settings-indexer-login-challenge-image/);
});

test("without a sign-in image nothing is added beside the answer field", async () => {
  const html = await renderFieldGroup({ state: "none" });

  assert.match(html, /id="settings-indexer-field-cardiganncaptcha"/);
  assert.doesNotMatch(html, /settings-indexer-login-challenge-/);
});
