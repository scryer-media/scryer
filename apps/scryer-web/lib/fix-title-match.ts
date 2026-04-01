import type { Translate } from "@/components/root/types";

type FixMatchCompletionArgs = {
  warnings: string[];
  refreshTitleDetail: () => Promise<void>;
  setGlobalStatus: (message: string) => void;
  t: Translate;
  titleName?: string | null;
};

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
