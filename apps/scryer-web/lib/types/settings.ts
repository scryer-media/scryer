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
  pluginHttpCaBundlePem: string;
  pluginHttpTrustedCertificates: TrustedCertificateEntry[];
};

export type TrustedCertificateEntry = {
  fingerprintSha256: string;
  pem: string;
};

export type SecuritySettings = {
  formLoginEnabled: boolean;
  passwordMinLength: number;
  skipLoginForLocalIps: boolean;
  mfaRequireConfigStepUp: boolean;
  mfaRequirePasswordLogin: boolean;
  totpRequireJellyfinLogin: boolean;
  effectiveFormLoginEnabled: boolean;
  envOverrideActive: boolean;
  envOverrideDescription: string | null;
};

export type ExternalAccountProvider = "plex" | "jellyfin";
export type ExternalAccountStatus = "pending_claim" | "active" | "disabled";
export type MediaServerProvider = "jellyfin" | "plex" | "emby";

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
  enabled: boolean;
  loginEnabled: boolean;
  linkingEnabled: boolean;
  autoAddEnabled: boolean;
  defaultAppPermissions: string[];
  defaultLibraryGrants: MediaServerDefaultLibraryGrant[];
  machineIdPresent: boolean;
  apiKeyPresent: boolean;
  pathMappings: MediaServerPathMapping[];
  createdAt: string;
  updatedAt: string;
};

export type PlexServerDiscovery = {
  id: string;
  name: string;
};

export type MediaServerUserGroupStatus =
  | "ready"
  | "missing_credentials"
  | "error";

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

export type AuthRuntimeState = {
  effectiveFormLoginEnabled: boolean;
  skipLoginForLocalIps: boolean;
  passkeyEnabled: boolean;
  envOverrideActive: boolean;
  mfaRequirePasswordLogin: boolean;
  mfaRequireConfigStepUp: boolean;
  totpRequireJellyfinLogin: boolean;
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

export type ImportMode = "hardlink_or_copy" | "move";

export type MediaSettings = {
  scope: "movie" | "series" | "anime";
  libraryPath: string;
  rootFolders: { path: string; isDefault: boolean }[];
  requiredAudioLanguages: string[];
  folderTemplate: string;
  seasonFolderTemplate: string | null;
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
