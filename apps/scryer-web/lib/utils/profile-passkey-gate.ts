export function shouldLoadProfilePasskeys({
  authLoading,
  effectiveFormLoginEnabled,
  passkeyEnabled,
  userId,
}: {
  authLoading: boolean;
  effectiveFormLoginEnabled: boolean | undefined;
  passkeyEnabled: boolean;
  userId: string | null | undefined;
}): boolean {
  return (
    !authLoading &&
    effectiveFormLoginEnabled === true &&
    passkeyEnabled &&
    Boolean(userId)
  );
}
