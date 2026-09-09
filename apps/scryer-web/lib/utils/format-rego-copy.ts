/**
 * Conservative layout for copied Rego v1: expand `if`/`else` bodies without
 * parsing or regenerating expressions. Quoted/raw strings and comments remain
 * opaque; collection literals keep their existing line breaks. Invalid lexical
 * structure falls back to the original source so copying never loses a draft.
 */
export function formatRegoCopy(source: string): string {
  const tokens = /"(?:\\[\s\S]|[^"\\])*"|`[^`]*`|#[^\r\n]*|\r\n|[\r\n]|[ \t]+|[A-Za-z_][A-Za-z_0-9]*|(?:\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|:=|==|!=|<=|>=|[^\s]/gy;
  const frames: { close: string; body: boolean }[] = [];
  const lines: string[] = [];
  const operators = new Set([":=", "=", "==", "!=", "<=", ">=", "<", ">", "+", "*", "/", "%", "&", "|"]);
  let line = "";
  let space = false;
  let originalNewline = false;
  let previous = "";
  let beforePrevious = "";
  let offset = 0;

  const endLine = () => {
    if (line) lines.push(line);
    line = "";
    space = false;
  };
  const append = (text: string, needsSpace = false) => {
    if (!line) line = "  ".repeat(frames.length);
    else if (space || needsSpace) line += " ";
    line += text;
    space = false;
  };

  for (const match of source.matchAll(tokens)) {
    if (match.index !== offset) return source;
    const token = match[0];
    offset += token.length;
    if (token === "\n" || token === "\r" || token === "\r\n") {
      if (!line && originalNewline && lines.length > 0) lines.push("");
      endLine();
      originalNewline = true;
      continue;
    }
    if (/^[ \t]+$/.test(token)) {
      space = true;
      continue;
    }
    originalNewline = false;
    if (token === '"' || token === "`") return source;
    if (token.startsWith("#")) {
      append(token, true);
      continue;
    }

    if (token === ";" && frames.at(-1)?.body) {
      endLine();
    } else if (["}", "]", ")"].includes(token)) {
      const frame = frames.pop();
      if (!frame || frame.close !== token) return source;
      if (frame.body) endLine();
      append(token);
    } else {
      const body = token === "{" && ["if", "else"].includes(previous) && beforePrevious !== ".";
      append(token, body || operators.has(token) || operators.has(previous) || [",", ":", ";"].includes(previous));
      const close = token === "{" ? "}" : token === "[" ? "]" : token === "(" ? ")" : null;
      if (close) {
        frames.push({ close, body });
        if (frames.length > 128) return source;
        if (body) endLine();
      }
    }
    beforePrevious = previous;
    previous = token;
  }

  if (offset !== source.length || frames.length) return source;
  lines.push(line);
  return lines.join("\n");
}
