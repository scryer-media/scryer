import {
  autocompletion,
  snippetCompletion,
  type Completion,
} from "@codemirror/autocomplete";
import { EditorView, hoverTooltip } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import {
  regoCatalog,
  regoCodeAt,
  regoCompletionContext,
  regoFieldCompletions,
  regoFields,
  normalizeRegoPath,
  REGO_INDEX_SOURCE,
  type RegoFamily,
} from "@/lib/utils/rego-assistance";

type Translate = (key: string) => string;

export function regoAssistance(
  family: RegoFamily,
  t: Translate,
  readOnly: boolean,
): Extension {
  const fields = regoFields(family);
  const describe = (field: (typeof fields)[number]) =>
    [t(field.descKey), field.availabilityKey ? t(field.availabilityKey) : ""]
      .filter(Boolean)
      .join("\n\n");
  const helpers = regoCatalog.helpers;
  const hover = hoverTooltip((view, position) => {
    const source = view.state.doc.toString();
    if (!regoCodeAt(source, position)) return null;
    const pattern = new RegExp(
      String.raw`\b(?:input|scryer|object|lower|upper|count|startswith)\b(?:\.[A-Za-z_]\w*|${REGO_INDEX_SOURCE})*`,
      "g",
    );
    for (const match of source.matchAll(pattern)) {
      if (position < match.index || position > match.index + match[0].length)
        continue;
      const name = normalizeRegoPath(match[0]);
      const field = fields.find((item) => item.path === name);
      const helper = helpers.find((item) => item.name === name);
      if (!field && !helper) return null;
      return {
        pos: match.index,
        end: match.index + match[0].length,
        above: true,
        create() {
          const dom = document.createElement("div");
          dom.className = "rego-documentation";
          const heading = document.createElement("strong");
          heading.textContent = field
            ? `${name}: ${field.type}`
            : `${name}(${helper!.args.join(", ")}): ${helper!.returns}`;
          const body = document.createElement("p");
          body.textContent = field ? describe(field) : t(helper!.descKey);
          dom.append(heading, body);
          return { dom };
        },
      };
    }
    return null;
  });
  const theme = EditorView.theme({
    ".cm-tooltip": {
      backgroundColor: "var(--popover)",
      color: "var(--popover-foreground)",
      border: "1px solid var(--border)",
      borderRadius: "6px",
      maxWidth: "min(480px, 90vw)",
    },
    ".cm-tooltip-autocomplete ul li[aria-selected]": {
      backgroundColor: "var(--accent)",
      color: "var(--accent-foreground)",
    },
    ".cm-completionInfo, .rego-documentation": {
      padding: "8px 12px",
      whiteSpace: "pre-wrap",
      overflowWrap: "anywhere",
      maxWidth: "min(420px, 85vw)",
    },
    ".cm-completionDetail": { marginLeft: "1em", opacity: "0.75" },
    ".cm-tooltip-lint": {
      maxWidth: "min(480px, 90vw)",
      whiteSpace: "pre-wrap",
    },
  });
  if (readOnly) return [hover, theme];
  return [
    hover,
    theme,
    autocompletion({
      override: [
        (context) => {
          const cursor = regoCompletionContext(
            context.state.doc.toString(),
            context.pos,
          );
          if (
            !cursor ||
            (!cursor.parent && !cursor.prefix && !context.explicit)
          )
            return null;
          const options: Completion[] = regoFieldCompletions(
            family,
            cursor.parent,
          ).map((field) => ({
            label: field.path.slice(field.path.lastIndexOf(".") + 1),
            type: "property",
            detail: field.type,
            info: describe(field),
          }));
          for (const helper of helpers) {
            const dot = helper.name.lastIndexOf(".");
            const parent = dot < 0 ? "" : helper.name.slice(0, dot);
            if (cursor.parent !== parent) continue;
            const name = helper.name.slice(dot + 1);
            options.push(
              snippetCompletion(
                `${name}(${helper.args.map((arg) => `\${${arg}}`).join(", ")})`,
                {
                  label: name,
                  type: "function",
                  detail: `(${helper.args.join(", ")}) → ${helper.returns}`,
                  info: t(helper.descKey),
                },
              ),
            );
          }
          if (!cursor.parent) {
            for (const label of ["input", "scryer", "object"])
              options.push({ label, type: "namespace" });
            for (const label of [
              "if",
              "some",
              "every",
              "in",
              "not",
              "contains",
              "default",
              "true",
              "false",
              "null",
            ])
              options.push({ label, type: "keyword" });
            for (const item of regoCatalog.snippets.filter(
              (item) => item.family === family,
            )) {
              options.push(
                snippetCompletion(item.source, {
                  label: item.label,
                  type: "text",
                  detail: t("settings.regoSnippetLabel"),
                  info: t(item.descKey),
                }),
              );
            }
          }
          return { from: cursor.from, options, validFor: /^\w*$/ };
        },
      ],
    }),
  ];
}
