import type { Client } from "@urql/core";
import { backendClient } from "@/lib/graphql/urql-client";
import {
  accountSecurityPasskeyCompleteMutation,
  accountSecurityPasskeyStartMutation,
  webauthnAuthenticateCompleteMutation,
  webauthnAuthenticateStartMutation,
  loginVerificationPasskeyCompleteMutation,
  loginVerificationPasskeyStartMutation,
  webauthnLoginEnrollmentCompleteMutation,
  webauthnLoginEnrollmentStartMutation,
  webauthnRegisterCompleteMutation,
  webauthnRegisterStartMutation,
} from "@/lib/graphql/mutations";
import type { AuthUser } from "@/lib/hooks/use-auth";
import type { PasskeySummary } from "@/lib/types/settings";

type JsonCreationOptions = {
  challenge: string;
  rp: PublicKeyCredentialRpEntity;
  user: PublicKeyCredentialUserEntity & { id: string };
  pubKeyCredParams: PublicKeyCredentialParameters[];
  timeout?: number;
  excludeCredentials?: Array<PublicKeyCredentialDescriptor & { id: string }>;
  authenticatorSelection?: AuthenticatorSelectionCriteria;
  attestation?: AttestationConveyancePreference;
  extensions?: AuthenticationExtensionsClientInputs;
};

type JsonCredentialCreationOptions = {
  publicKey: JsonCreationOptions;
};

type JsonRequestOptions = {
  challenge: string;
  timeout?: number;
  rpId?: string;
  allowCredentials?: Array<PublicKeyCredentialDescriptor & { id: string }>;
  userVerification?: UserVerificationRequirement;
  extensions?: AuthenticationExtensionsClientInputs;
};

type JsonCredentialRequestOptions = {
  publicKey: JsonRequestOptions;
  mediation?: CredentialMediationRequirement;
};

type RequestOptionsMode = "manual" | "conditional";

type LoginPayload = {
  token: string;
  user: AuthUser | null;
  mfaEnrollmentRequired?: boolean;
  mfaVerifiedUntil?: string | null;
  persistSession: boolean;
};

type PublicKeyCredentialJsonHelpers = {
  parseCreationOptionsFromJSON?: (value: unknown) => PublicKeyCredentialCreationOptions;
  parseRequestOptionsFromJSON?: (value: unknown) => PublicKeyCredentialRequestOptions;
};

type ConditionalMediationCredential = typeof PublicKeyCredential & {
  isConditionalMediationAvailable?: () => Promise<boolean>;
};

export class PasskeyClientError extends Error {
  readonly code: "unsupported" | "cancelled" | "invalid_response" | "failed";

  constructor(code: "unsupported" | "cancelled" | "invalid_response" | "failed", message: string) {
    super(message);
    this.code = code;
  }
}

function credentialHelpers(): PublicKeyCredentialJsonHelpers {
  return PublicKeyCredential as unknown as PublicKeyCredentialJsonHelpers;
}

function ensurePasskeySupport() {
  if (
    typeof window === "undefined" ||
    typeof PublicKeyCredential === "undefined" ||
    typeof navigator === "undefined" ||
    typeof navigator.credentials?.create !== "function" ||
    typeof navigator.credentials?.get !== "function"
  ) {
    throw new PasskeyClientError("unsupported", "Passkeys are not supported in this browser.");
  }
}

function toUint8Array(value: ArrayBuffer | ArrayBufferView): Uint8Array {
  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }

  return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
}

function base64UrlToBuffer(value: string): ArrayBuffer {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
  const binary = window.atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
}

function bufferToBase64Url(value: ArrayBuffer | ArrayBufferView | null | undefined): string | null {
  if (!value) {
    return null;
  }

  const bytes = toUint8Array(value);
  let binary = "";
  bytes.forEach((byte) => {
    binary += String.fromCharCode(byte);
  });

  return window
    .btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

function parseCreationOptions(optionsJson: unknown): CredentialCreationOptions {
  const parsed = optionsJson as JsonCredentialCreationOptions;
  const publicKey = parsed.publicKey;
  if (!publicKey?.challenge || !publicKey.user?.id) {
    throw new PasskeyClientError(
      "invalid_response",
      "Passkey registration options were malformed.",
    );
  }

  const helper = credentialHelpers();
  if (typeof helper.parseCreationOptionsFromJSON === "function") {
    return { publicKey: helper.parseCreationOptionsFromJSON(publicKey) };
  }

  return {
    publicKey: {
      ...publicKey,
      challenge: base64UrlToBuffer(publicKey.challenge),
      user: {
        ...publicKey.user,
        id: base64UrlToBuffer(publicKey.user.id),
      },
      excludeCredentials: publicKey.excludeCredentials?.map((credential) => ({
        ...credential,
        id: base64UrlToBuffer(credential.id),
      })),
    },
  };
}

function normalizeRequestOptionsForMode(
  publicKey: JsonRequestOptions,
  mediation: CredentialMediationRequirement | undefined,
  mode: RequestOptionsMode,
): JsonCredentialRequestOptions {
  const normalizedPublicKey = { ...publicKey };
  let normalizedMediation = mediation;

  if (mode === "manual") {
    if (normalizedMediation === "conditional") {
      normalizedMediation = undefined;
    }

    if (
      Array.isArray(normalizedPublicKey.allowCredentials) &&
      normalizedPublicKey.allowCredentials.length === 0
    ) {
      delete normalizedPublicKey.allowCredentials;
    }
  }

  if (mode === "conditional") {
    normalizedMediation = "conditional";
  }

  if (
    mode === "conditional" &&
    Array.isArray(normalizedPublicKey.allowCredentials) &&
    normalizedPublicKey.allowCredentials.length === 0
  ) {
    delete normalizedPublicKey.allowCredentials;
  }

  return {
    publicKey: normalizedPublicKey,
    mediation: normalizedMediation,
  };
}

function parseRequestOptions(
  optionsJson: unknown,
  mode: RequestOptionsMode = "manual",
): CredentialRequestOptions {
  const parsed = optionsJson as JsonCredentialRequestOptions;
  if (!parsed.publicKey?.challenge) {
    throw new PasskeyClientError(
      "invalid_response",
      "Passkey authentication options were malformed.",
    );
  }

  const { publicKey, mediation } = normalizeRequestOptionsForMode(
    parsed.publicKey,
    parsed.mediation,
    mode,
  );

  const helper = credentialHelpers();
  if (typeof helper.parseRequestOptionsFromJSON === "function") {
    return {
      publicKey: helper.parseRequestOptionsFromJSON(publicKey),
      mediation,
    };
  }

  return {
    publicKey: {
      ...publicKey,
      challenge: base64UrlToBuffer(publicKey.challenge),
      allowCredentials: publicKey.allowCredentials?.map((credential) => ({
        ...credential,
        id: base64UrlToBuffer(credential.id),
      })),
    },
    mediation,
  };
}

function credentialToJson(credential: PublicKeyCredential): unknown {
  const jsonValue = (credential as PublicKeyCredential & { toJSON?: () => unknown }).toJSON?.();
  if (jsonValue) {
    return jsonValue;
  }

  const base = {
    id: credential.id,
    type: credential.type,
    rawId: bufferToBase64Url(credential.rawId),
    authenticatorAttachment: credential.authenticatorAttachment ?? undefined,
    clientExtensionResults: credential.getClientExtensionResults(),
  };

  const response = credential.response;
  if ("attestationObject" in response) {
    const attestation = response as AuthenticatorAttestationResponse;
    return {
      ...base,
      response: {
        clientDataJSON: bufferToBase64Url(attestation.clientDataJSON),
        attestationObject: bufferToBase64Url(attestation.attestationObject),
        transports:
          typeof attestation.getTransports === "function"
            ? attestation.getTransports()
            : undefined,
      },
    };
  }

  const assertion = response as AuthenticatorAssertionResponse;
  return {
    ...base,
    response: {
      clientDataJSON: bufferToBase64Url(assertion.clientDataJSON),
      authenticatorData: bufferToBase64Url(assertion.authenticatorData),
      signature: bufferToBase64Url(assertion.signature),
      userHandle: bufferToBase64Url(assertion.userHandle),
    },
  };
}

async function runMutation<TData, TVariables extends object>(
  client: Client,
  mutation: string,
  variables: TVariables,
  field: keyof TData,
): Promise<TData[keyof TData]> {
  const result = await client.mutation<TData, TVariables>(mutation, variables).toPromise();
  if (result.error || !result.data?.[field]) {
    throw result.error ?? new PasskeyClientError("failed", "Passkey request failed.");
  }

  return result.data[field];
}

function normalizePasskeyError(error: unknown): never {
  if (error instanceof PasskeyClientError) {
    throw error;
  }

  if (
    error instanceof DOMException &&
    (error.name === "NotAllowedError" || error.name === "AbortError")
  ) {
    throw new PasskeyClientError("cancelled", "Passkey request was cancelled.");
  }

  if (error instanceof Error) {
    throw new PasskeyClientError("failed", error.message);
  }

  throw new PasskeyClientError("failed", "Passkey request failed.");
}

export function passkeysSupported(): boolean {
  try {
    ensurePasskeySupport();
    return true;
  } catch {
    return false;
  }
}

export async function conditionalPasskeyMediationSupported(): Promise<boolean> {
  if (!passkeysSupported()) return false;

  try {
    return (
      (await (PublicKeyCredential as ConditionalMediationCredential)
        .isConditionalMediationAvailable?.()) ?? false
    );
  } catch {
    return false;
  }
}

export async function authenticateWithPasskey(
  username?: string,
  persistSession?: boolean,
  client: Client = backendClient,
): Promise<LoginPayload> {
  ensurePasskeySupport();

  try {
    const start = await runMutation<
      {
        webauthnAuthenticateStart: {
          challengeId: string;
          optionsJson: unknown;
        };
      },
      { username?: string | null }
    >(
      client,
      webauthnAuthenticateStartMutation,
      { username: username?.trim() ? username.trim() : null },
      "webauthnAuthenticateStart",
    );

    const credential = await navigator.credentials.get(
      parseRequestOptions(start.optionsJson, "manual"),
    );
    if (!(credential instanceof PublicKeyCredential)) {
      throw new PasskeyClientError("invalid_response", "No passkey assertion was returned.");
    }

    return runMutation<
      { webauthnAuthenticateComplete: LoginPayload },
      { input: { challengeId: string; responseJson: unknown; persistSession?: boolean } }
    >(
      client,
      webauthnAuthenticateCompleteMutation,
      {
        input: {
          challengeId: start.challengeId,
          responseJson: credentialToJson(credential),
          persistSession,
        },
      },
      "webauthnAuthenticateComplete",
    ) as Promise<LoginPayload>;
  } catch (error) {
    normalizePasskeyError(error);
  }
}

export async function reauthenticateAccountSecurityWithPasskey(
  client: Client = backendClient,
): Promise<LoginPayload> {
  ensurePasskeySupport();

  try {
    const start = await runMutation<
      {
        accountSecurityPasskeyStart: {
          challengeId: string;
          optionsJson: unknown;
        };
      },
      Record<string, never>
    >(client, accountSecurityPasskeyStartMutation, {}, "accountSecurityPasskeyStart");
    const credential = await navigator.credentials.get(
      parseRequestOptions(start.optionsJson, "manual"),
    );
    if (!(credential instanceof PublicKeyCredential)) {
      throw new PasskeyClientError("invalid_response", "No passkey assertion was returned.");
    }

    return runMutation<
      { accountSecurityPasskeyComplete: LoginPayload },
      { input: { challengeId: string; responseJson: unknown } }
    >(
      client,
      accountSecurityPasskeyCompleteMutation,
      {
        input: {
          challengeId: start.challengeId,
          responseJson: credentialToJson(credential),
        },
      },
      "accountSecurityPasskeyComplete",
    ) as Promise<LoginPayload>;
  } catch (error) {
    normalizePasskeyError(error);
  }
}

export async function authenticateWithConditionalPasskey(
  persistSession?: boolean,
  signal?: AbortSignal,
  client: Client = backendClient,
  onChallengeStarted?: (expiresAt: string) => void,
): Promise<LoginPayload> {
  ensurePasskeySupport();

  try {
    const start = await runMutation<
      {
        webauthnAuthenticateStart: {
          challengeId: string;
          optionsJson: unknown;
          expiresAt: string;
        };
      },
      { username?: string | null }
    >(
      client,
      webauthnAuthenticateStartMutation,
      { username: null },
      "webauthnAuthenticateStart",
    );
    onChallengeStarted?.(start.expiresAt);
    const options = parseRequestOptions(start.optionsJson, "conditional") as CredentialRequestOptions & {
      signal?: AbortSignal;
    };
    options.signal = signal;
    const credential = await navigator.credentials.get(options);
    if (!(credential instanceof PublicKeyCredential)) {
      throw new PasskeyClientError("cancelled", "No passkey assertion was selected.");
    }
    return runMutation<
      { webauthnAuthenticateComplete: LoginPayload },
      { input: { challengeId: string; responseJson: unknown; persistSession?: boolean } }
    >(
      client,
      webauthnAuthenticateCompleteMutation,
      {
        input: {
          challengeId: start.challengeId,
          responseJson: credentialToJson(credential),
          persistSession,
        },
      },
      "webauthnAuthenticateComplete",
    ) as Promise<LoginPayload>;
  } catch (error) {
    normalizePasskeyError(error);
  }
}

export async function authenticateLoginVerificationPasskey(
  loginChallengeId: string,
  signal?: AbortSignal,
  client: Client = backendClient,
): Promise<LoginPayload> {
  ensurePasskeySupport();

  try {
    const start = await runMutation<
      {
        loginVerificationPasskeyStart: {
          challengeId: string;
          optionsJson: unknown;
        };
      },
      { challengeId: string }
    >(
      client,
      loginVerificationPasskeyStartMutation,
      { challengeId: loginChallengeId },
      "loginVerificationPasskeyStart",
    );
    const options = parseRequestOptions(start.optionsJson, "manual") as CredentialRequestOptions & {
      signal?: AbortSignal;
    };
    options.signal = signal;
    const credential = await navigator.credentials.get(options);
    if (!(credential instanceof PublicKeyCredential)) {
      throw new PasskeyClientError("invalid_response", "No passkey assertion was returned.");
    }
    return runMutation<
      { loginVerificationPasskeyComplete: LoginPayload },
      {
        input: {
          loginChallengeId: string;
          webauthnChallengeId: string;
          responseJson: unknown;
        };
      }
    >(
      client,
      loginVerificationPasskeyCompleteMutation,
      {
        input: {
          loginChallengeId,
          webauthnChallengeId: start.challengeId,
          responseJson: credentialToJson(credential),
        },
      },
      "loginVerificationPasskeyComplete",
    ) as Promise<LoginPayload>;
  } catch (error) {
    normalizePasskeyError(error);
  }
}

export async function registerLoginEnrollmentPasskey(
  client: Client,
): Promise<{ passkey: PasskeySummary; login: LoginPayload }> {
  ensurePasskeySupport();

  try {
    const start = await runMutation<
      {
        webauthnLoginEnrollmentStart: {
          challengeId: string;
          optionsJson: unknown;
        };
      },
      Record<string, never>
    >(client, webauthnLoginEnrollmentStartMutation, {}, "webauthnLoginEnrollmentStart");
    const credential = await navigator.credentials.create(parseCreationOptions(start.optionsJson));
    if (!(credential instanceof PublicKeyCredential)) {
      throw new PasskeyClientError("invalid_response", "No passkey registration was returned.");
    }
    return runMutation<
      {
        webauthnLoginEnrollmentComplete: {
          passkey: PasskeySummary;
          login: LoginPayload;
        };
      },
      {
        input: {
          challengeId: string;
          responseJson: unknown;
          friendlyName: string | null;
        };
      }
    >(
      client,
      webauthnLoginEnrollmentCompleteMutation,
      {
        input: {
          challengeId: start.challengeId,
          responseJson: credentialToJson(credential),
          friendlyName: null,
        },
      },
      "webauthnLoginEnrollmentComplete",
    ) as Promise<{ passkey: PasskeySummary; login: LoginPayload }>;
  } catch (error) {
    normalizePasskeyError(error);
  }
}

export async function registerPasskey(
  client: Client = backendClient,
): Promise<PasskeySummary> {
  ensurePasskeySupport();

  try {
    const start = await runMutation<
      {
        webauthnRegisterStart: {
          challengeId: string;
          optionsJson: unknown;
        };
      },
      Record<string, never>
    >(client, webauthnRegisterStartMutation, {}, "webauthnRegisterStart");

    const credential = await navigator.credentials.create(parseCreationOptions(start.optionsJson));
    if (!(credential instanceof PublicKeyCredential)) {
      throw new PasskeyClientError("invalid_response", "No passkey registration was returned.");
    }

    return runMutation<
      {
        webauthnRegisterComplete: PasskeySummary;
      },
      {
        input: {
          challengeId: string;
          responseJson: unknown;
          friendlyName: string | null;
        };
      }
    >(
      client,
      webauthnRegisterCompleteMutation,
      {
        input: {
          challengeId: start.challengeId,
          responseJson: credentialToJson(credential),
          friendlyName: null,
        },
      },
      "webauthnRegisterComplete",
    );
  } catch (error) {
    normalizePasskeyError(error);
  }
}
