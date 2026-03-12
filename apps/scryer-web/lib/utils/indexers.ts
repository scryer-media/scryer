import type { ProviderTypeInfo } from "@/lib/types";

function readConfigUrl(
  configValues: Record<string, string>,
  keys: string[],
): string | null {
  for (const key of keys) {
    const value = configValues[key]?.trim();
    if (value) {
      return value;
    }
  }
  return null;
}

export function providerDerivesBaseUrlFromConfig(
  provider: ProviderTypeInfo | null | undefined,
): boolean {
  return !!provider?.configFields.some((field) => field.key === "feed_url");
}

export function deriveIndexerBaseUrlFromConfig(
  configValues: Record<string, string>,
): string | null {
  const feedUrl = readConfigUrl(configValues, ["feed_url", "feedUrl"]);
  if (!feedUrl) {
    return null;
  }

  try {
    return new URL(feedUrl).origin;
  } catch {
    return null;
  }
}

export function resolveIndexerBaseUrl(
  provider: ProviderTypeInfo | null | undefined,
  baseUrl: string,
  configValues: Record<string, string>,
): string {
  return (
    provider?.defaultBaseUrl?.trim() ||
    deriveIndexerBaseUrlFromConfig(configValues) ||
    baseUrl.trim()
  );
}

export function showStandardIndexerConnectionFields(
  provider: ProviderTypeInfo | null | undefined,
): boolean {
  return showIndexerBaseUrlField(provider) && showIndexerApiKeyField(provider);
}

export function showIndexerBaseUrlField(
  provider: ProviderTypeInfo | null | undefined,
): boolean {
  return (
    !provider?.defaultBaseUrl && !providerDerivesBaseUrlFromConfig(provider)
  );
}

export function showIndexerApiKeyField(
  provider: ProviderTypeInfo | null | undefined,
): boolean {
  return !providerDerivesBaseUrlFromConfig(provider);
}
