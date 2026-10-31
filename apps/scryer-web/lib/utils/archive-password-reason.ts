/** Compatibility for history written before reasons were separate fields. */
export function legacyArchivePasswordReason(
  reason: string | null | undefined,
): boolean {
  return Boolean(
    reason?.startsWith("ARCHIVE_PASSWORD_REQUIRED:") ||
    reason?.startsWith("ARCHIVE_PASSWORD_OR_CORRUPTION:") ||
    reason === "archive_password_required" ||
    reason === "archive_password_or_corruption",
  );
}

/** A typed condition wins over compatibility text, including unknown codes. */
export function isArchivePasswordFailure(
  reasonCode: string | null | undefined,
  legacyReason?: string | null,
): boolean {
  if (reasonCode)
    return (
      reasonCode === "ARCHIVE_PASSWORD_REQUIRED" ||
      reasonCode === "ARCHIVE_PASSWORD_OR_CORRUPTION"
    );
  return legacyArchivePasswordReason(legacyReason);
}
