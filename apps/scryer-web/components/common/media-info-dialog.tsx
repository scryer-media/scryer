import * as React from "react";
import { Info } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { formatUiDate } from "@/lib/utils/date-format";
import {
  attachmentRows,
  audioTrackRows,
  captionRows,
  chapterRows,
  mediaFileBaseName,
  mediaInfoSections,
  subtitleTrackRows,
  type MediaInfoFileDetails,
  type MediaInfoSection,
} from "@/lib/utils/media-info-format";

type Translate = (key: string, values?: Record<string, string | number>) => string;

/**
 * The file row's "everything we know" affordance: one pill that opens the full
 * record. Styled to sit in whichever pill rail it was dropped into.
 */
export function MediaInfoButton({
  file,
  id,
  presentation = "default",
}: {
  file: MediaInfoFileDetails;
  id?: string;
  presentation?: "default" | "selected-title";
}) {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);
  return <>
    <button
      id={id}
      type="button"
      title={t("mediaFile.allInfo")}
      aria-label={t("mediaFile.allInfo")}
      onClick={() => setOpen(true)}
      className={
        presentation === "selected-title"
          ? "inline-flex cursor-pointer items-center gap-1 rounded-[6px] bg-[var(--scry-chip)] px-[9px] py-[3px] text-[10.5px] font-semibold text-[var(--scry-muted2)] hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
          : "inline-flex cursor-pointer items-center gap-1 rounded border border-border bg-muted/50 px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground hover:bg-muted dark:hover:bg-muted/80"
      }
    >
      <Info className="h-3 w-3 opacity-70" />
      {t("mediaFile.allInfo")}
    </button>
    {open ? <MediaInfoDialog open onOpenChange={setOpen} file={file} /> : null}
  </>;
}

/** Every recovered fact about one media file, as plain tables. */
export function MediaInfoDialog({
  open,
  onOpenChange,
  file,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  file: MediaInfoFileDetails;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const labels = {
    added: file.createdAt ? formatUiDate(file.createdAt, dateTimeFormat, { fallback: file.createdAt }) : null,
    grabbedAt: file.grabbedAt ? formatUiDate(file.grabbedAt, dateTimeFormat, { fallback: file.grabbedAt }) : null,
    yes: t("label.yes"),
    no: t("label.no"),
  };
  const sections = mediaInfoSections(file, labels);
  const audio = audioTrackRows(file);
  const subtitles = subtitleTrackRows(file);
  const chapters = chapterRows(file);
  const attachments = attachmentRows(file);
  const captions = captionRows(file);
  const attempt = file.analysisAttempt;
  const fileName = mediaFileBaseName(file.filePath);

  const fieldSection = (id: string) => sections.find((entry) => entry.id === id) ?? null;

  return <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent id="media-info-dialog" className="max-h-[85vh] overflow-y-auto sm:max-w-3xl">
      <DialogHeader>
        <DialogTitle className="break-all">{fileName ?? t("mediaFile.allInfo")}</DialogTitle>
        {file.filePath ? <DialogDescription className="break-all">{file.filePath}</DialogDescription> : null}
      </DialogHeader>

      <div className="space-y-5 text-sm">
        <FieldTable section={fieldSection("media-info-file")} t={t} />
        <FieldTable section={fieldSection("media-info-video")} t={t} />

        {audio.length > 0 ? (
          <Section id="media-info-audio" heading={t("mediaInfo.sectionAudio")}>
            <Table density="dense">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("mediaInfo.colIndex")}</TableHead>
                  <TableHead>{t("mediaInfo.colLanguage")}</TableHead>
                  <TableHead>{t("mediaInfo.colCodec")}</TableHead>
                  <TableHead>{t("mediaInfo.colProfile")}</TableHead>
                  <TableHead>{t("mediaInfo.colChannels")}</TableHead>
                  <TableHead>{t("mediaInfo.colBitrate")}</TableHead>
                  <TableHead>{t("mediaInfo.colSampleRate")}</TableHead>
                  <TableHead>{t("mediaInfo.colSampleDepth")}</TableHead>
                  <TableHead>{t("mediaInfo.colRoles")}</TableHead>
                  <TableHead>{t("mediaInfo.colName")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {audio.map((track) => (
                  <TableRow key={track.index}>
                    <TableCell>{track.index}</TableCell>
                    <TableCell>{track.language}</TableCell>
                    <TableCell>{track.codec ?? "—"}</TableCell>
                    <TableCell>{track.profile ?? "—"}</TableCell>
                    <TableCell>{track.channels ?? "—"}</TableCell>
                    <TableCell>{track.bitrate ?? "—"}</TableCell>
                    <TableCell>{track.sampleRate ?? "—"}</TableCell>
                    <TableCell>{track.sampleDepth ?? "—"}</TableCell>
                    <TableCell>{track.roleKeys.map((key) => t(key)).join(", ") || "—"}</TableCell>
                    <TableCell className="break-all">{track.name ?? "—"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </Section>
        ) : null}

        {subtitles.length > 0 ? (
          <Section id="media-info-subtitles" heading={t("mediaInfo.sectionSubtitles")}>
            <Table density="dense">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("mediaInfo.colIndex")}</TableHead>
                  <TableHead>{t("mediaInfo.colLanguage")}</TableHead>
                  <TableHead>{t("mediaInfo.colCodec")}</TableHead>
                  <TableHead>{t("mediaInfo.colForced")}</TableHead>
                  <TableHead>{t("mediaInfo.colDefault")}</TableHead>
                  <TableHead>{t("mediaInfo.colName")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {subtitles.map((track) => (
                  <TableRow key={track.index}>
                    <TableCell>{track.index}</TableCell>
                    <TableCell>{track.language}</TableCell>
                    <TableCell>{track.codec}</TableCell>
                    <TableCell>{track.forced ? labels.yes : labels.no}</TableCell>
                    <TableCell>{track.default ? labels.yes : labels.no}</TableCell>
                    <TableCell className="break-all">{track.name ?? "—"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </Section>
        ) : null}

        {captions.length > 0 ? (
          <Section id="media-info-captions" heading={t("mediaInfo.sectionCaptions")}>
            <ul className="space-y-1 text-muted-foreground">
              {captions.map((caption) => (
                <li key={`${caption.streamId}-${caption.standard}-${caption.serviceNumber}`}>
                  {[caption.standard, caption.serviceNumber, caption.language, caption.streamId]
                    .filter((part): part is string => Boolean(part))
                    .join(" · ")}
                </li>
              ))}
            </ul>
          </Section>
        ) : null}

        <FieldTable section={fieldSection("media-info-release")} t={t}>
          {file.scoringLog ? (
            <details id="media-info-scoring-log" className="mt-2 text-xs">
              <summary className="cursor-pointer text-muted-foreground">{t("mediaInfo.scoringLog")}</summary>
              <pre className="mt-1 whitespace-pre-wrap font-[var(--font-code)] text-muted-foreground">{file.scoringLog}</pre>
            </details>
          ) : null}
        </FieldTable>

        <FieldTable section={fieldSection("media-info-analysis")} t={t} />

        {chapters.length > 0 ? (
          <Section id="media-info-chapters" heading={t("mediaInfo.sectionChapters")}>
            <Table density="dense">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("mediaInfo.colIndex")}</TableHead>
                  <TableHead>{t("mediaInfo.colStart")}</TableHead>
                  <TableHead>{t("mediaInfo.colEnd")}</TableHead>
                  <TableHead>{t("mediaInfo.colTitle")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {chapters.map((chapter) => (
                  <TableRow key={chapter.index}>
                    <TableCell>{chapter.index}</TableCell>
                    <TableCell>{chapter.start}</TableCell>
                    <TableCell>{chapter.end ?? "—"}</TableCell>
                    <TableCell className="break-all">{chapter.title ?? "—"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </Section>
        ) : null}

        {attachments.length > 0 ? (
          <Section id="media-info-attachments" heading={t("mediaInfo.sectionAttachments")}>
            <Table density="dense">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("mediaInfo.colName")}</TableHead>
                  <TableHead>{t("mediaInfo.colType")}</TableHead>
                  <TableHead>{t("mediaInfo.colSize")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {attachments.map((attachment) => (
                  <TableRow key={attachment.id}>
                    <TableCell className="break-all">{attachment.name}</TableCell>
                    <TableCell>{attachment.mediaType ?? "—"}</TableCell>
                    <TableCell>{attachment.size ?? "—"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </Section>
        ) : null}

        {file.analysis ? null : (
          <p id="media-info-no-analysis" className="text-xs text-muted-foreground">{t("mediaFile.noAnalysis")}</p>
        )}
        {attempt && !attempt.succeeded ? (
          <div id="media-info-attempt-failed" className="space-y-1 rounded border border-amber-500 p-2 text-xs" role="status">
            <p>{t("mediaFile.analysisLatestInspection", {
              status: attempt.report.status.toLowerCase(),
              time: new Date(attempt.attemptedAt).toLocaleString(),
            })}</p>
            <p>{t((file.analysis?.revision ?? 0) > 0 ? "mediaFile.analysisPreviousRetained" : "mediaFile.analysisNoneSaved")}</p>
            {attempt.report.budgetExhausted ? <p>{t("mediaFile.analysisBudgetExhausted")}</p> : null}
            {attempt.report.warnings.map((warning, index) => <p key={`${warning.code}-${index}`}>{warning.message}</p>)}
          </div>
        ) : null}
      </div>
    </DialogContent>
  </Dialog>;
}

function Section({ id, heading, children }: { id: string; heading: string; children: React.ReactNode }) {
  return <section id={id} className="space-y-1">
    <h3 className="text-xs font-medium text-muted-foreground">{heading}</h3>
    {children}
  </section>;
}

function FieldTable({
  section,
  t,
  children,
}: {
  section: MediaInfoSection | null;
  t: Translate;
  children?: React.ReactNode;
}) {
  if (!section) return null;
  return <Section id={section.id} heading={t(section.titleKey)}>
    <Table density="dense">
      <TableBody>
        {section.rows.map((entry, index) => (
          <TableRow key={`${entry.labelKey}-${index}`}>
            <TableCell className="w-56 align-top font-medium text-muted-foreground">{t(entry.labelKey)}</TableCell>
            <TableCell className="break-all">{entry.value}</TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
    {children}
  </Section>;
}
