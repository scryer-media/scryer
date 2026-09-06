import { userFacingGraphQlErrorMessage } from "../graphql/error-message.ts";

type SetGlobalStatus = (status: string) => void;

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
  run: () => Promise<string | void | null | undefined>;
};

export async function runConnectionFeedback({
  setGlobalStatus,
  startMessage,
  successMessage,
  failureFallbackMessage,
  announceSuccess = true,
  run,
}: RunConnectionFeedbackOptions): Promise<void> {
  if (startMessage) {
    setGlobalStatus(startMessage);
  }

  try {
    const nextSuccessMessage = (await run()) ?? successMessage;
    if (announceSuccess) {
      setGlobalStatus(nextSuccessMessage);
    }
  } catch (error) {
    // urql prefixes `CombinedError.message` with "[GraphQL] " and the server
    // prefixes validation errors with "validation: ", so the raw message reads
    // as machinery. The shared helper strips both and keeps the reference id on
    // errors the server really did mask.
    const message = userFacingGraphQlErrorMessage(error, failureFallbackMessage);
    setGlobalStatus(message);
    throw new ReportedConnectionFeedbackError(message);
  }
}

export function isReportedConnectionFeedbackError(
  error: unknown,
): error is ReportedConnectionFeedbackError {
  return error instanceof ReportedConnectionFeedbackError;
}
