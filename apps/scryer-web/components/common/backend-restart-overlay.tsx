import { Loader2 } from "lucide-react";

export function BackendRestartOverlay() {
  return (
    <div className="fixed inset-0 z-[9999] flex flex-col items-center justify-center bg-[#070b18]">
      <div className="flex flex-col items-center">
        <h1
          className="mb-8 text-3xl font-bold tracking-tight text-[#dbe5ff]"
          style={{ fontFamily: "'Space Grotesk', Inter, ui-sans-serif, system-ui, sans-serif" }}
        >
          scryer
        </h1>
        <Loader2 className="mb-6 size-7 animate-spin text-[#5b64ff]" />
        <p className="text-sm text-[#8b96b9]">Service is restarting&hellip;</p>
      </div>
    </div>
  );
}
