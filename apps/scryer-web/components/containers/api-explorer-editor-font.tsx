import { useEffect } from "react";
import { useGraphiQL, useMonaco } from "@graphiql/react";

export function ApiExplorerEditorFont() {
  const query = useGraphiQL((state) => state.queryEditor);
  const variables = useGraphiQL((state) => state.variableEditor);
  const headers = useGraphiQL((state) => state.headerEditor);
  const response = useGraphiQL((state) => state.responseEditor);
  const monaco = useMonaco((state) => state.monaco);

  useEffect(() => {
    const fontFamily = getComputedStyle(document.documentElement).getPropertyValue("--font-code").trim();
    if (!fontFamily) return;
    // Configure Monaco itself so glyph measurements and the caret match the font.
    for (const editor of [query, variables, headers, response]) {
      editor?.getDomNode()?.setAttribute("data-code-font", "");
      editor?.updateOptions({ fontFamily });
    }
    void document.fonts.ready.then(() => monaco?.editor.remeasureFonts());
  }, [query, variables, headers, response, monaco]);

  return null;
}
