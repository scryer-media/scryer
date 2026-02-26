import type { DownloadClientDraft } from "@/lib/types/download-clients";

export const SUPPORTED_DOWNLOAD_CLIENT_TYPES = ["nzbget", "sabnzbd", "qbittorrent"] as const;

export const DEFAULT_DOWNLOAD_CLIENT_DRAFT: DownloadClientDraft = {
  name: "",
  clientType: "nzbget",
  host: "",
  port: "8080",
  urlBase: "",
  useSsl: true,
  apiKey: "",
  username: "",
  password: "",
  isEnabled: true,
};
