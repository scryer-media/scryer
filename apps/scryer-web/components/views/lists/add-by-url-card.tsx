import * as React from "react";
import { Link2 } from "lucide-react";

import { LoadingMark } from "@/components/common/loading-mark";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListPreview, ListProviderManifest } from "@/lib/types/lists";
import { recognizeListUrl } from "@/lib/utils/lists";

import { ProviderTile } from "./provider-tile";

type AddByUrlCardProps = {
  providers: ListProviderManifest[];
  onPreviewUrl: (url: string) => Promise<ListPreview | null>;
  onRecognized: (url: string, preview: ListPreview) => void;
};

/**
 * Paste a list URL. Recognition runs locally for instant feedback; the
 * server preview decides whether the list can be followed.
 */
export function AddByUrlCard({ providers, onPreviewUrl, onRecognized }: AddByUrlCardProps) {
  const t = useTranslate();
  const [url, setUrl] = React.useState("");
  const [checking, setChecking] = React.useState(false);
  const [unrecognized, setUnrecognized] = React.useState(false);
  const recognition = React.useMemo(() => recognizeListUrl(url, providers), [providers, url]);
  const item = recognition
    ? recognition.manifest.groups
        .flatMap((group) => group.items)
        .find((entry) => entry.sourceType === recognition.source.sourceType)
    : null;

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    const trimmed = url.trim();
    if (!trimmed) return;
    setChecking(true);
    setUnrecognized(false);
    try {
      const preview = await onPreviewUrl(trimmed);
      if (!preview) return;
      if (!preview.recognized) {
        setUnrecognized(true);
        return;
      }
      onRecognized(trimmed, preview);
      setUrl("");
    } finally {
      setChecking(false);
    }
  };

  return (
    <form
      id="lists-add-by-url"
      onSubmit={(event) => void submit(event)}
      className="space-y-2 rounded-[12px] border border-[var(--scry-border3)] bg-[var(--scry-surf)] p-4"
    >
      <label htmlFor="lists-add-by-url-input" className="block text-[13px] font-semibold text-[var(--scry-ink2)]">
        {t("lists.url.label")}
      </label>
      <div className="flex flex-col gap-2 sm:flex-row">
        <div className="relative min-w-0 flex-1">
          <Link2 className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--scry-muted)]" />
          <Input
            id="lists-add-by-url-input"
            inputMode="url"
            className="pl-9"
            placeholder={t("lists.url.placeholder")}
            value={url}
            onChange={(event) => {
              setUrl(event.target.value);
              setUnrecognized(false);
            }}
          />
        </div>
        <Button id="lists-add-by-url-submit" type="submit" disabled={checking || !url.trim()}>
          {checking ? <LoadingMark className="h-4 w-4" /> : null}
          {t("lists.url.submit")}
        </Button>
      </div>
      <div aria-live="polite" className="min-h-5 text-[12.5px]">
        {recognition ? (
          <span id="lists-add-by-url-recognized" className="inline-flex items-center gap-2 text-[var(--scry-ink2)]">
            <ProviderTile provider={recognition.manifest} size="sm" className="h-5 w-5 rounded-[5px] text-[9px]" />
            {t("lists.url.recognized", {
              provider: recognition.manifest.name,
              list: item?.name ?? recognition.source.sourceType,
            })}
          </span>
        ) : unrecognized ? (
          <span id="lists-add-by-url-unrecognized" className="text-[var(--scry-warning-text)]">
            {t("lists.url.unrecognized")}
          </span>
        ) : (
          <span className="text-[var(--scry-muted)]">{t("lists.url.help")}</span>
        )}
      </div>
    </form>
  );
}
