import type { ConfigFieldDef } from "../types/indexers.ts";

export type SplitConfigFields = {
  /// Shown up front.
  standard: ConfigFieldDef[];
  /// Shown behind the form's advanced disclosure.
  advanced: ConfigFieldDef[];
};

/// Split a provider's fields into what a form shows up front and what it keeps
/// behind a disclosure.
///
/// The plugin decides which is which; a form that hard-codes the split ends up
/// disagreeing with the next provider that ships.
export function splitAdvancedConfigFields(
  fields: ConfigFieldDef[],
): SplitConfigFields {
  const standard: ConfigFieldDef[] = [];
  const advanced: ConfigFieldDef[] = [];
  for (const field of fields) {
    (field.advanced ? advanced : standard).push(field);
  }
  return { standard, advanced };
}
