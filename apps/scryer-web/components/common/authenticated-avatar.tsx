import * as React from "react";

import { scryerFetch } from "@/lib/graphql/urql-client";
import { getAuthToken } from "@/lib/hooks/use-auth";
import {
  fetchProtectedMediaServerAvatar,
  isProtectedMediaServerAvatarUrl,
} from "@/lib/utils/authenticated-avatar";

export function useAuthenticatedAvatarSource(
  avatarUrl: string | null | undefined,
): string | null {
  const protectedAvatar =
    typeof window !== "undefined" &&
    Boolean(
      avatarUrl &&
      isProtectedMediaServerAvatarUrl(avatarUrl, window.location.origin),
    );
  const [loaded, setLoaded] = React.useState<{
    requestedUrl: string;
    objectUrl: string;
  } | null>(null);

  React.useEffect(() => {
    if (!protectedAvatar || !avatarUrl) return;
    const token = getAuthToken();
    if (!token) return;

    const controller = new AbortController();
    let objectUrl: string | null = null;
    void fetchProtectedMediaServerAvatar(
      avatarUrl,
      window.location.origin,
      token,
      controller.signal,
      scryerFetch,
    )
      .then((blob) => {
        if (controller.signal.aborted) return;
        objectUrl = window.URL.createObjectURL(blob);
        setLoaded({ requestedUrl: avatarUrl, objectUrl });
      })
      .catch(() => {
        // The initials fallback remains visible for auth, network, and image failures.
      });

    return () => {
      controller.abort();
      if (objectUrl) {
        window.URL.revokeObjectURL(objectUrl);
        setLoaded((current) =>
          current?.objectUrl === objectUrl ? null : current,
        );
      }
    };
  }, [avatarUrl, protectedAvatar]);

  if (!protectedAvatar) return avatarUrl ?? null;
  return loaded && loaded.requestedUrl === avatarUrl ? loaded.objectUrl : null;
}

export function AuthenticatedAvatar({
  avatarUrl,
  label,
  imageClassName,
  fallbackClassName,
}: {
  avatarUrl: string | null | undefined;
  label: string;
  imageClassName: string;
  fallbackClassName: string;
}) {
  const source = useAuthenticatedAvatarSource(avatarUrl);
  const [failedSource, setFailedSource] = React.useState<string | null>(null);

  React.useEffect(() => {
    setFailedSource(null);
  }, [source]);

  return source && failedSource !== source ? (
    <img
      src={source}
      alt=""
      className={imageClassName}
      loading="lazy"
      onError={() => setFailedSource(source)}
    />
  ) : (
    <span aria-hidden="true" className={fallbackClassName}>
      {label.trim().slice(0, 1).toUpperCase() || "?"}
    </span>
  );
}
