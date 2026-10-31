import * as React from "react";

import { useSessionUser } from "@/lib/hooks/use-auth";
import type { AuthUser } from "@/lib/hooks/use-auth";
import {
  hasLibraryPermission,
  LIBRARY_PERMISSIONS,
} from "@/lib/utils/permissions";

/**
 * Whether the viewer may change one title: its settings, files, searches and
 * blocklist. Every title overview, the movie panel and the series and anime
 * page alike, gates its actions on this, so the check cannot differ between
 * them. It is always judged against the title's own library, never against
 * whichever library the viewer happens to manage.
 */
export function canManageOverviewTitle(
  user: AuthUser | null,
  libraryId: string | null | undefined,
): boolean {
  return hasLibraryPermission(
    user,
    libraryId,
    LIBRARY_PERMISSIONS.manageTitles,
  );
}

export function useCanManageOverviewTitle(
  libraryId: string | null | undefined,
): boolean {
  const user = useSessionUser();
  return React.useMemo(
    () => canManageOverviewTitle(user, libraryId),
    [libraryId, user],
  );
}
