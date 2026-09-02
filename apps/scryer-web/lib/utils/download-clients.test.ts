import assert from "node:assert/strict";
import test from "node:test";

import {
  buildDownloadClientBaseUrl,
  buildDownloadClientConfigValues,
  isFileBackedDownloadClientConfigField,
  normalizeDownloadClientDraft,
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

test("a URL pasted into the host box is put where it belongs", () => {
  const pasted = normalizeDownloadClientDraft(
    draft({ host: "http://192.168.1.5:8080/qbt/", port: "", urlBase: "" }),
  );
  assert.equal(pasted.host, "192.168.1.5");
  assert.equal(pasted.port, "8080");
  assert.equal(pasted.urlBase, "/qbt");
  assert.equal(pasted.useSsl, false);

  // A scheme the operator wrote sets the SSL box, both ways.
  assert.equal(
    normalizeDownloadClientDraft(draft({ host: "https://qbit.example.com" })).useSsl,
    true,
  );
  assert.equal(
    normalizeDownloadClientDraft(draft({ host: "http://qbit.example.com", useSsl: true }))
      .useSsl,
    false,
  );
  // Without a scheme the checkbox is the operator's own statement and stands.
  assert.equal(
    normalizeDownloadClientDraft(draft({ host: "qbit.example.com", useSsl: true })).useSsl,
    true,
  );

  // A port in the address wins over a stale one in the port box; without one,
  // the port box is kept.
  assert.equal(
    normalizeDownloadClientDraft(draft({ host: "192.168.1.5:9091", port: "6789" })).port,
    "9091",
  );
  assert.equal(
    normalizeDownloadClientDraft(draft({ host: "192.168.1.5", port: "6789" })).port,
    "6789",
  );

  // Nothing to do means the same object back, and a value we should not touch
  // is left exactly as typed.
  const plain = draft({ host: "download-client", port: "6789" });
  assert.equal(normalizeDownloadClientDraft(plain), plain);
  const credentials = draft({ host: "http://admin:pw@nzbget.lan:6789" });
  assert.equal(normalizeDownloadClientDraft(credentials), credentials);
});

test("the connection test dials what a save would store", () => {
  assert.equal(
    buildDownloadClientBaseUrl(draft({ host: "http://192.168.1.5:8080/qbt", port: "", urlBase: "" })),
    "http://192.168.1.5:8080/qbt",
  );
  assert.equal(
    buildDownloadClientBaseUrl(draft({ host: "https://qbit.example.com", port: "", urlBase: "" })),
    "https://qbit.example.com",
  );
});
