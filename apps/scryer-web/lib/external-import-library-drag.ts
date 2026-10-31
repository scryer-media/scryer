export type ImportLibraryDropState = "idle" | "compatible" | "incompatible";

export function shouldEnableNativeImportDrag(
  isNarrowViewport: boolean,
  hasCoarsePointer: boolean,
): boolean {
  return !isNarrowViewport && !hasCoarsePointer;
}

export function importLibraryDropState(
  isDragging: boolean,
  isCompatible: boolean,
): ImportLibraryDropState {
  if (!isDragging) return "idle";
  return isCompatible ? "compatible" : "incompatible";
}
