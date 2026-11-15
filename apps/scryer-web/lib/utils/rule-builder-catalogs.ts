import type { Client } from "urql";
import {
  qualityProfileOptionsQuery,
  ruleBuilderTagLabelsQuery,
} from "@/lib/graphql/queries";

/// Catalogs the rule builder's pickers can load, shared by every rule page.
/// Each reads only the names its picker shows.

export async function loadQualityProfileNames(client: Client): Promise<string[]> {
  const { data, error } = await client
    .query(qualityProfileOptionsQuery, {})
    .toPromise();
  if (error) throw error;
  const profiles: { name: string }[] =
    data?.qualityProfileSettings?.profiles ?? [];
  return profiles.map((profile) => profile.name);
}

export async function loadTitleTagLabels(client: Client): Promise<string[]> {
  const { data, error } = await client
    .query(ruleBuilderTagLabelsQuery, {})
    .toPromise();
  if (error) throw error;
  const tags: { label: string }[] = data?.titleTagDefinitions ?? [];
  return tags.map((tag) => tag.label);
}
