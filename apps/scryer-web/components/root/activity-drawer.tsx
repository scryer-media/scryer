
import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { ActivityEvent, Translate } from "@/components/root/types";

type ActivityDrawerProps = {
  t: Translate;
  isOpen: boolean;
  activityToasts: ActivityEvent[];
  onClose: () => void;
  activitySeverityClass: (severity: string) => string;
};

export function ActivityDrawer({
  t,
  isOpen,
  activityToasts,
  onClose,
  activitySeverityClass,
}: ActivityDrawerProps) {
  if (!isOpen) {
    return null;
  }

  return (
    <section className="fixed inset-y-0 right-0 z-40 w-80 border-l border-border bg-background/95 shadow-xl">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <h2 className="text-sm font-semibold text-foreground">{t("activity.title")}</h2>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={onClose}
          className="h-7 px-2"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>
      <div className="h-[calc(100%-2.625rem)] space-y-2 overflow-y-auto p-2">
        {activityToasts.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("activity.noActivity")}</p>
        ) : (
          activityToasts.map((event) => (
            <Card
              key={event.id}
              className={`border ${activitySeverityClass(event.severity ?? "info")}`}
            >
              <CardHeader>
                <CardTitle className="text-sm">
                  <span>{event.kind}</span>
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-sm text-card-foreground">{event.message}</p>
                <p className="mt-1 text-xs text-muted-foreground">
                  {event.actorUserId
                    ? t("activity.actor", { actor: event.actorUserId })
                    : t("activity.actorUnknown")}
                  • {event.occurredAt ?? t("activity.timeUnknown")}
                </p>
              </CardContent>
            </Card>
          ))
        )}
      </div>
    </section>
  );
}
