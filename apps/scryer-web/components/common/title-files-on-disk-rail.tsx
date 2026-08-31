import type { ReactNode } from "react";
import { HardDrive } from "lucide-react";
import {
  TitleWorkspaceSectionCard,
  TitleWorkspaceSectionHeader,
} from "@/components/views/media-content/title-workspace-primitives";
import { useTranslate } from "@/lib/context/translate-context";

type TitleFilesOnDiskRailProps = {
  children: ReactNode;
  action?: ReactNode;
  footer?: ReactNode;
  variant?: "panel" | "workspace";
};

export function TitleFilesOnDiskRail({
  children,
  action,
  footer,
  variant = "panel",
}: TitleFilesOnDiskRailProps) {
  const t = useTranslate();

  if (variant === "workspace") {
    return (
      <TitleWorkspaceSectionCard>
        <TitleWorkspaceSectionHeader
          icon={HardDrive}
          title={t("title.files")}
          action={action}
        />
        <div className="border-t border-[var(--scry-line3)] pt-3">{children}</div>
        {footer}
      </TitleWorkspaceSectionCard>
    );
  }

  return (
    <section className="space-y-3 rounded-lg border border-border/70 bg-card/60 p-4">
      <div className="flex items-center gap-2">
        <span className="flex size-8 items-center justify-center rounded-lg bg-primary/15 text-primary">
          <HardDrive className="h-4 w-4" />
        </span>
        <h2 className="text-sm font-semibold uppercase tracking-[0.08em] text-muted-foreground">
          {t("title.files")}
        </h2>
        {action ? <div className="ml-auto shrink-0">{action}</div> : null}
      </div>
      {children}
      {footer}
    </section>
  );
}
