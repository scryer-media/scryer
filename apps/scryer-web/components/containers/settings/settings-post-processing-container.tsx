import { useState } from "react";

import { ScriptEditorDialogs } from "@/components/common/script-editor-dialogs";
import { SettingsPostProcessingSection } from "@/components/views/settings/settings-post-processing-section";
import { useScriptEditor } from "@/lib/hooks/use-script-editor";
import type {
  PostProcessingScript,
  PostProcessingScriptDraft,
  PostProcessingScriptRun,
} from "@/lib/types/scripts";

export type PPScript = PostProcessingScript;
export type PPScriptRun = PostProcessingScriptRun;
export type PPScriptDraft = PostProcessingScriptDraft;

export function SettingsPostProcessingContainer() {
  const editor = useScriptEditor("POST_IMPORT");
  const [expandedScriptId, setExpandedScriptId] = useState<string | null>(null);

  return (
    <>
      <SettingsPostProcessingSection
        scripts={editor.scripts}
        isEditorOpen={editor.isEditorOpen}
        editorMode={editor.editingScriptId ? "edit" : "create"}
        editingScriptId={editor.editingScriptId}
        scriptDraft={editor.scriptDraft}
        setScriptDraft={editor.setScriptDraft}
        submitScript={editor.submitScript}
        mutatingScriptId={editor.mutatingScriptId}
        resetDraft={editor.requestCloseEditor}
        startCreateScript={editor.requestCreateEditor}
        editScript={editor.requestEditScript}
        toggleScript={editor.toggleScript}
        deleteScript={editor.requestDeleteScript}
        expandedScriptId={expandedScriptId}
        setExpandedScriptId={setExpandedScriptId}
        scriptRuns={editor.scriptRuns}
        loadRunsForScript={editor.loadRunsForScript}
      />
      <ScriptEditorDialogs
        state={editor.dialogs}
        ids={{
          inlineShellContent: "settings-post-processing-inline-shell-confirm",
          inlineShellAccept: "settings-post-processing-inline-shell-confirm-accept",
          inlineShellCancel: "settings-post-processing-inline-shell-confirm-cancel",
        }}
      />
    </>
  );
}
