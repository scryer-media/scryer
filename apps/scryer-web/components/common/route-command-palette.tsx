
import * as React from "react";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import type { LucideIcon } from "lucide-react";

export type RouteCommandItem = {
  id: string;
  label: string;
  description: string;
  icon?: LucideIcon;
  keywords?: string[];
  onSelect: () => void;
};

export type RouteCommandPaletteConfig = {
  title: string;
  description: string;
  placeholder: string;
  noResultsText: string;
  groupLabel: string;
  items: RouteCommandItem[];
};

type RouteCommandPaletteProps = {
  config?: RouteCommandPaletteConfig;
};

function isTextInput(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  const tag = target.tagName.toLowerCase();
  return (
    target.isContentEditable ||
    tag === "input" ||
    tag === "textarea" ||
    tag === "select"
  );
}

export function RouteCommandPalette({
  config,
}: RouteCommandPaletteProps) {
  const [open, setOpen] = React.useState(false);
  const lastShiftPressAt = React.useRef(0);

  const handleCommandNavigate = React.useCallback((callback: () => void) => {
    setOpen(false);
    callback();
  }, []);

  React.useEffect(() => {
    if (!config || config.items.length === 0) {
      return undefined;
    }

    const onKeyDown = (event: KeyboardEvent) => {
      // Cmd+K / Ctrl+K
      if (event.key === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        setOpen((prev) => !prev);
        return;
      }

      // Double-Shift
      if (event.key !== "Shift" || event.repeat || isTextInput(event.target)) {
        return;
      }

      const now = performance.now();
      const previousShiftPressAt = lastShiftPressAt.current;

      if (previousShiftPressAt && now - previousShiftPressAt < 300) {
        setOpen(true);
        lastShiftPressAt.current = 0;
        return;
      }

      lastShiftPressAt.current = now;
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      lastShiftPressAt.current = 0;
    };
  }, [config]);

  if (!config || config.items.length === 0) {
    return null;
  }

  return (
    <CommandDialog
      open={open}
      onOpenChange={setOpen}
      title={config.title}
      description={config.description}
      showCloseButton={false}
    >
      <CommandInput placeholder={config.placeholder} />
      <CommandList>
        <CommandEmpty>{config.noResultsText}</CommandEmpty>
        <CommandGroup heading={config.groupLabel}>
          {config.items.map((item) => (
            <CommandItem
              key={item.id}
              value={item.id}
              keywords={item.keywords}
              onSelect={() => handleCommandNavigate(item.onSelect)}
            >
              <div className="flex flex-1 items-center gap-2">
                {item.icon ? <item.icon className="h-4 w-4" /> : null}
                <span className="truncate">{item.label}</span>
                <span className="ml-auto text-xs text-muted-foreground">{item.description}</span>
              </div>
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
