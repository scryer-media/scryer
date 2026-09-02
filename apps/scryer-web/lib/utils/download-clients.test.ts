import assert from "node:assert/strict";
import test from "node:test";

import {
  buildDownloadClientConfigValues,
  isFileBackedDownloadClientConfigField,
} from "./download-clients.ts";
import type { ConfigFieldDef, DownloadClientDraft } from "../types/index.ts";

function configField(
  overrides: Partial<ConfigFieldDef> & Pick<ConfigFieldDef, "key">,
): ConfigFieldDef {
  return {
    key: overrides.key,
    label: overrides.label ?? overrides.key,
    fieldType: overrides.fieldType ?? "STRING",
    required: overrides.required ?? false,
    defaultValue: overrides.defaultValue ?? null,
    valueSource: overrides.valueSource ?? "USER",
    role: overrides.role ?? null,
    hostBinding: overrides.hostBinding ?? null,
    options: overrides.options ?? [],
    helpText: overrides.helpText ?? null,
  };
}

function draft(overrides: Partial<DownloadClientDraft>): DownloadClientDraft {
  return {
    name: "Test client",
    clientType: "nzbget",
    host: "download-client",
    port: "6789",
    urlBase: "",
    useSsl: false,
    apiKey: "",
    username: "",
    password: "",
    remotePathMappings: "",
    configValues: {},
    proxyConfigId: null,
    isEnabled: true,
    ...overrides,
  };
}

function valuesByKey(
  values: ReturnType<typeof buildDownloadClientConfigValues>,
) {
  return new Map(values.map((value) => [value.key, value]));
}

test("descriptor credentials are not overwritten by blank fixed fields", () => {
  const values = valuesByKey(
    buildDownloadClientConfigValues(
      draft({
        clientType: "qbittorrent",
        username: "",
        password: "",
        configValues: {
          username: "qbit-user",
          password: "qbit-pass",
        },
      }),
      [
        configField({ key: "username" }),
        configField({ key: "password", fieldType: "PASSWORD" }),
      ],
    ),
  );

  assert.equal(values.get("username")?.secretValue, "qbit-user");
  assert.equal(values.get("password")?.secretValue, "qbit-pass");
});

test("fixed credentials serialize when descriptors do not own those keys", () => {
  const values = valuesByKey(
    buildDownloadClientConfigValues(
      draft({
        username: "nzbget-user",
        password: "nzbget-pass",
      }),
    ),
  );

  assert.equal(values.get("username")?.secretValue, "nzbget-user");
  assert.equal(values.get("password")?.secretValue, "nzbget-pass");
});

test("descriptor api key is not overwritten by blank fixed api key", () => {
  const values = valuesByKey(
    buildDownloadClientConfigValues(
      draft({
        clientType: "sabnzbd",
        apiKey: "",
        configValues: {
          api_key: "descriptor-api-key",
        },
      }),
      [configField({ key: "api_key", fieldType: "PASSWORD" })],
    ),
  );

  assert.equal(values.get("api_key")?.secretValue, "descriptor-api-key");
});

test("blank optional descriptor secrets are omitted", () => {
  const values = valuesByKey(
    buildDownloadClientConfigValues(
      draft({
        clientType: "qbittorrent",
        configValues: {
          password: "",
        },
      }),
      [configField({ key: "password", fieldType: "PASSWORD" })],
    ),
  );

  assert.equal(values.has("password"), false);
});

test("stored descriptor secrets can be explicitly cleared", () => {
  const values = valuesByKey(
    buildDownloadClientConfigValues(
      draft({
        clientType: "qbittorrent",
        configValues: { api_key: "" },
      }),
      [configField({ key: "api_key", fieldType: "PASSWORD" })],
      new Set(["api_key"]),
    ),
  );

  assert.equal(values.get("api_key")?.clearSecret, true);
});

test("file-backed config field detection recognizes explicit and inferred paths", () => {
  assert.equal(
    isFileBackedDownloadClientConfigField(
      configField({ key: "output", fieldType: "PATH" }),
    ),
    true,
  );
  assert.equal(
    isFileBackedDownloadClientConfigField(configField({ key: "save_path" })),
    true,
  );
  assert.equal(
    isFileBackedDownloadClientConfigField(
      configField({
        key: "target",
        valueSource: "HOST_BINDING",
        hostBinding: "download_directory",
      }),
    ),
    true,
  );
  assert.equal(
    isFileBackedDownloadClientConfigField(configField({ key: "username" })),
    false,
  );
});
