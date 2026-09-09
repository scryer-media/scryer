import { useEffect, useRef, useState, type FormEvent } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";
import { isSolverProxyProvider, type ProxyRecord } from "@/lib/types";

export type ProxyUrlTestResult = {
  ok: boolean;
  message: string | null;
  observedIp: string | null;
  httpStatus: number | null;
  durationMs: number | null;
};

export function ProxyTestForm({
  proxy,
  onTest,
  onClose,
}: {
  proxy: ProxyRecord;
  onTest: (proxy: ProxyRecord, url: string) => Promise<ProxyUrlTestResult>;
  onClose: () => void;
}) {
  const t = useTranslate();
  const [url, setUrl] = useState("https://api64.ipify.org?format=json");
  const [isTesting, setIsTesting] = useState(false);
  const [result, setResult] = useState<ProxyUrlTestResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const isSolver = isSolverProxyProvider(proxy.providerType);
  const formRef = useRef<HTMLFormElement>(null);

  useEffect(() => {
    formRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }, []);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (isTesting) return;
    setIsTesting(true);
    setResult(null);
    setError(null);
    try {
      setResult(await onTest(proxy, url.trim()));
    } catch (error) {
      setError(
        error instanceof Error ? error.message : t("status.proxyTestFailed"),
      );
    } finally {
      setIsTesting(false);
    }
  }

  return (
    <Card id="settings-proxy-test-form">
      <CardHeader>
        <CardTitle>
          {t("settings.proxyTestTitle", { name: proxy.name })}
        </CardTitle>
        <p className="text-sm text-muted-foreground">
          {t(
            isSolver
              ? "settings.proxyTestSolverDescription"
              : "settings.proxyTestDescription",
          )}
        </p>
      </CardHeader>
      <CardContent>
        <form
          ref={formRef}
          onSubmit={(event) => void submit(event)}
          className="max-w-2xl space-y-4"
        >
          <div className="space-y-2">
            <Label htmlFor="settings-proxy-test-url">
              {t("settings.proxyTestUrl")}
            </Label>
            <Input
              id="settings-proxy-test-url"
              type="url"
              pattern="https?://.*"
              required
              value={url}
              disabled={isTesting}
              onChange={(event) => {
                setUrl(event.target.value);
                setResult(null);
                setError(null);
              }}
            />
          </div>
          <div aria-live="polite" aria-busy={isTesting}>
            {isTesting && (
              <p className="text-sm text-muted-foreground">
                {t("settings.proxyTestRunning")}
              </p>
            )}
            {error && (
              <p role="alert" className="break-words text-sm text-destructive">
                {error}
              </p>
            )}
            {result && (
              <div className="space-y-2 rounded-md border border-border p-3">
                {result.observedIp && (
                  <div>
                    <p className="text-xs text-muted-foreground">
                      {t(
                        isSolver
                          ? "settings.proxyTestSolverIp"
                          : "settings.proxyTestExitIp",
                      )}
                    </p>
                    <p className="break-all font-mono text-lg font-semibold select-text">
                      {result.observedIp}
                    </p>
                  </div>
                )}
                <p
                  className={result.ok ? "text-sm" : "text-sm text-destructive"}
                >
                  {result.ok
                    ? t(
                        result.observedIp
                          ? "status.proxyTestPassed"
                          : "settings.proxyTestNoIp",
                      )
                    : result.message || t("status.proxyTestFailed")}
                </p>
                <div className="flex gap-3 text-xs text-muted-foreground">
                  {result.httpStatus !== null && (
                    <span>HTTP {result.httpStatus}</span>
                  )}
                  {result.durationMs !== null && (
                    <span>{result.durationMs} ms</span>
                  )}
                </div>
              </div>
            )}
          </div>
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="secondary"
              disabled={isTesting}
              onClick={onClose}
            >
              {t("label.close")}
            </Button>
            <Button
              id="settings-proxy-test-submit"
              type="submit"
              disabled={isTesting || !url.trim()}
            >
              {isTesting && <Loader2 className="mr-2 size-4 animate-spin" />}
              {t("settings.proxyTest")}
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  );
}
