import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { restartForNetworkChangeMutation } from "@/lib/graphql/mutations";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { useTranslate } from "@/lib/context/translate-context";

// The restart is scheduled after the response, and the old listener answers
// until it has shut down, so an immediate probe would reach the process that
// is on its way out.
const RESTART_PROBE_DELAY_MS = 3_000;
const RESTART_PROBE_INTERVAL_MS = 1_500;
/** After this long the page says Scryer may now be at another address. */
const RESTART_PROBE_TIMEOUT_MS = 45_000;

export type NetworkRestartPhase =
  "idle" | "requesting" | "restarting" | "unreachable";

function healthUrl(): string {
  const base = getRuntimeBasePath();
  return base === "/" ? "/health" : `${base}/health`;
}

/** Whether Scryer answered; reloads the page when it did. */
async function reloadOnceAnswering(cancelled: () => boolean): Promise<boolean> {
  try {
    const response = await fetch(healthUrl(), { cache: "no-store" });
    if (!cancelled() && response.ok) {
      window.location.reload();
      return true;
    }
  } catch {
    // Expected for as long as Scryer is down.
  }
  return false;
}

/** Restart Scryer so a saved network choice applies, then reload once it answers. */
export function useNetworkRestart() {
  const client = useClient();
  const t = useTranslate();
  const [phase, setPhase] = useState<NetworkRestartPhase>("idle");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (phase !== "restarting") return;
    let cancelled = false;
    let timer: number | undefined;
    const deadline = Date.now() + RESTART_PROBE_TIMEOUT_MS;
    const probe = async () => {
      if (cancelled) return;
      if (await reloadOnceAnswering(() => cancelled)) return;
      if (cancelled) return;
      if (Date.now() >= deadline) {
        setPhase("unreachable");
        return;
      }
      timer = window.setTimeout(probe, RESTART_PROBE_INTERVAL_MS);
    };
    timer = window.setTimeout(probe, RESTART_PROBE_DELAY_MS);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [phase]);

  const restart = useCallback(async () => {
    setError(null);
    setPhase("requesting");
    try {
      const result = await client
        .mutation<{ restartForNetworkChange: boolean }>(
          restartForNetworkChangeMutation,
          {},
        )
        .toPromise();
      if (result.error) throw result.error;
      setPhase("restarting");
    } catch (cause) {
      setError(
        userFacingGraphQlErrorMessage(
          cause,
          t("settings.networkRestartFailed"),
          t,
        ),
      );
      setPhase("idle");
    }
  }, [client, t]);

  return { phase, error, restart };
}
