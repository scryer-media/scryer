import assert from "node:assert/strict";
import { test } from "node:test";
import { CompletionContext } from "@codemirror/autocomplete";
import { EditorState, type TransactionSpec } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { history, undo, insertNewlineAndIndent } from "@codemirror/commands";
import { ensureSyntaxTree } from "@codemirror/language";
import { formatScriptIndentation, scriptAssistance, scriptCompletions, scriptLanguageExtensions, scriptVariables, type ScriptEditorLanguage } from "../../components/common/script-editor-assistance.ts";

const t = (key: string) => key;
const languages: ScriptEditorLanguage[] = ["shell", "python", "powershell", "batch", "go"];
function stateFor(language: ScriptEditorLanguage, doc: string, readOnly = false) {
  const state = EditorState.create({ doc, extensions: [scriptLanguageExtensions(language), history(), EditorState.readOnly.of(readOnly)] });
  assert.ok(ensureSyntaxTree(state, state.doc.length, 1000));
  return state;
}
function format(language: ScriptEditorLanguage, doc: string, readOnly = false) {
  const editor = {
    state: stateFor(language, doc, readOnly),
    dispatch: (spec: TransactionSpec) => { editor.state = editor.state.update(spec).state; },
  };
  const view = editor as unknown as EditorView;
  const handled = formatScriptIndentation(view);
  return { editor, view, handled, result: editor.state.doc.toString() };
}

test("variable catalog follows the actual trigger contracts", () => {
  assert.deepEqual(scriptVariables("POST_IMPORT"), ["SCRYER_METADATA", "SCRYER_EVENT", "SCRYER_FILE_PATH", "SCRYER_FACET", "SCRYER_TITLE_NAME", "SCRYER_TITLE_ID"]);
  assert.deepEqual(scriptVariables("SCHEDULE"), ["SCRYER_METADATA", "SCRYER_EVENT", "SCRYER_JOB_ID", "SCRYER_JOB_NAME", "SCRYER_RUN_ID", "SCRYER_TRIGGER_SOURCE"]);
});

test("completion sources retain their identity across transactions so pending menus can finish", () => {
  const state = EditorState.create({ doc: "SCR", extensions: [scriptLanguageExtensions("shell"), scriptAssistance("shell", "SCHEDULE", t, false)] });
  const sources = state.languageDataAt("autocomplete", 3);
  const updated = state.update({ selection: { anchor: 3 } }).state;
  assert.deepEqual(updated.languageDataAt("autocomplete", 3), sources);
});

for (const [language, opening, closing] of [["shell", "if true; then", "fi"], ["powershell", "if ($true) {", "}"]] as const) {
  test(`${language}: Enter indents a virtual new line and typing a closer dedents it`, () => {
    const editor = {
      state: EditorState.create({ doc: opening, selection: { anchor: opening.length }, extensions: [scriptLanguageExtensions(language), scriptAssistance(language, "SCHEDULE", t, false)] }),
      dispatch: (spec: TransactionSpec) => { editor.state = editor.state.update(spec).state; },
    };
    assert.equal(insertNewlineAndIndent(editor as unknown as EditorView), true);
    assert.equal(editor.state.doc.toString(), `${opening}\n  `);
    const pos = editor.state.doc.length;
    editor.dispatch({ changes: { from: pos, insert: closing }, selection: { anchor: pos + closing.length }, userEvent: "input.type" });
    assert.equal(editor.state.doc.toString(), `${opening}\n${closing}`);
  });
}

for (const language of languages) {
  test(`${language}: language constructs and trigger-specific variables`, () => {
    const state = stateFor(language, "SCR");
    const complete = (trigger: "POST_IMPORT" | "SCHEDULE") => scriptCompletions(language, trigger, t)(new CompletionContext(state, 3, true))!;
    const job = complete("SCHEDULE");
    assert.ok(job.options.some((option) => option.label === "SCRYER_JOB_ID"));
    assert.ok(!job.options.some((option) => option.label === "SCRYER_FILE_PATH"));
    assert.ok(job.options.some((option) => option.label === "if" && typeof option.apply === "function"));
    const imported = complete("POST_IMPORT");
    assert.ok(imported.options.some((option) => option.label === "SCRYER_FILE_PATH"));
    assert.ok(!imported.options.some((option) => option.label === "SCRYER_JOB_ID"));
  });
  test(`${language}: formatting respects read-only`, () => {
    const result = format(language, "  echo example", true);
    assert.equal(result.handled, false);
    assert.equal(result.result, "  echo example");
  });
}

for (const [language, doc, suffix] of [
  ["shell", "echo $SCR", "SCRYER_METADATA"],
  ["shell", "echo ${SCR", "SCRYER_METADATA}"],
  ["powershell", "Write-Output $env:SCR", "SCRYER_METADATA"],
  ["batch", "echo %SCR", "SCRYER_METADATA%"],
  ["python", 'os.environ["SCR', "SCRYER_METADATA"],
  ["python", 'os.getenv("SCR', "SCRYER_METADATA"],
  ["go", 'os.Getenv("SCR', "SCRYER_METADATA"],
] as const) {
  test(`${language}: completes ${doc} using native environment syntax`, () => {
    const state = stateFor(language, doc);
    const result = scriptCompletions(language, "SCHEDULE", t)(new CompletionContext(state, doc.length, true))!;
    assert.ok(result);
    const option = result.options.find((option) => option.label === "SCRYER_METADATA")!;
    assert.equal(option.apply, suffix);
    assert.equal(doc.slice(0, result.from), doc.slice(0, -3));
    assert.ok(result.options.every((option) => option.type === "variable"));
  });
}

for (const [language, doc] of [
  ["shell", "# echo $SCR"], ["python", "# SCR"], ["powershell", "# $env:SCR"], ["batch", "rem %SCR"], ["go", "// SCR"],
  ["shell", "echo 'SCR'"], ["python", 'print("SCR")'], ["go", 'println("SCR")'],
] as const) {
  test(`${language}: ignores unrelated strings and comments (${doc})`, () => {
    const pos = doc.indexOf("SCR") + 3;
    const state = stateFor(language, doc);
    assert.equal(scriptCompletions(language, "SCHEDULE", t)(new CompletionContext(state, pos, true)), null);
  });
}

for (const [language, source, expected] of [
  ["shell", 'if [ -n "$SCRYER_EVENT" ]; then\necho yes\nelse\necho no\nfi', 'if [ -n "$SCRYER_EVENT" ]; then\n  echo yes\nelse\n  echo no\nfi'],
  ["python", 'if True:\n print("hello")', 'if True:\n    print("hello")'],
  ["powershell", 'if ($true) {\nWrite-Output "hello"\n}', 'if ($true) {\n  Write-Output "hello"\n}'],
  ["batch", 'if defined SCRYER_EVENT (\necho yes\n)', 'if defined SCRYER_EVENT (\n  echo yes\n)'],
  ["go", 'package main\nfunc main() {\nprintln("hello")\n}', 'package main\nfunc main() {\n\tprintln("hello")\n}'],
] as const) {
  test(`${language}: indentation formatting is idempotent and undoable`, () => {
    const { result, view, editor } = format(language, source);
    assert.equal(result, expected);
    assert.equal(format(language, result).result, expected);
    assert.equal(undo(view), true);
    assert.equal(editor.state.doc.toString(), source);
  });
}

for (const [language, source] of [
  ["shell", "cat <<'EOF'\n   preserve data\nEOF"],
  ["python", 'text = """first\n   preserve data\nlast"""'],
  ["powershell", "$text = @'\n   preserve data\n'@"],
  ["powershell", 'if ($true) {\n  $text = @"\n$env:SCRYER_METADATA\n"@\n}'],
  ["go", 'package main\nvar text = `first\n   preserve data\nlast`'],
] as const) {
  test(`${language}: formatting preserves multiline literal contents`, () => {
    assert.equal(format(language, source).result, source);
  });
}
