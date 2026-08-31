import * as React from "react";
import { buttonVariants } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ChevronDown, Search, X } from "lucide-react";
import {
  SUBTITLE_LANGUAGES,
  type SubtitleLanguage,
} from "@/lib/constants/subtitle-languages";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

export type SubtitleLanguagePickerProps = {
  value: string[];
  onChange: (codes: string[]) => void;
  languageOptions?: SubtitleLanguage[];
  className?: string;
  buttonClassName?: string;
  compact?: boolean;
  disabled?: boolean;
  singleSelect?: boolean;
  triggerId?: string;
  panelId?: string;
  searchInputId?: string;
  optionIdPrefix?: string;
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
  languageOptions = SUBTITLE_LANGUAGES,
  className,
  buttonClassName,
  compact = false,
  disabled = false,
  singleSelect = false,
  triggerId,
  panelId,
  searchInputId,
  optionIdPrefix,
}: SubtitleLanguagePickerProps) {
  const t = useTranslate();
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const [isOpen, setIsOpen] = React.useState(false);
  const [filter, setFilter] = React.useState("");

  React.useEffect(() => {
    if (!isOpen) {
      setFilter("");
    }
  }, [isOpen]);

  React.useEffect(() => {
    if (disabled) {
      setIsOpen(false);
    }
  }, [disabled]);

  const selectedSet = React.useMemo(() => new Set<string>(value), [value]);
  const languageByCode = React.useMemo(
    () => new Map(languageOptions.map((language) => [language.code, language])),
    [languageOptions],
  );
  const selectedLabel = React.useMemo(() => {
    if (value.length === 0) {
      return t("settings.sub.languagePickerSelect");
    }
    return value
      .map((code) => languageByCode.get(code)?.name ?? code)
      .join(", ");
  }, [languageByCode, t, value]);

  const filteredLanguages = React.useMemo(
    () =>
      filter.trim()
        ? languageOptions.filter((lang) => matchesFilter(lang, filter.trim()))
        : languageOptions,
    [filter, languageOptions],
  );

  React.useEffect(() => {
    if (!isOpen || typeof window === "undefined") {
      return;
    }
    const frame = window.requestAnimationFrame(() => {
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [isOpen]);

  const toggleLanguage = (code: string) => {
    if (singleSelect) {
      onChange([code]);
      setIsOpen(false);
      return;
    }

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

  const handleTriggerKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (disabled) {
        return;
      }
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        setIsOpen((previous) => !previous);
      } else if (event.key === "Escape") {
        event.preventDefault();
        setIsOpen(false);
      }
    },
    [disabled],
  );

  const floatingPanel =
    isOpen
      ? (
          <PopoverContent
            id={panelId}
            className="z-[90] flex max-h-80 w-[var(--radix-popover-trigger-width)] min-w-[20rem] flex-col overflow-hidden rounded-xl border border-border bg-popover p-0 shadow-lg"
            align="end"
            side="bottom"
            sideOffset={8}
          >
            {/* Search input */}
            <div className="border-b border-border p-2">
              <div className="flex items-center gap-2 rounded-md border border-input bg-field px-2 py-1">
                <Search className="h-3.5 w-3.5 text-muted-foreground" />
                <input
                  id={searchInputId}
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
            <div className="min-h-0 flex-1 overflow-y-auto p-2">
              {filteredLanguages.length === 0 ? (
                <p className="px-2 py-3 text-center text-sm text-muted-foreground">
                  {t("settings.sub.languagePickerEmpty")}
                </p>
              ) : (
                <div className="space-y-0.5">
                  {filteredLanguages.map((lang) => (
                    <label
                      key={lang.code}
                      id={optionIdPrefix ? `${optionIdPrefix}-${lang.code}` : undefined}
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
                      <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 font-[var(--font-code)] text-xs text-muted-foreground">
                        {lang.code}
                      </span>
                    </label>
                  ))}
                </div>
              )}
            </div>
          </PopoverContent>
        )
      : null;

  return (
    <div className={cn("inline-block w-full", className)}>
      <Popover open={isOpen} onOpenChange={setIsOpen}>
        <PopoverTrigger asChild>
        <div
          id={triggerId}
          role="button"
          tabIndex={disabled ? -1 : 0}
          aria-disabled={disabled}
          aria-expanded={isOpen}
          aria-haspopup="dialog"
          aria-controls={panelId}
          className={cn(
            buttonVariants({ variant: "secondary" }),
            "h-auto min-h-10 w-full justify-between gap-2 border border-input bg-field px-3 py-2 text-sm",
            "cursor-pointer",
            compact && "h-9 min-h-9 py-0",
            disabled && "pointer-events-none cursor-not-allowed opacity-50",
            buttonClassName,
          )}
          onKeyDown={handleTriggerKeyDown}
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
                const lang = languageByCode.get(code);
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
        </div>
        </PopoverTrigger>
        {floatingPanel}
      </Popover>
    </div>
  );
});
