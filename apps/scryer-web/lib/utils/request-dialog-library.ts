/**
 * The library a request dialog selects: the current choice while the list
 * still holds it, otherwise the default library, otherwise the first one.
 */
export function requestDialogLibraryId(
  libraries: ReadonlyArray<{ id: string; isDefault: boolean }>,
  currentId: string,
): string {
  if (currentId && libraries.some((library) => library.id === currentId)) {
    return currentId;
  }
  return (
    libraries.find((library) => library.isDefault)?.id || libraries[0]?.id || ""
  );
}
