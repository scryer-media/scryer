export type JwtPayload = {
  sub: string;
  exp: number;
  iat: number;
  iss: string;
  username: string;
  appPermissions?: string[];
  libraryPermissions?: {
    libraryId: string;
    permissions: string[];
  }[];
  mfaVerifiedUntil?: number | string | null;
  securityActionVerifiedUntil?: number | string | null;
  mfaStepUpVerifiedUntil?: number | string | null;
  authScope?: "full" | "mfa_enrollment" | "password_change_required";
};

/** Decode a JWT payload without signature verification. Returns null if malformed. */
export function decodeJwtPayload(token: string): JwtPayload | null {
  try {
    const parts = token.split(".");
    if (parts.length !== 3) return null;
    const base64 = parts[1].replace(/-/g, "+").replace(/_/g, "/");
    return JSON.parse(atob(base64)) as JwtPayload;
  } catch {
    return null;
  }
}

/** Check if a decoded JWT is expired (with optional clock skew tolerance). */
export function isTokenExpired(payload: JwtPayload, skewSeconds = 30): boolean {
  return payload.exp * 1000 < Date.now() + skewSeconds * 1000;
}

export function jwtDateClaimToMillis(value: unknown): number | null {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value * 1000 : null;
  }

  if (typeof value !== "string") {
    return null;
  }

  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }

  const numeric = Number(trimmed);
  if (Number.isFinite(numeric) && /^\d+(?:\.\d+)?$/.test(trimmed)) {
    return numeric * 1000;
  }

  const parsed = Date.parse(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}
