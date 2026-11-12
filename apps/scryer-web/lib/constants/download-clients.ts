import type { DownloadClientDraft } from "@/lib/types/download-clients";
import type { ConfigFieldDef } from "@/lib/types/indexers";

export const BUILT_IN_DOWNLOAD_CLIENT_TYPES = [
  "weaver",
  "sabnzbd",
  "nzbget",
  "http",
  "torbox",
] as const;

/// Built-in clients that are not a server: no host, port or URL to set.
export const HOSTLESS_DOWNLOAD_CLIENT_TYPES: ReadonlySet<string> = new Set([
  "http",
  "torbox",
]);

export const DEFAULT_DOWNLOAD_CLIENT_TYPE = "weaver";

export const BUILT_IN_DOWNLOAD_CLIENT_TYPE_LABELS: Record<string, string> = {
  nzbget: "NZBGet",
  sabnzbd: "SABnzbd",
  weaver: "Weaver",
  qbittorrent: "qBittorrent",
  http: "HTTP",
  torbox: "TorBox",
};

function builtInField(
  field: Pick<ConfigFieldDef, "key" | "label" | "fieldType" | "required"> &
    Partial<ConfigFieldDef>,
): ConfigFieldDef {
  return {
    defaultValue: null,
    valueSource: "USER",
    role: null,
    hostBinding: null,
    options: [],
    helpText: null,
    visibleWhen: null,
    requiredWhen: null,
    advanced: false,
    ...field,
  };
}

/// The download folder of Scryer's own downloader, which every built-in
/// client that fetches files itself writes to.
const DOWNLOAD_DIR_FIELD = builtInField({
  key: "download_dir",
  label: "settings.downloadClientHttpDownloadDir",
  helpText: "settings.downloadClientHttpDownloadDirHelp",
  fieldType: "PATH",
  required: true,
});

/// The transfer limits of Scryer's own downloader.
const DOWNLOADER_LIMIT_FIELDS: ConfigFieldDef[] = [
  builtInField({
    key: "max_concurrent_downloads",
    label: "settings.downloadClientHttpMaxConcurrentDownloads",
    helpText: "settings.downloadClientHttpMaxConcurrentDownloadsHelp",
    fieldType: "NUMBER",
    required: false,
    defaultValue: "2",
  }),
  builtInField({
    key: "max_connections_per_host",
    label: "settings.downloadClientHttpMaxConnectionsPerHost",
    helpText: "settings.downloadClientHttpMaxConnectionsPerHostHelp",
    fieldType: "NUMBER",
    required: false,
    defaultValue: "2",
    advanced: true,
  }),
  builtInField({
    key: "bandwidth_limit_kbps",
    label: "settings.downloadClientHttpBandwidthLimit",
    helpText: "settings.downloadClientHttpBandwidthLimitHelp",
    fieldType: "NUMBER",
    required: false,
    advanced: true,
  }),
];

/// Settings of built-in clients whose form is not host/port/credentials.
/// Labels and help texts are translation keys.
export const BUILT_IN_DOWNLOAD_CLIENT_CONFIG_FIELDS: Record<
  string,
  ConfigFieldDef[]
> = {
  http: [DOWNLOAD_DIR_FIELD, ...DOWNLOADER_LIMIT_FIELDS],
  torbox: [
    builtInField({
      key: "api_key",
      label: "settings.downloadClientTorboxApiKey",
      helpText: "settings.downloadClientTorboxApiKeyHelp",
      fieldType: "PASSWORD",
      required: true,
    }),
    DOWNLOAD_DIR_FIELD,
    builtInField({
      key: "cached_only",
      label: "settings.downloadClientTorboxCachedOnly",
      helpText: "settings.downloadClientTorboxCachedOnlyHelp",
      fieldType: "BOOL",
      required: false,
      defaultValue: "false",
    }),
    ...DOWNLOADER_LIMIT_FIELDS,
  ],
};

export const WEAVER_API_KEY_SETUP_PATH =
  "/settings/security?createApiKey=1&name=Scryer&scope=integration";

export const DEFAULT_PORT_FOR_CLIENT_TYPE: Record<string, string> = {
  nzbget: "6789",
  sabnzbd: "8080",
  weaver: "9090",
  qbittorrent: "8080",
};

export const DEFAULT_DOWNLOAD_CLIENT_DRAFT: DownloadClientDraft = {
  name: "Weaver",
  clientType: DEFAULT_DOWNLOAD_CLIENT_TYPE,
  host: "",
  port: DEFAULT_PORT_FOR_CLIENT_TYPE[DEFAULT_DOWNLOAD_CLIENT_TYPE] ?? "8080",
  urlBase: "",
  useSsl: false,
  apiKey: "",
  username: "",
  password: "",
  remotePathMappings: "",
  configValues: {},
  proxyConfigId: null,
  isEnabled: true,
};
