import type { CatalogDiscoveryGroup } from "../types/discovery.ts";

export const MOVIE_POPULAR_GROUP_ID = "catalog_movie_popular";
const POPULAR_SECTIONS = new Set([
  "popular_right_now",
  "popular_movies",
  "trending_now",
]);

export function mergeMoviePopularGroups(
  groups: CatalogDiscoveryGroup[],
): CatalogDiscoveryGroup[] {
  const result: CatalogDiscoveryGroup[] = [];
  let popular: CatalogDiscoveryGroup | undefined;
  const seen = new Set<string>();
  for (const group of groups) {
    const sectionId =
      group.kind === "PUBLIC_TOP"
        ? group.labelKey
        : group.id.replace(/^public_section_/, "");
    if (
      group.surface !== "PUBLIC" ||
      (group.kind !== "PUBLIC_TOP" && group.kind !== "PUBLIC_SECTION") ||
      !sectionId ||
      !POPULAR_SECTIONS.has(sectionId)
    ) {
      result.push(group);
      continue;
    }
    if (!popular) {
      popular = {
        ...group,
        id: MOVIE_POPULAR_GROUP_ID,
        items: [],
        totalCount: 0,
      };
      result.push(popular);
    }
    for (const item of group.items) {
      const key = `${item.targetKind}:${item.targetKey}`;
      if (!seen.has(key)) {
        seen.add(key);
        popular.items.push(item);
      }
    }
    popular.totalCount = popular.items.length;
  }
  return result;
}
