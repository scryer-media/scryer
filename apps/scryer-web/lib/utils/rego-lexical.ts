// This is cursor context, not a Rego parser. Track strings across lines so a
// partial document never opens field completion inside a comment or raw string.
export function regoCodeAt(source: string, position: number): boolean {
  let state: "code" | "comment" | "string" | "raw" = "code";
  for (let i = 0; i < position; i++) {
    const ch = source[i];
    if (state === "comment") {
      if (ch === "\n") state = "code";
    } else if (state === "string") {
      if (ch === "\\") i++;
      else if (ch === '"') state = "code";
    } else if (state === "raw") {
      if (ch === "`") state = "code";
    } else if (ch === "#") state = "comment";
    else if (ch === '"') state = "string";
    else if (ch === "`") state = "raw";
  }
  return state === "code";
}
