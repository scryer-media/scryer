import type { Translate } from "@/components/root/types";
import type { SetGlobalStatus } from "@/lib/context/global-status-context";
import { userFacingGraphQlErrorMessage } from "../graphql/error-message.ts";
import { pluginErrorMessage } from "./plugin-error-message.ts";

export class ReportedConnectionFeedbackError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ReportedConnectionFeedbackError";
  }
}

type RunConnectionFeedbackOptions = {
  setGlobalStatus: SetGlobalStatus;
  startMessage?: string;
  successMessage: string;
  failureFallbackMessage: string;
  announceSuccess?: boolean;
  /**
   * Translates a failure the plugin behind the connection named a reason for.
   * Without it every failure shows the server's sentence.
   */
  t?: Translate;
  run: () => Promise<string | void | null | undefined>;
};

export async function runConnectionFeedback({
  setGlobalStatus,
  startMessage,
  successMessage,
  failureFallbackMessage,
  announceSuccess = true,
  t,
  run,
}: RunConnectionFeedbackOptions): Promise<void> {
  if (startMessage) {
    setGlobalStatus(startMessage);
  }

  try {
    const nextSuccessMessage = (await run()) ?? successMessage;
    if (announceSuccess) {
      setGlobalStatus(nextSuccessMessage, { level: "SUCCESS" });
    }
  } catch (error) {
    // urql prefixes `CombinedError.message` with "[GraphQL] " and the server
    // prefixes validation errors with "validation: ", so the raw message reads
    // as machinery. The shared helper strips both and keeps the reference id on
    // errors the server really did mask.
    const message = t
      ? pluginErrorMessage(error, t, failureFallbackMessage)
      : userFacingGraphQlErrorMessage(error, failureFallbackMessage, t);
    setGlobalStatus(message, { level: "ERROR" });
    throw new ReportedConnectionFeedbackError(message);
  }
}

export function isReportedConnectionFeedbackError(
  error: unknown,
): error is ReportedConnectionFeedbackError {
  return error instanceof ReportedConnectionFeedbackError;
}
