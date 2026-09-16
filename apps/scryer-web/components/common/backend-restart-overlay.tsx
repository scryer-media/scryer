import { LoadingMark } from "@/components/common/loading-mark";

export function BackendRestartOverlay() {
  return (
    <div
      id="backend-restart-overlay"
      className="fixed inset-0 z-[9999] grid place-items-center bg-background"
    >
      <div className="text-center">
        <h1
          className="mb-8 text-3xl font-bold tracking-tight text-foreground"
          style={{ fontFamily: "'Space Grotesk Variable', 'Inter Variable', ui-sans-serif, system-ui, sans-serif" }}
        >
          scryer
        </h1>
        <LoadingMark className="mx-auto mb-6 block size-10" />
        <p id="backend-restart-status" className="text-sm text-muted-foreground">Service is restarting&hellip;</p>
      </div>
    </div>
  );
}
