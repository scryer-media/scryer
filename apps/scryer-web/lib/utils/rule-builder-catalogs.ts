import type { Client } from "urql";
import {
  qualityProfileOptionsQuery,
  ruleBuilderListOptionsQuery,
  ruleBuilderTagLabelsQuery,
  ruleBuilderUserOptionsQuery,
} from "@/lib/graphql/queries";
import type { RuleBuilderOptionValue } from "@/lib/utils/rule-builder";

/// Catalogs the rule builder's pickers can load, shared by every rule page.
/// Each reads only the names its picker shows.

export async function loadQualityProfileNames(
  client: Client,
): Promise<string[]> {
  const { data, error } = await client
    .query(qualityProfileOptionsQuery, {})
    .toPromise();
  if (error) throw error;
  const profiles: { name: string }[] =
    data?.qualityProfileSettings?.profiles ?? [];
  return profiles.map((profile) => profile.name);
}

/// Quality profiles by id, for a condition on a fact that holds the id.
export async function loadQualityProfileIdOptions(
  client: Client,
): Promise<RuleBuilderOptionValue[]> {
  const { data, error } = await client
    .query(qualityProfileOptionsQuery, {})
    .toPromise();
  if (error) throw error;
  const profiles: { id: string; name: string }[] =
    data?.qualityProfileSettings?.profiles ?? [];
  return profiles.map((profile) => ({
    value: profile.id,
    label: profile.name,
  }));
}

export async function loadUserIdOptions(
  client: Client,
): Promise<RuleBuilderOptionValue[]> {
  const { data, error } = await client
    .query(ruleBuilderUserOptionsQuery, {})
    .toPromise();
  if (error) throw error;
  const users: { id: string; username: string }[] = data?.users ?? [];
  return users.map((user) => ({ value: user.id, label: user.username }));
}

export async function loadListIdOptions(
  client: Client,
): Promise<RuleBuilderOptionValue[]> {
  const { data, error } = await client
    .query(ruleBuilderListOptionsQuery, {})
    .toPromise();
  if (error) throw error;
  const lists: { id: string; name: string }[] = data?.listSubscriptions ?? [];
  return lists.map((list) => ({ value: list.id, label: list.name }));
}

export async function loadTitleTagLabels(client: Client): Promise<string[]> {
  const { data, error } = await client
    .query(ruleBuilderTagLabelsQuery, {})
    .toPromise();
  if (error) throw error;
  const tags: { label: string }[] = data?.titleTagDefinitions ?? [];
  return tags.map((tag) => tag.label);
}
