const MEDIA_SERVER_AVATAR_PREFIX = "/api/media-server-avatars/";

export function isProtectedMediaServerAvatarUrl(
  value: string,
  origin: string,
): boolean {
  try {
    const base = new URL(origin);
    const url = new URL(value, base);
    return (
      url.origin === base.origin &&
      url.pathname.startsWith(MEDIA_SERVER_AVATAR_PREFIX)
    );
  } catch {
    return false;
  }
}

export async function fetchProtectedMediaServerAvatar(
  value: string,
  origin: string,
  token: string,
  signal: AbortSignal,
  fetchImage: typeof fetch,
): Promise<Blob> {
  if (!isProtectedMediaServerAvatarUrl(value, origin)) {
    throw new TypeError(
      "Refusing to send credentials to an untrusted avatar URL",
    );
  }

  const response = await fetchImage(value, {
    headers: { Authorization: `Bearer ${token}` },
    signal,
  });
  if (!response.ok) {
    throw new TypeError("Failed to load authenticated avatar");
  }
  const blob = await response.blob();
  if (!blob.type.toLowerCase().startsWith("image/")) {
    throw new TypeError("Authenticated avatar response was not an image");
  }
  return blob;
}
