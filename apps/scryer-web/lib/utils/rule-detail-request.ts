export type RuleDetailEditorState = { isOpen: boolean; isDirty: boolean };

export type RuleDetailRequestResult<T> =
  | { type: "ignore" }
  | { type: "open"; detail: T }
  | { type: "confirm"; detail: T };

export async function resolveRuleDetailRequest<T>(
  request: number,
  currentRequest: () => number,
  load: () => Promise<T | null>,
  editorState: () => RuleDetailEditorState,
): Promise<RuleDetailRequestResult<T>> {
  const detail = await load();
  if (!detail || request !== currentRequest()) return { type: "ignore" };
  return editorState().isOpen && editorState().isDirty
    ? { type: "confirm", detail }
    : { type: "open", detail };
}
