import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useGraphiQL, useGraphiQLActions } from "@graphiql/react";
import { Pencil } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { renameExplorerTab, restoreExplorerTabNames } from "@/lib/graphql/api-explorer-tabs";

function InlineTabName({ name, onFinish }: { name: string; onFinish: (name?: string) => void }) {
  const t = useTranslate();
  const finished = useRef(false);
  const finish = (value?: string) => {
    if (finished.current) return;
    finished.current = true;
    onFinish(value);
  };
  return <input className="scryer-api-tab-name" aria-label={t("apiExplorer.tabName")}
    defaultValue={name} maxLength={100} autoFocus
    style={{ width: `${Math.min(32, Math.max(12, name.length + 2))}ch` }}
    onFocus={(event) => event.target.select()}
    onPointerDown={(event) => event.stopPropagation()}
    onBlur={(event) => finish(event.currentTarget.value)}
    onKeyDown={(event) => {
      event.stopPropagation();
      if (event.nativeEvent.isComposing) return;
      if (event.key === "Enter" || event.key === "Escape") {
        event.preventDefault();
        finish(event.key === "Enter" ? event.currentTarget.value : undefined);
      }
    }} />;
}

export function ApiExplorerRenameTab() {
  const t = useTranslate();
  const tabs = useGraphiQL((state) => state.tabs);
  const editor = useGraphiQL((state) => state.queryEditor);
  const activeTabIndex = useGraphiQL((state) => state.activeTabIndex);
  const { storeTabs } = useGraphiQLActions();
  const [editingId, setEditingId] = useState<string | null>(null);
  const [targets, setTargets] = useState<{ id: string; element: HTMLElement }[]>([]);
  const beginRename = useCallback((id: string) => {
    const tab = tabs.find((tab) => tab.id === id);
    if (!tab) return;
    setEditingId(id);
  }, [tabs]);

  useLayoutEffect(() => {
    // Persist labels only. moveTab also calls Monaco.setValue, which resets the
    // selection and undo history and triggers another editor-change event.
    const restored = restoreExplorerTabNames(tabs);
    if (restored) storeTabs({ tabs: restored, activeTabIndex });
    const root = editor?.getDomNode()?.closest(".graphiql-container");
    tabs.forEach((tab, index) => {
      const label = root?.querySelector(`#graphiql-session-tab-${index}`);
      if (!label) return;
      if (label.textContent !== tab.title) label.textContent = tab.title;
      label.setAttribute("title", tab.title);
    });
  }, [tabs, activeTabIndex, editor, editingId, storeTabs]);

  useLayoutEffect(() => {
    // GraphiQL has no tab-label slot. Extend its native tabs without replacing
    // their switching, closing, or drag-reordering behavior.
    const root = editor?.getDomNode()?.closest(".graphiql-container");
    const cleanups: (() => void)[] = [];
    const next: typeof targets = [];
    tabs.forEach((tab, index) => {
      const label = root?.querySelector(`#graphiql-session-tab-${index}`);
      const element = label?.closest<HTMLElement>(".graphiql-tab");
      if (!label || !element) return;
      const rename = () => beginRename(tab.id);
      label.addEventListener("dblclick", rename);
      cleanups.push(() => label.removeEventListener("dblclick", rename));
      // Keep the native label immediately before Close: GraphiQL relies on it.
      const slot = document.createElement("span");
      slot.className = "scryer-api-tab-actions";
      element.prepend(slot);
      cleanups.push(() => slot.remove());
      next.push({ id: tab.id, element: slot });
    });
    setTargets(next);
    return () => { for (const cleanup of cleanups) cleanup(); };
  }, [editor, tabs, beginRename]);

  return targets.map(({ id, element }) => createPortal(
      editingId === id ? <InlineTabName name={tabs.find((tab) => tab.id === id)?.title ?? ""}
        onFinish={(name) => {
          setEditingId(null);
          if (name === undefined) return;
          const renamed = renameExplorerTab(tabs, tabs.findIndex((tab) => tab.id === id), name);
          if (renamed) storeTabs({ tabs: renamed, activeTabIndex });
        }} /> :
        <button type="button" className="scryer-api-tab-rename"
          aria-label={t("apiExplorer.renameTab")} title={t("apiExplorer.renameTab")}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => { event.stopPropagation(); beginRename(id); }}>
          <Pencil size={12} aria-hidden="true" />
        </button>, element, id,
  ));
}
