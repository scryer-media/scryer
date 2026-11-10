const providerTextKeys: Record<string, string> = {
  "Known provider": "providerConfig.Known_provider",
  "Base URL": "providerConfig.Base_URL",
  "API Key": "providerConfig.API_Key",
  "API key": "providerConfig.API_key",
  "API Path": "providerConfig.API_Path",
  "API path": "providerConfig.API_path",
  Host: "providerConfig.Host",
  Hostname: "providerConfig.Hostname",
  Port: "providerConfig.Port",
  Username: "providerConfig.Username",
  Password: "providerConfig.Password",
  Category: "providerConfig.Category",
  Categories: "providerConfig.Categories",
  Tags: "providerConfig.Tags",
  Priority: "providerConfig.Priority",
  Default: "providerConfig.Default",
  Custom: "providerConfig.Custom",
  None: "providerConfig.None",
  Timeout: "providerConfig.Timeout",
  "Request timeout": "providerConfig.Request_timeout",
  "Use SSL": "providerConfig.Use_SSL",
  "Verify SSL": "providerConfig.Verify_SSL",
  "Verify SSL certificate": "providerConfig.Verify_SSL_certificate",
  "URL Base": "providerConfig.URL_Base",
  "URL base": "providerConfig.URL_base",
  "URL Path": "providerConfig.URL_Path",
  "Download directory": "providerConfig.Download_directory",
  "Completed directory": "providerConfig.Completed_directory",
  "Polling interval": "providerConfig.Polling_interval",
  "Rate limit": "providerConfig.Rate_limit",
  "Search limit": "providerConfig.Search_limit",
  "Page size": "providerConfig.Page_size",
  Proxy: "providerConfig.Proxy",
  "Enable RSS": "providerConfig.Enable_RSS",
  "Enable search": "providerConfig.Enable_search",
  "Seed ratio": "providerConfig.Seed_ratio",
  "Seed time": "providerConfig.Seed_time",
  "Minimum seeders": "providerConfig.Minimum_seeders",
};

/** Localize display labels only; configuration keys, values and unknown plugin text are unchanged. */
export function translateProviderConfigText(
  text: string,
  t: (key: string) => string,
): string {
  const key = providerTextKeys[text];
  return key ? t(key) : text;
}
