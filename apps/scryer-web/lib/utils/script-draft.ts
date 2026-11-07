import type { PostProcessingScriptDraft } from "@/lib/types/scripts";

/**
 * Inline code and a file path share `scriptContent` on the wire, so a draft
 * parks the one not in use. Neither ever appears in the other's control, and
 * switching back restores what was there.
 */
export function withScriptType(
  draft: PostProcessingScriptDraft,
  scriptType: string,
): PostProcessingScriptDraft {
  if (scriptType === draft.scriptType) return draft;
  return {
    ...draft,
    scriptType,
    scriptContent: draft.parkedScriptContent,
    parkedScriptContent: draft.scriptContent,
  };
}
