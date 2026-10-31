import type {
  ExternalAccountProvider,
  MediaServerProvider,
} from "@/lib/types/settings";

export const VISIBLE_MEDIA_SERVER_PROVIDERS = [
  "JELLYFIN",
  "PLEX",
  "EMBY",
] as const satisfies readonly MediaServerProvider[];
export const VISIBLE_EXTERNAL_ACCOUNT_PROVIDERS = [
  "JELLYFIN",
  "PLEX",
  "EMBY",
] as const satisfies readonly ExternalAccountProvider[];

export type VisibleMediaServerProvider =
  (typeof VISIBLE_MEDIA_SERVER_PROVIDERS)[number];
export type VisibleExternalAccountProvider =
  (typeof VISIBLE_EXTERNAL_ACCOUNT_PROVIDERS)[number];

const VISIBLE_MEDIA_SERVER_PROVIDER_SET = new Set<string>(
  VISIBLE_MEDIA_SERVER_PROVIDERS,
);
const VISIBLE_EXTERNAL_ACCOUNT_PROVIDER_SET = new Set<string>(
  VISIBLE_EXTERNAL_ACCOUNT_PROVIDERS,
);

export function isVisibleMediaServerProvider(
  provider: MediaServerProvider | string,
): provider is VisibleMediaServerProvider {
  return VISIBLE_MEDIA_SERVER_PROVIDER_SET.has(provider);
}

export function isVisibleExternalAccountProvider(
  provider: ExternalAccountProvider | string,
): provider is VisibleExternalAccountProvider {
  return VISIBLE_EXTERNAL_ACCOUNT_PROVIDER_SET.has(provider);
}
