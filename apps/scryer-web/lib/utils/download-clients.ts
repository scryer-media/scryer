import type {
  DownloadClientConfigPayloadRecord,
  DownloadClientDraft,
  DownloadClientRecord,
  DownloadClientTypeOption,
  ConfigFieldDef,
  ProviderTypeInfo,
} from "../types/index.ts";
import {
  BUILT_IN_DOWNLOAD_CLIENT_TYPE_LABELS,
  BUILT_IN_DOWNLOAD_CLIENT_TYPES,
  DEFAULT_DOWNLOAD_CLIENT_TYPE,
  DEFAULT_DOWNLOAD_CLIENT_DRAFT,
  WEAVER_API_KEY_SETUP_PATH,
} from "../constants/download-clients.ts";
import {
  type ProviderConfigValueInput,
  providerConfigValuesToRecord,
} from "./provider-config.ts";
import { parseHostInput } from "./url-input.ts";

type BuiltInDownloadClientType = (typeof BUILT_IN_DOWNLOAD_CLIENT_TYPES)[number];

export function isBuiltInDownloadClientType(value: string): value is BuiltInDownloadClientType {
  const normalized = value.trim().toLowerCase();
  return BUILT_IN_DOWNLOAD_CLIENT_TYPES.includes(normalized as BuiltInDownloadClientType);
}

export function normalizeDownloadClientType(
  value: string,
  fallback = DEFAULT_DOWNLOAD_CLIENT_TYPE,
): string {
  const normalized = value.trim().toLowerCase();
  return normalized || fallback;
}

export function buildDownloadClientTypeOptions(
  providerTypes: ProviderTypeInfo[],
): DownloadClientTypeOption[] {
  const options: DownloadClientTypeOption[] = BUILT_IN_DOWNLOAD_CLIENT_TYPES.map((value) => ({
    value,
    label: BUILT_IN_DOWNLOAD_CLIENT_TYPE_LABELS[value],
  }));
  const seenValues = new Set(options.map((option) => option.value));

  for (const providerType of providerTypes) {
    const value = normalizeDownloadClientType(providerType.providerType, "");
    if (!value || seenValues.has(value)) {
      continue;
    }

    options.push({
      value,
      label: providerType.name?.trim() || value,
      configFields: providerType.configFields,
      defaultBaseUrl: providerType.defaultBaseUrl,
    });
    seenValues.add(value);
  }

  return options;
}

export function ensureDownloadClientTypeOption(
  options: DownloadClientTypeOption[],
  clientType: string,
): DownloadClientTypeOption[] {
  const normalized = normalizeDownloadClientType(clientType, "");
  if (!normalized || options.some((option) => option.value === normalized)) {
    return options;
  }

  return [
    ...options,
    {
      value: normalized,
      label: clientType.trim() || normalized,
    },
  ];
}

export const FIXED_DOWNLOAD_CLIENT_CONFIG_FIELD_KEYS = new Set([
  "host",
  "port",
  "use_ssl",
  "useSsl",
  "url_base",
  "urlBase",
  "base_url",
  "baseUrl",
  "remote_path_mappings",
  "remotePathMappings",
  "client_type",
]);

export function defaultDownloadClientConfigValuesForFields(
  fields: ConfigFieldDef[],
) {
  return Object.fromEntries(
    fields.map((field) => [
      field.key,
      field.defaultValue ?? (field.fieldType === "BOOL" ? "false" : ""),
    ]),
  );
}

export function downloadClientConfigFieldValue(
  draft: DownloadClientDraft,
  field: ConfigFieldDef,
  hasStoredSecretValue = false,
) {
  return (
    draft.configValues[field.key] ??
    (hasStoredSecretValue ? "" : field.defaultValue) ??
    (field.fieldType === "BOOL" ? "false" : "")
  );
}

function configFieldByKey(fields: ConfigFieldDef[]) {
  return new Map(fields.map((field) => [field.key, field]));
}

function normalizedConfigFieldKeys(fields: ConfigFieldDef[]) {
  return new Set(fields.map((field) => field.key.trim().toLowerCase()));
}

function descriptorOwnsAnyField(
  fieldKeys: Set<string>,
  candidates: string[],
): boolean {
  return candidates.some((candidate) =>
    fieldKeys.has(candidate.trim().toLowerCase()),
  );
}

function configValueInput(
  key: string,
  value: unknown,
  field: ConfigFieldDef | undefined,
): ProviderConfigValueInput {
  const normalizedKey = key.trim().toLowerCase();
  const isSecretKey =
    normalizedKey === "api_key" ||
    normalizedKey === "apikey" ||
    normalizedKey === "username" ||
    normalizedKey === "user_name" ||
    normalizedKey.includes("api_key") ||
    normalizedKey.includes("password") ||
    normalizedKey.includes("secret") ||
    normalizedKey.includes("token");

  if (field?.fieldType === "BOOL") {
    return { key, boolValue: String(value).trim().toLowerCase() === "true" };
  }

  if (field?.fieldType === "NUMBER") {
    const parsed = Number(String(value).trim());
    if (Number.isNaN(parsed)) {
      return { key, stringValue: String(value) };
    }
    if (Number.isInteger(parsed)) {
      return { key, intValue: parsed };
    }
    return { key, floatValue: parsed };
  }

  if (typeof value === "boolean") {
    return { key, boolValue: value };
  }
  if (typeof value === "number" && Number.isInteger(value)) {
    return { key, intValue: value };
  }
  if (typeof value === "number") {
    return { key, floatValue: value };
  }
  if (isSecretKey) {
    return { key, secretValue: String(value) };
  }
  return { key, stringValue: String(value) };
}

export function readConfigValueAsString(rawValue: unknown): string {
  if (typeof rawValue === "string") {
    return rawValue.trim();
  }
  if (typeof rawValue === "number" && Number.isFinite(rawValue)) {
    return String(rawValue);
  }
  if (typeof rawValue === "boolean") {
    return rawValue ? "true" : "false";
  }
  return "";
}

export function readConfigValueAsBoolean(rawValue: unknown): boolean {
  if (typeof rawValue === "boolean") {
    return rawValue;
  }
  if (typeof rawValue === "number" && Number.isFinite(rawValue)) {
    return rawValue !== 0;
  }
  if (typeof rawValue === "string") {
    const normalized = rawValue.trim().toLowerCase();
    if (normalized === "false") {
      return false;
    }
    return normalized === "1" || normalized === "true" || normalized === "yes";
  }
  return false;
}

function parseJsonPayloadObject(raw: string | null | undefined) {
  if (!raw) {
    return null;
  }
  try {
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : null;
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to parse download client config JSON", { raw, error });
    }
    return null;
  }
}

export function parseJsonPayload(raw: string | null | undefined): DownloadClientConfigPayloadRecord {
  const parsed = parseJsonPayloadObject(raw);
  return parsed ? (parsed as DownloadClientConfigPayloadRecord) : {};
}

export function readConfigStringValue(
  payload: DownloadClientConfigPayloadRecord,
  keys: string[],
  fallback = "",
) {
  for (const key of keys) {
    const readValue = readConfigValueAsString(payload[key]);
    if (readValue) {
      return readValue;
    }
  }
  return fallback;
}

export function readConfigBooleanValue(
  payload: DownloadClientConfigPayloadRecord,
  keys: string[],
  fallback = false,
) {
  for (const key of keys) {
    if (!Object.prototype.hasOwnProperty.call(payload, key)) {
      continue;
    }
    const rawValue = payload[key];
    if (typeof rawValue === "string" || typeof rawValue === "number" || typeof rawValue === "boolean") {
      return readConfigValueAsBoolean(rawValue);
    }
  }
  return fallback;
}

export function splitBaseUrlForDraft(
  rawBaseUrl: string | null | undefined,
): {
  host: string;
  port: string;
  urlBase: string;
  useSsl: boolean;
} {
  const raw = (rawBaseUrl ?? "").trim();
  if (!raw) {
    return {
      host: "",
      port: "",
      urlBase: "",
      useSsl: false,
    };
  }

  const withScheme = raw.includes("://") ? raw : `https://${raw}`;
  try {
    const parsed = new URL(withScheme);
    const host = parsed.hostname.trim();
    const port = parsed.port.trim();
    const normalizedPath = parsed.pathname === "/" ? "" : parsed.pathname.trim();
    return {
      host,
      port,
      urlBase: normalizedPath,
      useSsl: parsed.protocol.toLowerCase() === "https:",
    };
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to parse download client base URL", { rawBaseUrl: raw, error });
    }
    return {
      host: "",
      port: "",
      urlBase: "",
      useSsl: false,
    };
  }
}

/**
 * The draft as it will be stored, with whatever the operator typed into the
 * host box put where it belongs.
 *
 * Nobody types a bare hostname into a host box when they have a URL in front of
 * them: they paste `http://192.168.1.5:8080/qbt`, because that is what their
 * browser is showing. So a scheme sets the SSL box, a port fills the port box,
 * and a path fills the URL-base box, rather than all three being refused as
 * "not a host". A scheme-less value leaves the SSL box alone, because that
 * checkbox is the operator's own statement and nothing here should overrule it.
 */
export function normalizeDownloadClientDraft(
  draft: DownloadClientDraft,
): DownloadClientDraft {
  const parsed = parseHostInput(draft.host);
  if (!parsed) {
    return draft;
  }
  const port = parsed.port !== "" ? parsed.port : draft.port.trim();
  const urlBase =
    parsed.path !== "" ? parsed.path : draft.urlBase.trim().replace(/\/+$/, "");
  const useSsl = parsed.scheme === null ? draft.useSsl : parsed.scheme === "https";
  if (
    parsed.host === draft.host &&
    port === draft.port &&
    urlBase === draft.urlBase &&
    useSsl === draft.useSsl
  ) {
    return draft;
  }
  return { ...draft, host: parsed.host, port, urlBase, useSsl };
}

export function buildDownloadClientBaseUrl(rawDraft: DownloadClientDraft) {
  // Normalized here as well as on save, so every caller — the connection test
  // included — addresses the same server the operator meant.
  const draft = normalizeDownloadClientDraft(rawDraft);
  const host = draft.host.trim();
  if (!host) {
    return "";
  }
  const normalizedPort = draft.port.trim().length ? `:${draft.port.trim()}` : "";
  const protocol = draft.useSsl ? "https" : "http";
  const basePath = draft.urlBase.trim();
  const normalizedPath = basePath ? `/${basePath.replace(/^\/+/, "")}` : "";
  return `${protocol}://${host}${normalizedPort}${normalizedPath}`;
}

export function cleanPayloadObject(payload: Record<string, unknown>) {
  return Object.entries(payload).reduce<Record<string, unknown>>((accumulator, [key, value]) => {
    if (typeof value === "undefined" || value === null) {
      return accumulator;
    }
    if (typeof value === "string" && !value.trim()) {
      return accumulator;
    }
    accumulator[key] = value;
    return accumulator;
  }, {});
}

/// Whether a field should offer the folder browser.
///
/// `PATH` is the declaration to reach for. The name inference behind it is
/// deliberate and covers providers that predate the `path` field type — it is
/// tested, so do not drop it without moving those declarations over first.
///
/// `role` and `valueSource` used to be sniffed the same way. They cannot match:
/// the only values they take are `CONNECTION_URL`, `USER` and `HOST_BINDING`,
/// none of which contain any of these words.
export function isFileBackedDownloadClientConfigField(field: ConfigFieldDef): boolean {
  const key = field.key.trim().toLowerCase();
  const hostBinding = field.hostBinding?.trim().toLowerCase() ?? "";
  const pathPattern = /(?:^|[_\-.])(path|folder|directory|file)(?:$|[_\-.])/;
  const namesAPath = (value: string) =>
    pathPattern.test(value) ||
    value.endsWith("path") ||
    value.endsWith("folder") ||
    value.endsWith("directory") ||
    value.endsWith("file");

  return (
    field.fieldType === "PATH" || namesAPath(key) || namesAPath(hostBinding)
  );
}

export function buildDownloadClientConfigValues(
  rawDraft: DownloadClientDraft,
  fields: ConfigFieldDef[] = [],
  storedSecretKeys: ReadonlySet<string> = new Set(),
) {
  const draft = normalizeDownloadClientDraft(rawDraft);
  const normalizedClientType = normalizeDownloadClientType(draft.clientType);
  const descriptorFieldKeys = normalizedConfigFieldKeys(fields);
  const payload: DownloadClientConfigPayloadRecord = {
    ...draft.configValues,
    host: draft.host.trim(),
    port: draft.port.trim(),
    use_ssl: draft.useSsl,
    url_base: draft.urlBase.trim(),
    remote_path_mappings: draft.remotePathMappings,
    client_type: normalizedClientType,
  };

  if (!descriptorOwnsAnyField(descriptorFieldKeys, ["username", "user_name"])) {
    payload.username = draft.username.trim();
  }
  if (!descriptorOwnsAnyField(descriptorFieldKeys, ["password"])) {
    payload.password = draft.password.trim();
  }
  if (
    (normalizedClientType === "sabnzbd" || normalizedClientType === "weaver") &&
    !descriptorOwnsAnyField(descriptorFieldKeys, ["api_key", "apiKey", "apikey"])
  ) {
    payload.api_key = draft.apiKey.trim();
  }

  const fieldsByKey = configFieldByKey(fields);
  const cleaned = cleanPayloadObject(payload);
  const values = Object.entries(cleaned).map(([key, value]): ProviderConfigValueInput =>
    configValueInput(key, value, fieldsByKey.get(key)),
  );
  const explicitSecretClears = fields
    .filter(
      (field) =>
        field.fieldType === "PASSWORD"
        && storedSecretKeys.has(field.key)
        && Object.hasOwn(draft.configValues, field.key)
        && draft.configValues[field.key] === "",
    )
    .map((field): ProviderConfigValueInput => ({
      key: field.key,
      clearSecret: true,
    }));
  return [...values, ...explicitSecretClears];
}

export function buildDownloadClientDraftFromRecord(record: DownloadClientRecord): DownloadClientDraft {
  const baseUrlParts = splitBaseUrlForDraft(record.baseUrl);
  const config = providerConfigValuesToRecord(record.config);

  // Fall back to config JSON fields when baseUrl is absent (e.g. weaver
  // entries that resolve host/port/ssl from config rather than a full URL).
  const host = baseUrlParts.host || readConfigStringValue(config, ["host"]);
  const port = baseUrlParts.port || readConfigStringValue(config, ["port"]);
  const urlBase = baseUrlParts.urlBase || readConfigStringValue(config, ["url_base", "urlBase"]);

  return {
    ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
    name: record.name,
    clientType: normalizeDownloadClientType(record.clientType),
    host,
    port,
    urlBase,
    isEnabled: record.isEnabled,
    proxyConfigId: record.proxyConfigId ?? null,
    apiKey: readConfigStringValue(config, ["api_key", "apiKey", "apikey"]),
    username: readConfigStringValue(config, ["username"]),
    password: "",
    remotePathMappings: readConfigStringValue(config, ["remote_path_mappings", "remotePathMappings"]),
    configValues: config,
    useSsl: readConfigBooleanValue(config, ["use_ssl", "useSsl"], baseUrlParts.useSsl),
  };
}

export function buildUrlPreview(draft: DownloadClientDraft): string {
  return buildDownloadClientBaseUrl(draft);
}

export function buildWeaverApiKeyUrl(draft: DownloadClientDraft): string {
  const baseUrl = buildDownloadClientBaseUrl(draft).replace(/\/+$/, "");
  if (!baseUrl) {
    return "";
  }
  return `${baseUrl}${WEAVER_API_KEY_SETUP_PATH}`;
}
