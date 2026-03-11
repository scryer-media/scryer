import type { DownloadClientDraft } from "@/lib/types/download-clients";

export const BUILT_IN_DOWNLOAD_CLIENT_TYPES = ["nzbget", "sabnzbd"] as const;

export const DEFAULT_DOWNLOAD_CLIENT_TYPE = "nzbget";

export const BUILT_IN_DOWNLOAD_CLIENT_TYPE_LABELS: Record<
  (typeof BUILT_IN_DOWNLOAD_CLIENT_TYPES)[number],
  string
> = {
  nzbget: "NZBGet",
  sabnzbd: "SABnzbd",
};

export const DEFAULT_DOWNLOAD_CLIENT_DRAFT: DownloadClientDraft = {
  name: "",
  clientType: DEFAULT_DOWNLOAD_CLIENT_TYPE,
  host: "",
  port: "8080",
  urlBase: "",
  useSsl: false,
  apiKey: "",
  username: "",
  password: "",
  isEnabled: true,
};
