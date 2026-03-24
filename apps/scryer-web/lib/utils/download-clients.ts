import type {
  DownloadClientConfigPayloadRecord,
  DownloadClientDraft,
  DownloadClientRecord,
  DownloadClientTypeOption,
  ProviderTypeInfo,
} from "@/lib/types";
import {
  BUILT_IN_DOWNLOAD_CLIENT_TYPE_LABELS,
  BUILT_IN_DOWNLOAD_CLIENT_TYPES,
  DEFAULT_DOWNLOAD_CLIENT_TYPE,
  DEFAULT_DOWNLOAD_CLIENT_DRAFT,
  WEAVER_API_KEY_SETUP_PATH,
} from "@/lib/constants/download-clients";

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

export function buildDownloadClientBaseUrl(draft: DownloadClientDraft) {
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

export function buildDownloadClientConfigJson(draft: DownloadClientDraft) {
  const normalizedClientType = normalizeDownloadClientType(draft.clientType);
  const payload: DownloadClientConfigPayloadRecord = {
    host: draft.host.trim(),
    port: draft.port.trim(),
    use_ssl: draft.useSsl,
    url_base: draft.urlBase.trim(),
    username: draft.username.trim(),
    password: draft.password.trim(),
    client_type: normalizedClientType,
  };

  if (normalizedClientType === "sabnzbd" || normalizedClientType === "weaver") {
    payload.api_key = draft.apiKey.trim();
  }

  const cleaned = cleanPayloadObject(payload);
  return JSON.stringify(cleaned);
}

export function buildDownloadClientDraftFromRecord(record: DownloadClientRecord): DownloadClientDraft {
  const baseUrlParts = splitBaseUrlForDraft(record.baseUrl);
  const config = parseJsonPayload(record.configJson);

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
    apiKey: readConfigStringValue(config, ["api_key", "apiKey", "apikey"]),
    username: readConfigStringValue(config, ["username"]),
    password: "",
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
