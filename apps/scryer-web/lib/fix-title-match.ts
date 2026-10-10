import type { Translate } from "@/components/root/types";
import type { SetGlobalStatus } from "@/lib/context/global-status-context";
import { metadataFacetGraphqlValue } from "./facets/registry.ts";

type FixMatchCompletionArgs = {
  warnings: string[];
  refreshTitleDetail: () => Promise<void>;
  setGlobalStatus: SetGlobalStatus;
  t: Translate;
  titleName?: string | null;
};

type FixMatchTitleIdentity = {
  id: string;
  facet: string;
};

export function fixTitleMatchDialogIdentity(
  title: FixMatchTitleIdentity | null | undefined,
): string | null {
  return title
    ? JSON.stringify([metadataFacetGraphqlValue(title.facet), title.id])
    : null;
}

export function buildFixTitleMatchSearchVariables(
  query: string,
  facet: string | null | undefined,
) {
  return {
    query,
    type: metadataFacetGraphqlValue(facet),
    limit: 8,
  };
}

export type FixTitleMatchTarget = { smgId?: number; tvdbId?: string };

/**
 * The identity a chosen search result rematches to, for every facet: its SMG
 * title id and/or its TVDB id. The server takes either one alone (for series
 * and anime a TVDB id wins when both are sent). Null when the result carries
 * neither, which is when Apply stays disabled.
 */
export function fixTitleMatchTarget(
  result: { smgId?: number | null; tvdbId?: string | null } | null | undefined,
): FixTitleMatchTarget | null {
  if (!result) return null;
  const target: FixTitleMatchTarget = {};
  if (result.smgId != null && result.smgId > 0) target.smgId = result.smgId;
  const tvdbId = result.tvdbId?.trim();
  if (tvdbId) target.tvdbId = tvdbId;
  return target.smgId === undefined && target.tvdbId === undefined ? null : target;
}

export async function handleFixTitleMatchComplete({
  warnings,
  refreshTitleDetail,
  setGlobalStatus,
  t,
  titleName,
}: FixMatchCompletionArgs) {
  try {
    await refreshTitleDetail();
  } catch (error) {
    setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"), { level: "ERROR" });
    return;
  }

  if (warnings.length > 0) {
    setGlobalStatus(warnings.join(" "), { level: "WARNING" });
    return;
  }

  setGlobalStatus(
    t("status.titleMatchUpdated", {
      name: titleName?.trim() || t("title.fixMatchUnnamed"),
    }),
    { level: "SUCCESS" },
  );
}
