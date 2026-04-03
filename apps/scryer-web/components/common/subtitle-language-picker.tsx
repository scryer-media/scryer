import * as React from "react";
import { createPortal } from "react-dom";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ChevronDown, Search, X } from "lucide-react";
import {
  SUBTITLE_LANGUAGES,
  getSubtitleLanguage,
  type SubtitleLanguage,
} from "@/lib/constants/subtitle-languages";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

type SubtitleLanguagePickerProps = {
  value: string[];
  onChange: (codes: string[]) => void;
  className?: string;
  buttonClassName?: string;
  compact?: boolean;
};

function matchesFilter(lang: SubtitleLanguage, filter: string): boolean {
  const lower = filter.toLowerCase();
  return (
    lang.code.toLowerCase().includes(lower) ||
    lang.name.toLowerCase().includes(lower) ||
    lang.nativeName.toLowerCase().includes(lower)
  );
}

export const SubtitleLanguagePicker = React.memo(function SubtitleLanguagePicker({
  value,
  onChange,
  className,
  buttonClassName,
  compact = false,
}: SubtitleLanguagePickerProps) {
  const t = useTranslate();
  const pickerRef = React.useRef<HTMLDivElement>(null);
  const floatingPanelRef = React.useRef<HTMLDivElement>(null);
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const [isOpen, setIsOpen] = React.useState(false);
  const [filter, setFilter] = React.useState("");
  const [pickerRect, setPickerRect] = React.useState<DOMRect | null>(null);

  React.useEffect(() => {
    if (!isOpen) {
      setFilter("");
      return;
    }
    if (pickerRef.current && typeof window !== "undefined") {
      setPickerRect(pickerRef.current.getBoundingClientRect());
    }
    requestAnimationFrame(() => {
      searchInputRef.current?.focus();
    });
  }, [isOpen]);

  React.useEffect(() => {
    if (!isOpen) {
      return;
    }
    const handlePointerDown = (event: MouseEvent) => {
      if (
        !pickerRef.current?.contains(event.target as Node) &&
        !floatingPanelRef.current?.contains(event.target as Node)
      ) {
        setIsOpen(false);
      }
    };

    const handleScrollOrResize = () => {
      if (!pickerRef.current || typeof window === "undefined") {
        return;
      }
      setPickerRect(pickerRef.current.getBoundingClientRect());
    };

    document.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("scroll", handleScrollOrResize, true);
    window.addEventListener("resize", handleScrollOrResize, true);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("scroll", handleScrollOrResize, true);
      window.removeEventListener("resize", handleScrollOrResize, true);
    };
  }, [isOpen]);

  const selectedSet = React.useMemo(() => new Set<string>(value), [value]);
  const selectedLabel = React.useMemo(() => {
    if (value.length === 0) {
      return t("settings.sub.languagePickerSelect");
    }
    return value
      .map((code) => getSubtitleLanguage(code)?.name ?? code)
      .join(", ");
  }, [t, value]);

  const filteredLanguages = React.useMemo(
    () =>
      filter.trim()
        ? SUBTITLE_LANGUAGES.filter((lang) => matchesFilter(lang, filter.trim()))
        : SUBTITLE_LANGUAGES,
    [filter],
  );

  const toggleLanguage = (code: string) => {
    const next = new Set(value);
    if (next.has(code)) {
      next.delete(code);
    } else {
      next.add(code);
    }
    onChange(Array.from(next));
  };

  const removeLanguage = (code: string, event: React.MouseEvent) => {
    event.stopPropagation();
    onChange(value.filter((c) => c !== code));
  };

  const floatingPanel =
    isOpen && pickerRect
      ? createPortal(
          <div
            ref={floatingPanelRef}
            className="z-50 max-h-80 overflow-hidden rounded-xl border border-border bg-popover shadow-lg"
            style={{
              position: "fixed",
              top: pickerRect.bottom + 4,
              left: pickerRect.left,
              width: Math.max(320, Math.round(pickerRect.width)),
            }}
          >
            {/* Search input */}
            <div className="border-b border-border p-2">
              <div className="flex items-center gap-2 rounded-md border border-input bg-field px-2 py-1">
                <Search className="h-3.5 w-3.5 text-muted-foreground" />
                <input
                  ref={searchInputRef}
                  type="text"
                  className="w-full bg-transparent text-sm text-foreground placeholder:text-muted-foreground focus:outline-none"
                  placeholder={t("settings.sub.languagePickerSearch")}
                  value={filter}
                  onChange={(event) => setFilter(event.target.value)}
                />
              </div>
            </div>

            {/* Language list */}
            <div className="max-h-64 overflow-y-auto p-2">
              {filteredLanguages.length === 0 ? (
                <p className="px-2 py-3 text-center text-sm text-muted-foreground">
                  {t("settings.sub.languagePickerEmpty")}
                </p>
              ) : (
                <div className="space-y-0.5">
                  {filteredLanguages.map((lang) => (
                    <label
                      key={lang.code}
                      className="flex items-center gap-3 rounded-md px-2 py-1.5 text-sm text-foreground hover:bg-accent/60"
                    >
                      <Checkbox
                        checked={selectedSet.has(lang.code)}
                        onCheckedChange={() => toggleLanguage(lang.code)}
                        aria-label={`${lang.name} (${lang.code})`}
                      />
                      <span className="flex min-w-0 flex-1 items-center gap-2">
                        <span className="truncate">
                          {lang.nativeName}
                          {lang.nativeName !== lang.name ? (
                            <span className="ml-1 text-muted-foreground">
                              {lang.name}
                            </span>
                          ) : null}
                        </span>
                      </span>
                      <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-muted-foreground">
                        {lang.code}
                      </span>
                    </label>
                  ))}
                </div>
              )}
            </div>
          </div>,
          document.body,
        )
      : null;

  return (
    <div ref={pickerRef} className={cn("relative inline-block w-full", className)}>
      <Button
        type="button"
        variant="secondary"
        className={cn(
          "h-auto min-h-10 w-full justify-between gap-2 border border-input bg-field px-3 py-2 text-sm",
          compact && "h-9 min-h-9 py-0",
          buttonClassName,
        )}
        onClick={() => setIsOpen((previous) => !previous)}
        aria-label={t("settings.sub.languagePickerAriaLabel")}
      >
        {compact ? (
          <span className={cn("min-w-0 flex-1 truncate text-left", value.length === 0 && "text-muted-foreground")}>
            {selectedLabel}
          </span>
        ) : (
          <span className="flex min-w-0 flex-1 flex-wrap gap-1">
            {value.length === 0 ? (
              <span className="text-muted-foreground">
                {t("settings.sub.languagePickerSelect")}
              </span>
            ) : (
              value.map((code) => {
                const lang = getSubtitleLanguage(code);
                return (
                  <span
                    key={code}
                    className="inline-flex items-center gap-1 rounded-md bg-primary/15 px-2 py-0.5 text-xs font-medium text-primary"
                  >
                    {lang?.name ?? code}
                    <button
                      type="button"
                      className="ml-0.5 rounded-sm hover:bg-primary/20"
                      onClick={(event) => removeLanguage(code, event)}
                      aria-label={t("settings.sub.languagePickerRemove", {
                        language: lang?.name ?? code,
                      })}
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </span>
                );
              })
            )}
          </span>
        )}
        <ChevronDown
          className={`h-4 w-4 shrink-0 transition-transform ${isOpen ? "rotate-180" : ""}`}
        />
      </Button>
      {floatingPanel}
    </div>
  );
});
