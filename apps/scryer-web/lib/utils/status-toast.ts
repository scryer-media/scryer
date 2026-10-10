export type StatusToastKind = "SUCCESS" | "ERROR" | "WARNING";

/**
 * The level a status toasts at: the one its caller stated, or none.
 *
 * The wording is never consulted. The code raising a status knows whether it
 * is reporting a failure, a success or a caveat; the sentence does not, in
 * English or in any other locale, and a server message can say anything. A
 * status raised without a level is deliberately silent (progress notes, form
 * hints). An empty status never toasts.
 */
export function resolveStatusToastLevel(
  message: string,
  options?: { level?: StatusToastKind },
): StatusToastKind | null {
  if (!message.trim()) {
    return null;
  }
  return options?.level ?? null;
}
