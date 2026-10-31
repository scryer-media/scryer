import type { CombinedError } from "urql";

const DOWNLOAD_FEEDBACK_TIMEOUT_CODE = "DOWNLOAD_FEEDBACK_TIMEOUT";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function graphQlErrorCode(
  error: CombinedError["graphQLErrors"][number],
): string | null {
  const { extensions } = error;
  if (!isRecord(extensions)) {
    return null;
  }

  return typeof extensions.code === "string" ? extensions.code : null;
}

function graphQlErrorAlias(
  error: CombinedError["graphQLErrors"][number],
): string | null {
  if (!Array.isArray(error.path) || error.path.length === 0) {
    return null;
  }

  return typeof error.path[0] === "string" ? error.path[0] : null;
}

export function extractDownloadFeedbackWarning(
  graphQlErrors: readonly CombinedError["graphQLErrors"][number][],
  allowedAliases: readonly string[],
): string | null {
  if (graphQlErrors.length === 0) {
    return null;
  }

  const allowedAliasSet = new Set(allowedAliases);
  let warningMessage: string | null = null;

  for (const graphQlError of graphQlErrors) {
    if (graphQlErrorCode(graphQlError) !== DOWNLOAD_FEEDBACK_TIMEOUT_CODE) {
      return null;
    }

    const alias = graphQlErrorAlias(graphQlError);
    if (!alias || !allowedAliasSet.has(alias)) {
      return null;
    }

    warningMessage ??= graphQlError.message;
  }

  return warningMessage;
}
