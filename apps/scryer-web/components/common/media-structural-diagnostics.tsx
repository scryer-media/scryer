import { useState } from "react";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import type { MediaStructuralDiagnostics as Diagnostics } from "@/lib/types/media-analysis";

const diagnoseMutation = `
  mutation DiagnoseMediaFile($fileId: ID!) {
    diagnoseMediaFile(fileId: $fileId) {
      sourceLength coverageTruncated warningsTruncated packetsSampled frameHeadersSampled checks
      coverage { offset length }
      report {
        status bytesRead seeks elapsedMs budgetExhausted
        warnings { code message streamId offset }
      }
    }
  }
`;

export function MediaStructuralDiagnostics({ fileId }: { fileId: string }) {
  const client = useClient();
  const t = useTranslate();
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<Diagnostics | null>(null);
  const [error, setError] = useState<string | null>(null);
  async function diagnose() {
    setRunning(true);
    setError(null);
    setResult(null);
    try {
      const response = await client.mutation<{ diagnoseMediaFile: Diagnostics }>(diagnoseMutation, { fileId }).toPromise();
      if (response.error) throw response.error;
      if (!response.data?.diagnoseMediaFile) throw new Error(t("mediaFile.diagnosticsFailed"));
      setResult(response.data.diagnoseMediaFile);
    } catch (error) {
      setError(error instanceof Error ? error.message : t("mediaFile.diagnosticsFailed"));
    } finally { setRunning(false); }
  }
  const covered = result?.coverage.reduce((total, range) => total + Number(range.length), 0) ?? 0;
  const percent = result && Number(result.sourceLength) > 0 ? covered * 100 / Number(result.sourceLength) : 0;
  return <div className="mt-3 space-y-2 border-t pt-2">
    <p className="text-muted-foreground">{t("mediaFile.diagnosticsScope")}</p>
    <button type="button" disabled={running} className="rounded border px-2 py-1 disabled:opacity-50"
      onClick={() => { void diagnose(); }}>{t(running ? "mediaFile.diagnosing" : "mediaFile.diagnose")}</button>
    {error ? <p role="alert" className="text-destructive">{error}</p> : null}
    {result ? <div aria-live="polite" className="space-y-1">
      <p>{result.report.status.toLowerCase()}</p>
      <p>{t("mediaFile.diagnosticsRead", { bytes: Number(result.report.bytesRead).toLocaleString(), milliseconds: String(result.report.elapsedMs) })}</p>
      <p>{t("mediaFile.diagnosticsSamples", { packets: String(result.packetsSampled), frames: String(result.frameHeadersSampled) })}</p>
      <p>{t("mediaFile.diagnosticsCoverage", { percent: percent.toFixed(3) })}</p>
      {result.report.warnings.map((warning, index) => <p key={index} className="text-amber-500">{warning.message}{warning.streamId ? ` · ${warning.streamId}` : ""}</p>)}
      {result.coverageTruncated || result.warningsTruncated ? <p>{t("mediaFile.diagnosticsTruncated")}</p> : null}
      <details><summary>{t("mediaFile.diagnosticsRanges")}</summary>
        <div className="max-h-32 overflow-y-auto font-mono">{result.coverage.map((range) => <p key={String(range.offset)}>{range.offset} + {range.length}</p>)}</div>
      </details>
    </div> : null}
  </div>;
}
