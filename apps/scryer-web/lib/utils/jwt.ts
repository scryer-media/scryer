export type JwtPayload = {
  sub: string;
  exp: number;
  iat: number;
  iss: string;
  username: string;
  entitlements: string[];
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
