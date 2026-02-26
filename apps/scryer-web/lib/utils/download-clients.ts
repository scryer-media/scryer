import type {
  DownloadClientConfigPayloadRecord,
  DownloadClientDraft,
  DownloadClientRecord,
} from "@/lib/types";
import {
  DEFAULT_DOWNLOAD_CLIENT_DRAFT,
  SUPPORTED_DOWNLOAD_CLIENT_TYPES,
} from "@/lib/constants/download-clients";

type SupportedDownloadClientType = (typeof SUPPORTED_DOWNLOAD_CLIENT_TYPES)[number];

export function isSupportedDownloadClientType(value: string): value is SupportedDownloadClientType {
  const normalized = value.trim().toLowerCase();
  return SUPPORTED_DOWNLOAD_CLIENT_TYPES.includes(normalized as SupportedDownloadClientType);
}

export function normalizeDownloadClientType(value: string): SupportedDownloadClientType {
  const normalized = value.trim().toLowerCase();
  return isSupportedDownloadClientType(normalized) ? normalized : "nzbget";
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
      useSsl: true,
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
      useSsl: true,
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

  if (normalizedClientType === "sabnzbd") {
    payload.api_key = draft.apiKey.trim();
  }

  const cleaned = cleanPayloadObject(payload);
  return JSON.stringify(cleaned);
}

export function buildDownloadClientDraftFromRecord(record: DownloadClientRecord): DownloadClientDraft {
  const baseUrlParts = splitBaseUrlForDraft(record.baseUrl);
  const config = parseJsonPayload(record.configJson);

  return {
    ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
    name: record.name,
    clientType: normalizeDownloadClientType(record.clientType),
    host: baseUrlParts.host,
    port: baseUrlParts.port,
    urlBase: baseUrlParts.urlBase,
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
