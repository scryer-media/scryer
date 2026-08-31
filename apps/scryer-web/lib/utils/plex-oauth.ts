const PLEX_CLIENT_IDENTIFIER_KEY = "scryer_plex_client_identifier";
const PLEX_PIN_TIMEOUT_MS = 15 * 60 * 1000;
const PLEX_PIN_POLL_INTERVAL_MS = 2_000;
const PLEX_PIN_RATE_LIMIT_BACKOFF_MS = 5_000;
const PLEX_PIN_MAX_BACKOFF_MS = 30_000;

type PlexPinResponse = {
  id?: number;
  code?: string;
  authToken?: string | null;
  auth_token?: string | null;
  expiresAt?: string | null;
  expires_at?: string | null;
};

function getPlexClientIdentifier(): string {
  const existing = window.localStorage.getItem(PLEX_CLIENT_IDENTIFIER_KEY);
  if (existing) {
    return existing;
  }

  const generated = createPlexClientIdentifier();
  window.localStorage.setItem(PLEX_CLIENT_IDENTIFIER_KEY, generated);
  return generated;
}

function createPlexClientIdentifier(): string {
  if (typeof window.crypto?.randomUUID === "function") {
    return window.crypto.randomUUID();
  }
  if (typeof window.crypto?.getRandomValues !== "function") {
    throw new Error("Secure browser randomness is required for Plex sign-in.");
  }
  const bytes = new Uint8Array(16);
  window.crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function plexHeaders(clientIdentifier: string): Record<string, string> {
  const userAgent = window.navigator.userAgent || "Browser";
  const platform = window.navigator.platform || "Browser";
  return {
    Accept: "application/json",
    "X-Plex-Product": "Scryer",
    "X-Plex-Version": "Plex OAuth",
    "X-Plex-Client-Identifier": clientIdentifier,
    "X-Plex-Model": "Plex OAuth",
    "X-Plex-Platform": platform,
    "X-Plex-Platform-Version": userAgent,
    "X-Plex-Device": platform,
    "X-Plex-Device-Name": `${platform} (Scryer)`,
    "X-Plex-Device-Screen-Resolution": `${window.screen.width}x${window.screen.height}`,
    "X-Plex-Language": window.navigator.language || "en",
  };
}

export function isolatePlexPopup(popup: Window): void {
  try {
    popup.opener = null;
  } catch {
    popup.close();
    throw new Error("Unable to isolate the Plex login window.");
  }
  if (popup.opener !== null) {
    popup.close();
    throw new Error("Unable to isolate the Plex login window.");
  }
}

function openPlexPopup(): Window {
  const width = 600;
  const height = 700;
  const left = window.screenX + Math.max(0, (window.outerWidth - width) / 2);
  const top = window.screenY + Math.max(0, (window.outerHeight - height) / 2);
  const popup = window.open(
    "/login/plex/loading",
    "Plex Auth",
    `scrollbars=yes,width=${width},height=${height},top=${top},left=${left}`,
  );
  if (!popup) {
    throw new Error("Unable to open the Plex login window. Allow popups and try again.");
  }
  isolatePlexPopup(popup);
  popup.focus();
  return popup;
}

function encodeParams(params: Record<string, string>): string {
  return Object.entries(params)
    .map(([key, value]) => [key, value].map(encodeURIComponent).join("="))
    .join("&");
}

async function createPlexPin(headers: Record<string, string>): Promise<{ id: number; code: string }> {
  const response = await fetch("https://plex.tv/api/v2/pins?strong=true", {
    method: "POST",
    headers,
  });
  if (!response.ok) {
    throw new Error("Unable to create a Plex sign-in PIN.");
  }
  const body = (await response.json()) as PlexPinResponse;
  if (typeof body.id !== "number" || !body.code) {
    throw new Error("Plex did not return a usable sign-in PIN.");
  }
  return { id: body.id, code: body.code };
}

async function pollPlexPin(
  pinId: number,
  headers: Record<string, string>,
  deadline: number,
): Promise<string> {
  let nextDelayMs = PLEX_PIN_POLL_INTERVAL_MS;
  while (Date.now() < deadline) {
    const response = await fetch(`https://plex.tv/api/v2/pins/${pinId}`, {
      headers,
    });
    if (response.status === 429) {
      const retryAfterMs = retryAfterHeaderToMs(response.headers.get("Retry-After"));
      nextDelayMs = Math.min(
        Math.max(retryAfterMs ?? nextDelayMs * 2, PLEX_PIN_RATE_LIMIT_BACKOFF_MS),
        PLEX_PIN_MAX_BACKOFF_MS,
      );
      await waitForPlexPoll(nextDelayMs, deadline);
      continue;
    }
    if (!response.ok) {
      throw new Error("Unable to check the Plex sign-in PIN.");
    }
    nextDelayMs = PLEX_PIN_POLL_INTERVAL_MS;
    const body = (await response.json()) as PlexPinResponse;
    const authToken = body.authToken ?? body.auth_token;
    if (authToken) {
      return authToken;
    }
    const expiresAt = body.expiresAt ?? body.expires_at;
    if (expiresAt && Date.now() >= Date.parse(expiresAt)) {
      break;
    }
    await waitForPlexPoll(nextDelayMs, deadline);
  }
  throw new Error("Plex sign-in expired before it completed.");
}

function retryAfterHeaderToMs(value: string | null): number | null {
  if (!value) return null;
  const seconds = Number(value);
  if (Number.isFinite(seconds) && seconds >= 0) {
    return seconds * 1000;
  }
  const timestamp = Date.parse(value);
  if (!Number.isNaN(timestamp)) {
    return Math.max(0, timestamp - Date.now());
  }
  return null;
}

async function waitForPlexPoll(delayMs: number, deadline: number) {
  const remainingMs = Math.max(0, deadline - Date.now());
  await new Promise((resolve) => window.setTimeout(resolve, Math.min(delayMs, remainingMs)));
}

export async function authenticateWithPlexPin(): Promise<string> {
  const popup = openPlexPopup();
  try {
    const clientIdentifier = getPlexClientIdentifier();
    const headers = plexHeaders(clientIdentifier);
    const pin = await createPlexPin(headers);
    popup.location.href = `https://app.plex.tv/auth/#!?${encodeParams({
      clientID: clientIdentifier,
      "context[device][product]": headers["X-Plex-Product"],
      "context[device][version]": headers["X-Plex-Version"],
      "context[device][platform]": headers["X-Plex-Platform"],
      "context[device][platformVersion]": headers["X-Plex-Platform-Version"],
      "context[device][device]": headers["X-Plex-Device"],
      "context[device][deviceName]": headers["X-Plex-Device-Name"],
      "context[device][model]": headers["X-Plex-Model"],
      "context[device][screenResolution]": headers["X-Plex-Device-Screen-Resolution"],
      "context[device][layout]": "desktop",
      code: pin.code,
    })}`;
    return await pollPlexPin(pin.id, headers, Date.now() + PLEX_PIN_TIMEOUT_MS);
  } finally {
    popup.close();
  }
}
