export type SubtitleLanguagePreference = {
  code: string;
  hearingImpaired: boolean;
  forced: boolean;
};

export type SubtitleSettings = {
  enabled: boolean;
  languages: SubtitleLanguagePreference[];
  autoDownloadOnImport: boolean;
  minimumScoreSeries: number;
  minimumScoreMovie: number;
  searchIntervalHours: number;
  includeAiTranslated: boolean;
  includeMachineTranslated: boolean;
  syncEnabled: boolean;
  syncThresholdSeries: number;
  syncThresholdMovie: number;
  syncMaxOffsetSeconds: number;
};

export type AcquisitionSettings = {
  enabled: boolean;
  upgradeCooldownHours: number;
  sameTierMinDelta: number;
  crossTierMinDelta: number;
  forcedUpgradeDeltaBypass: number;
  pollIntervalSeconds: number;
  syncIntervalSeconds: number;
  batchSize: number;
};

export type GeneralSettings = {
  keepHistoryForever: boolean;
  historyRetentionDays: number;
  imageCacheMaxSizeMb: number;
  effectiveImageCacheMaxSizeBytes: number;
  effectiveImageCacheMaxSizeMb: number;
  imageCacheMaxSizeEnvOverrideActive: boolean;
  pluginHttpCaBundlePem: string;
  pluginHttpTrustedCertificates: TrustedCertificateEntry[];
};

export type GeneralSettingsUpdate = Partial<
  Pick<
    GeneralSettings,
    | "keepHistoryForever"
    | "historyRetentionDays"
    | "imageCacheMaxSizeMb"
    | "pluginHttpCaBundlePem"
  >
>;

export type UiDateTimeFormat = "LOCALE" | "ISO24H";

export type UiTableColumnSetting = {
  facet: string;
  tableViewMode: string;
  columnId: string;
  columnOrder: number;
  visible: boolean;
};

export type UiSettings = {
  theme: "LIGHT" | "DARK" | "PRIDE" | "SYSTEM";
  dateTimeFormat: UiDateTimeFormat;
  highlightColor: string | null;
  secondaryColor: string | null;
  highContrastMode: boolean;
  reduceMotion: boolean;
  hideSponsorButton: boolean;
  density: "COMPACT" | "COMFORTABLE";
  sidebarMode: "EXPANDED" | "COLLAPSED";
  defaultLandingView:
    | "MOVIES"
    | "SERIES"
    | "ANIME"
    | "ACTIVITY"
    | "CALENDAR"
    | "WANTED"
    | "HISTORY"
    | "SETTINGS"
    | "SYSTEM";
  tableColumns: UiTableColumnSetting[];
};

export type TrustedCertificateEntry = {
  fingerprintSha256: string;
  pem: string;
};

export type SecuritySettings = {
  formLoginEnabled: boolean;
  passwordMinLength: number;
  skipLoginForLocalIps: boolean;
  apiKeysRestrictToSystemSettingsUsers: boolean;
  mfaRequireConfigStepUp: boolean;
  mfaRequirePasswordLogin: boolean;
  mfaRequireJellyfinLogin: boolean;
  mfaRequireEmbyLogin: boolean;
  effectiveFormLoginEnabled: boolean;
  envOverrideActive: boolean;
  envOverrideDescription: string | null;
};

export type ExternalAccountProvider = "PLEX" | "JELLYFIN" | "EMBY";
export type ExternalAccountStatus = "PENDING_CLAIM" | "ACTIVE" | "DISABLED";
export type MediaServerProvider = "JELLYFIN" | "PLEX" | "EMBY";

export type MediaServerPathMapping = {
  sourcePath: string;
  destinationPath: string;
};

export type MediaServerDefaultLibraryGrant = {
  libraryId: string;
  permissions: string[];
};

export type MediaServerConnection = {
  id: string;
  provider: MediaServerProvider;
  displayName: string;
  baseUrl: string;
  externalUrl: string | null;
  enabled: boolean;
  loginEnabled: boolean;
  linkingEnabled: boolean;
  autoAddEnabled: boolean;
  defaultAppPermissions: string[];
  defaultLibraryGrants: MediaServerDefaultLibraryGrant[];
  machineIdPresent: boolean;
  apiKeyPresent: boolean;
  embyServerIdPresent: boolean;
  embyConnectEnabled: boolean;
  pathMappings: MediaServerPathMapping[];
  createdAt: string;
  updatedAt: string;
};

export type PlexServerDiscovery = {
  id: string;
  name: string;
};

export type EmbyConnectAddressStatus =
  | "REACHABLE"
  | "UNREACHABLE"
  | "INVALID_URL"
  | "SERVER_ID_MISMATCH";
export type EmbyConnectServer = {
  serverId: string;
  name: string;
  userType: "LINKED_USER" | "GUEST" | "UNKNOWN";
  localAddress: string | null;
  remoteAddress: string | null;
  localApiBaseUrl: string | null;
  remoteApiBaseUrl: string | null;
  localStatus: EmbyConnectAddressStatus;
  remoteStatus: EmbyConnectAddressStatus;
  suggestedBaseUrl: string | null;
};

export type MediaServerUserGroupStatus =
  | "READY"
  | "MISSING_CREDENTIALS"
  | "ERROR";

export type MediaServerUser = {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
};

export type MediaServerUserGroup = {
  connectionId: string;
  connectionName: string;
  provider: ExternalAccountProvider;
  status: MediaServerUserGroupStatus;
  errorMessage: string | null;
  users: MediaServerUser[];
};

export type MediaServerConnectionDraft = {
  provider: MediaServerProvider;
  displayName: string;
  baseUrl: string;
  externalUrl: string;
  enabled: boolean;
  loginEnabled: boolean;
  linkingEnabled: boolean;
  autoAddEnabled: boolean;
  defaultAppPermissions: string[];
  defaultLibraryGrants: MediaServerDefaultLibraryGrant[];
  machineIdPresent: boolean;
  plexServerId: string;
  apiKey: string;
  clearApiKey: boolean;
  jellyfinCredentialMode: "apiKey" | "adminLogin";
  embyConnectionMode: "LOCAL" | "CONNECT";
  embyLocalSetupMethod: "API_KEY" | "ADMIN_CREDENTIALS";
  embyConnectEnabled: boolean;
  embyConnectUsernameOrEmail: string;
  embyConnectPassword: string;
  embyConnectServerId: string;
  embyDiscoveredServers: EmbyConnectServer[];
  adminUsername: string;
  adminPassword: string;
  pathMappingsText: string;
};

export type ExternalAuthRuntimeConnection = {
  id: string;
  provider: ExternalAccountProvider;
  displayName: string;
  loginEnabled: boolean;
  linkingEnabled: boolean;
  embyConnectEnabled: boolean;
};

export type ExternalAuthRuntimeSettings = {
  loginProviders: ExternalAccountProvider[];
  linkingProviders: ExternalAccountProvider[];
  connections: ExternalAuthRuntimeConnection[];
};

export type LinkedAccount = {
  id: string;
  userId: string;
  provider: ExternalAccountProvider;
  connectionId: string;
  externalUserId: string | null;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  status: ExternalAccountStatus;
  verifiedAt: string | null;
  lastLoginAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type AutoBackupSettings = {
  enabled: boolean;
  dailyTimeLocal: string;
  autoBackupKeyPresent: boolean;
  autoBackupDisabledMissingKeyNotice: boolean;
  nextRunAt: string | null;
};

export type BackupSettings = {
  customBackupPath: string | null;
  defaultBackupPath: string;
  effectiveBackupPath: string;
};

export type PluginAutoUpdateSettings = {
  enabled: boolean;
};

export type AuthRuntimeState = {
  effectiveFormLoginEnabled: boolean;
  skipLoginForLocalIps: boolean;
  passkeyEnabled: boolean;
  defaultPersistSession: boolean;
  envOverrideActive: boolean;
  mfaRequirePasswordLogin: boolean;
  mfaRequireConfigStepUp: boolean;
  mfaRequireJellyfinLogin: boolean;
};

export type PasskeySummary = {
  id: string;
  friendlyName: string | null;
  createdAt: string;
  lastUsedAt: string | null;
};

export type OAuthConnectedApp = {
  grantId: string;
  clientId: string;
  clientName: string;
  authorizedAt: string;
  lastUsedAt: string | null;
};

export type ApiKeySummary = {
  id: string;
  label: string;
  actor: string;
  expiresAt: string | null;
  revokedAt: string | null;
  lastUsedAt: string | null;
  createdAt: string;
  provisioningSource: "user" | "environment";
};

export type TotpStatus = {
  enabled: boolean;
  createdAt: string | null;
  lastUsedAt: string | null;
  recoveryCodesRemaining: number;
};

export type TotpEnrollmentStart = {
  challengeId: string;
  otpauthUrl: string;
  secretBase32: string;
  expiresAt: string;
};

export type TotpEnrollmentComplete = {
  status: TotpStatus;
  recoveryCodes: string[];
};

export type ImportMode = "HARDLINK_OR_COPY" | "MOVE";

export type MediaSettings = {
  scope: "MOVIE" | "SERIES" | "ANIME";
  libraryPath: string;
  rootFolders: { path: string; isDefault: boolean }[];
  requiredAudioLanguages: string[];
  useSeasonFolders: boolean;
  folderTemplate: string;
  seasonFolderTemplate: string | null;
  specialsFolderTemplate: string | null;
  renameEnabled: boolean;
  renameTemplate: string;
  renameCollisionPolicy: string;
  renameMissingMetadataPolicy: string;
  fillerPolicy: string | null;
  recapPolicy: string | null;
  monitorSpecials: boolean | null;
  interSeasonMovies: boolean | null;
  monitorFillerMovies: boolean | null;
  nfoWriteOnImport: boolean;
  plexmatchWriteOnImport: boolean | null;
  importMode: ImportMode;
  setPermissionsLinux: boolean;
  fileChmod: string | null;
  folderChmod: string | null;
  chownGroup: string | null;
};

export type LibraryPaths = {
  moviePath: string;
  seriesPath: string;
  animePath: string;
};

export type ServiceSettings = {
  tlsCertPath: string;
  tlsKeyPath: string;
};
