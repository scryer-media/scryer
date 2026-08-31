export type ManualImportVideoFacts = {
  containerFormat: string | null;
  videoCodec: string | null;
  audioCodec: string | null;
  videoWidth: number | null;
  videoHeight: number | null;
  durationSeconds: number | null;
};

function formatCodec(value: string): string {
  const normalized = value.trim().toLowerCase();
  const labels: Record<string, string> = {
    ac3: "AC-3",
    av1: "AV1",
    eac3: "E-AC-3",
    h264: "H.264",
    hevc: "HEVC",
    vp9: "VP9",
  };
  return labels[normalized] ?? value.toUpperCase();
}

export function formatManualImportVideoFacts(facts: ManualImportVideoFacts | null): string {
  if (!facts) return "Media details unavailable for this format";

  const parts: string[] = [];
  if (facts.containerFormat) parts.push(facts.containerFormat);
  if (facts.videoWidth && facts.videoHeight) {
    parts.push(`${facts.videoWidth}×${facts.videoHeight}`);
  }
  if (facts.videoCodec) parts.push(formatCodec(facts.videoCodec));
  if (facts.audioCodec) parts.push(formatCodec(facts.audioCodec));
  if (facts.durationSeconds && facts.durationSeconds > 0) {
    const totalMinutes = Math.round(facts.durationSeconds / 60);
    const hours = Math.floor(totalMinutes / 60);
    const minutes = totalMinutes % 60;
    parts.push(hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`);
  }
  return parts.join(" · ");
}
