import type { Translate } from "@/components/root/types";
import { metadataFacetGraphqlValue } from "./facets/registry.ts";

type FixMatchCompletionArgs = {
  warnings: string[];
  refreshTitleDetail: () => Promise<void>;
  setGlobalStatus: (message: string) => void;
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
    setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    return;
  }

  if (warnings.length > 0) {
    setGlobalStatus(warnings.join(" "));
    return;
  }

  setGlobalStatus(
    t("status.titleMatchUpdated", {
      name: titleName?.trim() || t("title.fixMatchUnnamed"),
    }),
  );
}
