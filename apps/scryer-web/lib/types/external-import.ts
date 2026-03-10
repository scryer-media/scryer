export type ExternalImportRootFolder = {
  source: string;
  path: string;
};

export type ExternalImportDownloadClient = {
  sources: string[];
  name: string;
  implementation: string;
  scryerClientType: string | null;
  host: string | null;
  port: string | null;
  useSsl: boolean;
  urlBase: string | null;
  username: string | null;
  apiKey: string | null;
  dedupKey: string;
  supported: boolean;
};

export type ExternalImportIndexer = {
  sources: string[];
  name: string;
  implementation: string;
  scryerProviderType: string | null;
  baseUrl: string | null;
  apiKey: string | null;
  dedupKey: string;
  supported: boolean;
};

export type ExternalImportPreview = {
  sonarrConnected: boolean;
  radarrConnected: boolean;
  sonarrVersion: string | null;
  radarrVersion: string | null;
  rootFolders: ExternalImportRootFolder[];
  downloadClients: ExternalImportDownloadClient[];
  indexers: ExternalImportIndexer[];
};

export type ExternalImportResult = {
  mediaPathsSaved: boolean;
  downloadClientsCreated: number;
  indexersCreated: number;
  pluginsInstalled: string[];
  errors: string[];
};
