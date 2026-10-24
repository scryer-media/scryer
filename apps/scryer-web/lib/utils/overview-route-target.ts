import type { ResolvedTitleOverviewTarget } from "../title-overview-loader.ts";

/** Reuse a clicked title only when it belongs to the route now being shown. */
export function overviewTargetFromNavigation(
  state: unknown,
  route: { view: string; librarySlug: string | null; slug: string | null },
): ResolvedTitleOverviewTarget | null {
  if (!state || typeof state !== "object" || !("scryerOverviewTarget" in state)) return null;
  const target = state.scryerOverviewTarget;
  if (!target || typeof target !== "object") return null;
  if (!("view" in target) || target.view !== route.view ||
      !("slug" in target) || target.slug !== route.slug ||
      !("librarySlug" in target) || target.librarySlug !== route.librarySlug ||
      !route.slug || !route.librarySlug ||
      !("id" in target) || typeof target.id !== "string" || !target.id.trim()) return null;
  return {
    id: target.id.trim(),
    slug: route.slug,
    librarySlug: route.librarySlug,
    libraryId: "libraryId" in target && typeof target.libraryId === "string" ? target.libraryId : null,
  };
}
