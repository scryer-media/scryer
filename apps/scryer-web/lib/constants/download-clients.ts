import type { DownloadClientDraft } from "@/lib/types/download-clients";

export const BUILT_IN_DOWNLOAD_CLIENT_TYPES = ["nzbget", "sabnzbd", "weaver"] as const;

export const DEFAULT_DOWNLOAD_CLIENT_TYPE = "nzbget";

export const BUILT_IN_DOWNLOAD_CLIENT_TYPE_LABELS: Record<
  (typeof BUILT_IN_DOWNLOAD_CLIENT_TYPES)[number],
  string
> = {
  nzbget: "NZBGet",
  sabnzbd: "SABnzbd",
  weaver: "Weaver",
};

export const WEAVER_API_KEY_SETUP_PATH =
  "/settings/security?createApiKey=1&name=Scryer&scope=integration";

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
