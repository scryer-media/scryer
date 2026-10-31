import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  snippetCompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import {
  indentOnInput,
  indentRange,
  indentService,
  indentUnit,
  StreamLanguage,
  syntaxTree,
  ensureSyntaxTree,
  bracketMatching,
  type StreamParser,
} from "@codemirror/language";
import { python } from "@codemirror/lang-python";
import { go } from "@codemirror/lang-go";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { powerShell } from "@codemirror/legacy-modes/mode/powershell";
import { EditorView, keymap } from "@codemirror/view";
import { EditorState, type Extension } from "@codemirror/state";
import type { ScriptTriggerValue } from "../../lib/types/scripts.ts";

type Translate = (key: string) => string;
export type ScriptEditorLanguage =
  "shell" | "python" | "powershell" | "batch" | "go";

const commonVariables = ["SCRYER_METADATA", "SCRYER_EVENT"];
const importVariables = [
  "SCRYER_FILE_PATH",
  "SCRYER_FACET",
  "SCRYER_TITLE_NAME",
  "SCRYER_TITLE_ID",
];
const jobVariables = [
  "SCRYER_JOB_ID",
  "SCRYER_JOB_NAME",
  "SCRYER_RUN_ID",
  "SCRYER_TRIGGER_SOURCE",
];

export function scriptVariables(trigger: ScriptTriggerValue): string[] {
  return [
    ...commonVariables,
    ...(trigger === "POST_IMPORT" ? importVariables : jobVariables),
  ];
}

const words: Record<ScriptEditorLanguage, readonly string[]> = {
  shell: [
    "if",
    "then",
    "else",
    "elif",
    "fi",
    "for",
    "in",
    "do",
    "done",
    "while",
    "case",
    "esac",
    "export",
    "return",
    "exit",
    "printf",
    "echo",
    "read",
    "test",
    "set",
    "cd",
    "pwd",
  ],
  python: [
    "import",
    "from",
    "as",
    "if",
    "elif",
    "else",
    "for",
    "in",
    "while",
    "def",
    "return",
    "try",
    "except",
    "finally",
    "with",
    "class",
    "True",
    "False",
    "None",
    "print",
    "len",
    "str",
    "int",
    "open",
  ],
  powershell: [
    "if",
    "else",
    "elseif",
    "foreach",
    "in",
    "while",
    "function",
    "param",
    "return",
    "try",
    "catch",
    "finally",
    "Write-Output",
    "Write-Error",
    "Get-Content",
    "Set-Content",
    "Test-Path",
    "Join-Path",
    "ConvertFrom-Json",
    "ConvertTo-Json",
  ],
  batch: [
    "echo",
    "set",
    "setlocal",
    "endlocal",
    "if",
    "else",
    "for",
    "in",
    "do",
    "call",
    "goto",
    "exit",
    "rem",
    "exist",
    "errorlevel",
    "defined",
    "pushd",
    "popd",
  ],
  go: [
    "package",
    "import",
    "func",
    "var",
    "const",
    "type",
    "if",
    "else",
    "for",
    "range",
    "switch",
    "case",
    "default",
    "return",
    "defer",
    "go",
    "nil",
    "true",
    "false",
    "make",
    "len",
    "append",
  ],
};

const snippets: Record<ScriptEditorLanguage, readonly [string, string][]> = {
  shell: [
    ["if", "if [ ${condition} ]; then\n\t${command}\nfi"],
    ["for", "for ${item} in ${items}; do\n\t${command}\ndone"],
  ],
  python: [
    ["if", "if ${condition}:\n\t${pass}"],
    ["for", "for ${item} in ${items}:\n\t${pass}"],
    ["def", "def ${name}(${args}):\n\t${pass}"],
  ],
  powershell: [
    ["if", "if (${condition}) {\n\t${command}\n}"],
    ["foreach", "foreach ($${item} in $${items}) {\n\t${command}\n}"],
  ],
  batch: [
    ["if", "if ${condition} (\n\t${command}\n)"],
    ["for", "for %%${i} in (${items}) do (\n\t${command}\n)"],
  ],
  go: [
    ["if", "if ${condition} {\n\t${statement}\n}"],
    ["func", "func ${name}(${args}) {\n\t${statement}\n}"],
  ],
};

function inLiteral(state: EditorState, pos: number): boolean {
  for (
    let node = syntaxTree(state).resolveInner(pos, -1);
    node;
    node = node.parent!
  ) {
    if (/string|comment|heredoc/i.test(node.name)) return true;
  }
  return false;
}

/** Complete environment access in each interpreter's syntax, including inside accessor strings. */
export function scriptCompletions(
  language: ScriptEditorLanguage,
  trigger: ScriptTriggerValue,
  t: Translate,
) {
  return (context: CompletionContext): CompletionResult | null => {
    const before = context.state.sliceDoc(
      context.state.doc.lineAt(context.pos).from,
      context.pos,
    );
    const patterns: Record<ScriptEditorLanguage, RegExp> = {
      shell: /\$(\{?)([A-Za-z_]\w*)?$/,
      powershell: /\$env:([A-Za-z_]\w*)?$/i,
      batch: /%([A-Za-z_]\w*)?$/,
      python: /os\.(?:environ\[|(?:getenv|environ\.get)\()["']([A-Za-z_]\w*)?$/,
      go: /os\.(?:Getenv|LookupEnv)\("([A-Za-z_]\w*)?$/,
    };
    const env = before.match(patterns[language]);
    const word = context.matchBefore(/[A-Za-z_][\w-]*/);
    const prefix = env
      ? (env[language === "shell" ? 2 : 1] ?? "")
      : (word?.text ?? "");
    const literal = inLiteral(context.state, context.pos);
    // Environment expressions inside strings are useful; comments never are.
    const node = syntaxTree(context.state).resolveInner(context.pos, -1);
    if (/comment/i.test(node.name) || (!env && literal)) return null;
    if (!env && !word && !context.explicit) return null;
    const from = context.pos - prefix.length;
    const after = context.state.sliceDoc(context.pos, context.pos + 1);
    const options: Completion[] = scriptVariables(trigger).map((name) => {
      let apply: string;
      if (env) {
        apply =
          name +
          (language === "shell" && env[1] === "{" && after !== "}"
            ? "}"
            : language === "batch" && after !== "%"
              ? "%"
              : "");
      } else {
        apply =
          language === "shell"
            ? `"$${name}"`
            : language === "powershell"
              ? `$env:${name}`
              : language === "batch"
                ? `%${name}%`
                : language === "python"
                  ? `os.environ["${name}"]`
                  : `os.Getenv("${name}")`;
      }
      return {
        label: name,
        type: "variable",
        apply,
        detail: t("script.editor.injectedVariable"),
        info: t(`script.editor.env.${name}`),
        boost: 5,
      };
    });
    if (!env) {
      options.push(
        ...words[language]
          .filter(
            (label) => !snippets[language].some(([name]) => name === label),
          )
          .map((label) => ({ label, type: "keyword" })),
      );
      options.push(
        ...snippets[language].map(([label, template]) =>
          snippetCompletion(template, {
            label,
            type: "text",
            detail: t("script.editor.snippet"),
          }),
        ),
      );
    }
    return { from, options, validFor: env ? /^\w*$/ : /^[\w-]*$/ };
  };
}

const batchMode: StreamParser<{ depth: number }> = {
  name: "batch",
  startState: () => ({ depth: 0 }),
  token(stream, state) {
    if (stream.eatSpace()) return null;
    if (
      /^\s*$/.test(stream.string.slice(0, stream.pos)) &&
      stream.match(/(?:@?rem\b|::).*/i)
    )
      return "comment";
    if (stream.match(/^\^./)) return null;
    if (stream.match(/^%[^%\s]+%|^![^!\s]+!|^%%[A-Za-z]|^%[0-9*]/))
      return "variableName";
    if (stream.match(/^"(?:[^"^\r\n]|\^.)*"/)) return "string";
    if (stream.match(/^:[\w-]+/)) return "labelName";
    if (stream.match(/^\d+/)) return "number";
    const word = stream.match(/^[\w-]+/);
    if (word)
      return words.batch.includes((word as RegExpMatchArray)[0].toLowerCase())
        ? "keyword"
        : null;
    const char = stream.next();
    if (char === "(") state.depth++;
    if (char === ")") state.depth = Math.max(0, state.depth - 1);
    return /[|&<>]/.test(char ?? "") ? "operator" : null;
  },
  indent: (state, textAfter, context) =>
    Math.max(0, state.depth - (textAfter.trimStart().startsWith(")") ? 1 : 0)) *
    context.unit,
  languageData: {
    commentTokens: { line: "rem" },
    closeBrackets: { brackets: ["(", '"'] },
  },
};

export function scriptLanguageExtensions(
  language: ScriptEditorLanguage,
): Extension {
  if (language === "python") return [python(), indentUnit.of("    ")];
  if (language === "go") return [go(), indentUnit.of("\t")];
  if (language === "batch")
    return [StreamLanguage.define(batchMode), indentUnit.of("  ")];
  return [
    StreamLanguage.define(language === "shell" ? shell : powerShell),
    indentUnit.of("  "),
    indentService.of((context, pos) => {
      const line = context.lineAt(pos);
      const number = context.state.doc.lineAt(pos).number;
      const splitting = context.simulatedBreak === pos;
      if (!splitting && number === 1) return 0;
      const previous = splitting
        ? context.lineAt(pos, -1)
        : context.state.doc.line(number - 1);
      if (
        inLiteral(context.state, previous.from + previous.text.length) &&
        inLiteral(context.state, line.from)
      )
        return null;
      // Mask literals before recognizing block punctuation or shell control words.
      const mask = (from: number, text: string) =>
        text
          .replace(/./g, (char, index: number) =>
            inLiteral(context.state, from + index + 1) ? " " : char,
          )
          .trim();
      const prior = mask(previous.from, previous.text);
      const current = mask(line.from, line.text);
      const opens =
        language === "shell" ? /(?:\b(?:then|do|else)\s*|\{)$/ : /[({[]$/;
      const closes =
        language === "shell"
          ? /^(?:fi\b|done\b|esac\b|else\b|elif\b|\})/
          : /^[)}\]]/;
      return Math.max(
        0,
        context.lineIndent(previous.from) +
          (opens.test(prior) ? context.unit : 0) -
          (closes.test(current) ? context.unit : 0),
      );
    }),
  ];
}

/** Only change leading indentation; preserve literal contents and make the edit undoable. */
export function formatScriptIndentation(view: EditorView): boolean {
  if (view.state.readOnly) return false;
  const tree = ensureSyntaxTree(view.state, view.state.doc.length, 100);
  if (!tree) return false;
  // PowerShell splits interpolated here-strings into separate string/variable tokens.
  const hereStringLines = new Set<number>();
  let hereStringEnd: string | null = null;
  for (let number = 1; number <= view.state.doc.lines; number++) {
    const line = view.state.doc.line(number);
    if (hereStringEnd) {
      hereStringLines.add(number);
      if (line.text.startsWith(hereStringEnd)) hereStringEnd = null;
      continue;
    }
    const opener = /@(['"])\s*$/.exec(line.text);
    if (
      opener &&
      /string/i.test(tree.resolveInner(line.from + opener.index + 1, 1).name)
    )
      hereStringEnd = `${opener[1]}@`;
  }
  const changes = indentRange(view.state, 0, view.state.doc.length);
  const safe: { from: number; to: number; insert: string }[] = [];
  changes.iterChanges((from, to, _fromB, _toB, inserted) => {
    const node = tree.resolveInner(from, 1);
    if (
      /string|heredoc/i.test(node.name) ||
      hereStringLines.has(view.state.doc.lineAt(from).number)
    )
      return;
    safe.push({ from, to, insert: inserted.toString() });
  });
  view.dispatch({ changes: safe, userEvent: "input.indent" });
  return true;
}

export function scriptAssistance(
  language: ScriptEditorLanguage,
  trigger: ScriptTriggerValue,
  t: Translate,
  readOnly: boolean,
): Extension {
  const theme = EditorView.theme({
    ".cm-tooltip": {
      backgroundColor: "var(--popover)",
      color: "var(--popover-foreground)",
      border: "1px solid var(--border-color)",
      borderRadius: "6px",
    },
    ".cm-tooltip-autocomplete ul li[aria-selected]": {
      backgroundColor: "var(--accent)",
      color: "var(--accent-foreground)",
    },
    ".cm-completionInfo": {
      padding: "8px 12px",
      maxWidth: "min(420px, 85vw)",
      whiteSpace: "pre-wrap",
    },
    ".cm-completionDetail": { marginLeft: "1em", opacity: "0.75" },
  });
  if (readOnly) return [theme, bracketMatching()];
  const autocomplete = scriptCompletions(language, trigger, t);
  return [
    theme,
    bracketMatching(),
    indentOnInput(),
    closeBrackets(),
    autocompletion(),
    // Augment the language package's built-in and local-name completions.
    EditorState.languageData.of(() => [
      {
        autocomplete,
        indentOnInput: /^\s*(?:[}\])]|fi|done|esac|else|elif)\s*$/,
      },
    ]),
    keymap.of([
      ...closeBracketsKeymap,
      {
        key: "Shift-Alt-f",
        run: formatScriptIndentation,
        preventDefault: true,
      },
    ]),
  ];
}
