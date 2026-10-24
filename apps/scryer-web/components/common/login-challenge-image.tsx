import { RefreshCw } from "lucide-react";
import { LoadingMark } from "@/components/common/loading-mark";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import type { LoginChallengeStatus } from "@/lib/types";

/**
 * The sign-in image an indexer's login asks for, shown beside the field that
 * takes the answer. The image comes from the server for the current draft and
 * is never stored; asking again fetches a fresh one.
 */
export function LoginChallengeImage({
  idPrefix,
  status,
  onRefresh,
}: {
  idPrefix: string;
  status: LoginChallengeStatus;
  onRefresh?: () => void;
}) {
  const t = useTranslate();
  if (status.state === "idle" || status.state === "none") {
    return null;
  }

  const refresh = onRefresh ? (
    <Button
      id={`${idPrefix}-refresh`}
      type="button"
      variant="ghost"
      size="sm"
      disabled={status.state === "loading"}
      onClick={onRefresh}
    >
      <RefreshCw className="h-4 w-4" />
      {t("settings.indexerLoginChallengeRefresh")}
    </Button>
  ) : null;

  if (status.state === "loading") {
    return (
      <p
        id={`${idPrefix}-loading`}
        role="status"
        className="flex items-center gap-2 text-xs text-muted-foreground"
      >
        <LoadingMark className="h-4 w-4" />
        {t("settings.indexerLoginChallengeLoading")}
      </p>
    );
  }

  if (status.state === "error") {
    return (
      <div className="flex flex-wrap items-center gap-2">
        <p
          id={`${idPrefix}-error`}
          role="alert"
          className="text-xs text-[var(--scry-danger-text-soft)]"
        >
          {t("settings.indexerLoginChallengeError", { error: status.message })}
        </p>
        {refresh}
      </div>
    );
  }

  return (
    <div className="flex flex-wrap items-center gap-2">
      <img
        id={`${idPrefix}-image`}
        src={`data:${status.contentType};base64,${status.imageData}`}
        alt={t("settings.indexerLoginChallengeImageAlt")}
        className="max-h-32 max-w-full rounded-md border border-border bg-white"
      />
      {refresh}
    </div>
  );
}
