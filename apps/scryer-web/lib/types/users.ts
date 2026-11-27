import type { ExternalAccountProvider } from "./settings";

export type UserAccountKind = "LOCAL" | "EXTERNAL_AUTO_PROVISIONED";

export type UserRecord = {
  id: string;
  username: string;
  loginEnabled: boolean;
  isDefaultAdmin: boolean;
  hasPassword: boolean;
  passwordChangeRequired: boolean;
  hasMfa: boolean;
  hasPasskey: boolean;
  accountKind: UserAccountKind;
  /** Media servers with an active, verified link; only Known users loads it. */
  linkedMediaServers?: ExternalAccountProvider[];
  appPermissions: string[];
  libraryPermissions: {
    libraryId: string;
    permissions: string[];
  }[];
};
