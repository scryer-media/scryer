import { useState } from "react";
import type { CSSProperties } from "react";
import {
  CircleAlert,
  CircleCheckBig,
  ExternalLink,
  KeyRound,
  Pencil,
  Plus,
  X,
} from "lucide-react";

import { AddNewButton } from "@/components/common/add-new-button";
import { selectorToken } from "@/lib/utils/dom-ids";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import {
  instancePillColors,
  productLogoUrl,
} from "@/components/setup/import/import-instance-pill";
import type {
  ImportInstance,
  ImportInstanceKind,
  UseExternalImportSetupReturn,
} from "@/lib/hooks/use-external-import-setup";
import { LoadingMark } from "@/components/common/loading-mark";

interface SetupImportConnectViewProps {
  wizard: UseExternalImportSetupReturn;
  t: (key: string, values?: Record<string, unknown>) => string;
}

interface KindColumn {
  kind: ImportInstanceKind;
  /** Brand-ish heading accent for the product column (design spec §3). */
  headingColor: string;
  /** Literal brand product name (acceptable per task — these are brand names). */
  productName: string;
  urlPlaceholder: string;
}

const KIND_COLUMNS: readonly KindColumn[] = [
  {
    kind: "SONARR",
    headingColor: "#56b6f0",
    productName: "Sonarr",
    urlPlaceholder: "http://localhost:8989",
  },
  {
    kind: "RADARR",
    headingColor: "#e7b94a",
    productName: "Radarr",
    urlPlaceholder: "http://localhost:7878",
  },
  {
    kind: "PROWLARR",
    headingColor: "#a987ff",
    productName: "Prowlarr",
    urlPlaceholder: "http://localhost:9696",
  },
];

const FIELD_LABEL_STYLE: CSSProperties = {
  display: "block",
  marginBottom: 5,
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: "0.08em",
  textTransform: "uppercase",
  color: "var(--scry-faint2)",
};

const MONO_INPUT_CLASS =
  "h-[38px] rounded-md font-[var(--font-code)] text-[12.5px] text-[#dbe6fb]";

export function SetupImportConnectView({
  wizard,
  t,
}: SetupImportConnectViewProps) {
  const {
    instancesByKind,
    addInstance,
    removeInstance,
    setInstanceName,
    setInstanceConnectionField,
    verifyInstance,
    connectionReady,
  } = wizard;

  /** Fire verification on blur of URL/key — only when the connection is ready. */
  const handleVerifyBlur = (inst: ImportInstance) => {
    if (connectionReady(inst)) void verifyInstance(inst.id);
  };

  return (
    <div id="setup-import-connect-view" data-slot="import-connect-view" className="w-full">
      {wizard.secretDraftOwnedByOther ? (
        <div
          role="status"
          className="mb-4 flex items-start gap-2 rounded-lg border px-3 py-2 text-xs"
          style={{
            background: "var(--scry-warning-bg)",
            borderColor: "var(--scry-warning-border)",
            color: "var(--scry-warning-text)",
          }}
        >
          <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>{t("setup.secretDraftOtherUserWarning")}</span>
        </div>
      ) : null}
      <div
        data-r="conngrid"
        className="grid items-start gap-4"
        style={{
          marginTop: 26,
          gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
        }}
      >
        {KIND_COLUMNS.map((column) => {
          const instances = instancesByKind(column.kind);
          return (
            <div
              key={column.kind}
              className="flex flex-col gap-3"
              data-slot="import-connect-column"
              data-kind={column.kind}
            >
              {/* Column header — product logo + name (design spec §3). */}
              <div className="flex flex-col items-center gap-2.5 text-center">
                <img
                  src={productLogoUrl(column.kind) ?? undefined}
                  alt=""
                  aria-hidden
                  style={{ width: 64, height: 64, objectFit: "contain" }}
                />
                <span
                  style={{
                    fontFamily: "var(--font-space-grotesk)",
                    fontWeight: 700,
                    fontSize: 20,
                    color: "#fff",
                  }}
                >
                  {column.productName}
                </span>
              </div>

              {instances.map((inst, index) => (
                <InstanceCard
                  key={inst.id}
                  inst={inst}
                  instanceIndex={index}
                  urlPlaceholder={column.urlPlaceholder}
                  t={t}
                  onName={(name) => setInstanceName(inst.id, name)}
                  onRemove={() => removeInstance(inst.id)}
                  onField={(field, value) =>
                    setInstanceConnectionField(inst.id, field, value)
                  }
                  onVerifyBlur={() => handleVerifyBlur(inst)}
                />
              ))}

              <AddNewButton
                id={`setup-import-${selectorToken(column.kind)}-add-instance`}
                data-slot="import-add-instance"
                icon={Plus}
                label={t("setup.addInstance", { product: column.productName })}
                onClick={() => addInstance(column.kind)}
                className="h-10 text-[13px]"
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** Deep-link to an instance's API-key settings page (Settings → General). */
function settingsUrl(rawBaseUrl: string): string | null {
  let url = rawBaseUrl.trim().replace(/\/+$/, "");
  if (!url) return null;
  if (!/^https?:\/\//i.test(url)) url = `http://${url}`;
  try {
    new URL(url);
    return `${url}/settings/general`;
  } catch {
    return null;
  }
}

interface InstanceCardProps {
  inst: ImportInstance;
  instanceIndex: number;
  urlPlaceholder: string;
  t: (key: string, values?: Record<string, unknown>) => string;
  onName: (name: string) => void;
  onRemove: () => void;
  onField: (field: "baseUrl" | "apiKey", value: string) => void;
  onVerifyBlur: () => void;
}

function InstanceCard({
  inst,
  instanceIndex,
  urlPlaceholder,
  t,
  onName,
  onRemove,
  onField,
  onVerifyBlur,
}: InstanceCardProps) {
  const [editingName, setEditingName] = useState(false);
  const colors = instancePillColors(inst.kind);
  const apiKeyHelpUrl = settingsUrl(inst.baseUrl);

  const named = inst.name.trim().length > 0;
  const nameDisplay = named ? inst.name : t("setup.unnamedInstance");
  const fieldId = (field: "url" | "api-key") =>
    instanceIndex === 0
      ? `setup-import-${selectorToken(inst.kind)}-${field}`
      : `setup-import-${selectorToken(inst.kind)}-${instanceIndex + 1}-${field}`;

  return (
    <div
      id={`setup-import-${selectorToken(inst.kind)}-${instanceIndex + 1}-instance`}
      data-slot="import-instance-card"
      data-kind={inst.kind}
      data-instance-index={instanceIndex}
      className="flex flex-col gap-[11px]"
      style={{
        border: "1px solid var(--scry-border)",
        borderRadius: 14,
        background: "rgba(10, 17, 32, 0.5)",
        padding: "13px 14px 14px",
      }}
    >
      {/* Top row: status dot + name (+ rename) + remove. */}
      <div className="flex items-center gap-2">
        <span
          aria-hidden
          style={{
            width: 7,
            height: 7,
            borderRadius: "50%",
            flex: "none",
            background: statusDotColor(inst.status, colors.dot),
            boxShadow: `0 0 0 3px ${colors.bg}`,
          }}
        />
        {editingName ? (
          <Input
            autoFocus
            value={inst.name}
            placeholder={t("setup.instanceName")}
            onChange={(e) => onName(e.target.value)}
            onBlur={() => setEditingName(false)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === "Escape") {
                e.currentTarget.blur();
              }
            }}
            className="h-7 flex-1 rounded-md text-[13.5px] font-semibold text-white"
            style={{
              border: "1px solid var(--scry-accent)",
              background: "var(--scry-chip)",
            }}
          />
        ) : (
          <span
            className="flex-1 truncate"
            title={nameDisplay}
            style={{
              fontSize: 13.5,
              fontWeight: 600,
              color: named ? "#eaf1ff" : "var(--scry-faint)",
            }}
          >
            {nameDisplay}
          </span>
        )}
        {!editingName ? (
          <IconButton
            type="button"
            onClick={() => setEditingName(true)}
            label={t("setup.renameInstance")}
            appearance="ghost"
            tone="accent"
            className="h-6 w-6 flex-none rounded-[7px] text-[var(--scry-faint)] hover:text-[var(--scry-accent-text)]"
          >
            <Pencil className="h-3.5 w-3.5" />
          </IconButton>
        ) : null}
        <IconButton
          type="button"
          onClick={onRemove}
          label={t("setup.removeInstance")}
          appearance="ghost"
          tone="delete"
          className="ml-auto h-[26px] w-[26px] flex-none rounded-[7px] text-[var(--scry-faint)] hover:text-[var(--scry-danger-text-soft)]"
        >
          <X className="h-3.5 w-3.5" />
        </IconButton>
      </div>

      {/* URL field. */}
      <div>
        <label style={FIELD_LABEL_STYLE}>URL</label>
        <Input
          id={fieldId("url")}
          value={inst.baseUrl}
          spellCheck={false}
          autoComplete="off"
          placeholder={urlPlaceholder}
          onChange={(e) => onField("baseUrl", e.target.value)}
          onBlur={onVerifyBlur}
          className={MONO_INPUT_CLASS}
        />
      </div>

      {/* API Key field. */}
      <div>
        <div className="mb-[5px] flex items-center justify-between gap-3">
          <label style={{ ...FIELD_LABEL_STYLE, marginBottom: 0 }}>
            API Key
          </label>
          {apiKeyHelpUrl ? (
            <a
              href={apiKeyHelpUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 text-[11px] text-primary hover:underline"
            >
              {t("setup.findApiKey")}
              <ExternalLink className="h-3 w-3" />
            </a>
          ) : null}
        </div>
        <Input
          id={fieldId("api-key")}
          type="password"
          ignorePasswordManagers
          value={inst.apiKey}
          spellCheck={false}
          placeholder={t("setup.apiKeyHelpHint")}
          onChange={(e) => onField("apiKey", e.target.value)}
          onBlur={onVerifyBlur}
          className={MONO_INPUT_CLASS}
        />
      </div>

      {/* Status row. */}
      <StatusRow inst={inst} t={t} />
    </div>
  );
}

function StatusRow({
  inst,
  t,
}: {
  inst: ImportInstance;
  t: (key: string, values?: Record<string, unknown>) => string;
}) {
  return (
    <div
      className="flex items-center gap-1.5"
      style={{ minHeight: 30, fontSize: 13 }}
    >
      {inst.status === "connected" ? (
        <span
          className="flex items-center gap-1.5"
          style={{ color: "var(--scry-success-text-soft)", fontWeight: 600 }}
        >
          <CircleCheckBig style={{ width: 15, height: 15 }} />
          {t("setup.connected")}
          {inst.version ? (
            <span style={{ color: "var(--scry-faint)", fontWeight: 500 }}>
              {inst.version}
            </span>
          ) : null}
        </span>
      ) : inst.status === "testing" ? (
        <span
          className="flex items-center gap-1.5"
          style={{ color: "var(--scry-accent-text)" }}
        >
          <LoadingMark className="size-[15px]" />
          {t("setup.testing")}
        </span>
      ) : inst.status === "error" ? (
        <span className="flex flex-col gap-1">
          <span
            className="flex items-center gap-1.5"
            style={{ color: "var(--scry-danger-text-soft)" }}
          >
            <CircleAlert style={{ width: 15, height: 15 }} />
            {t("setup.couldntConnect")}
          </span>
          {inst.error ? (
            <span
              className="text-xs"
              style={{ color: "var(--scry-faint)", lineHeight: 1.4 }}
            >
              {inst.error}
            </span>
          ) : null}
        </span>
      ) : (
        <span
          className="flex items-center gap-1.5"
          style={{ color: "var(--scry-faint)" }}
        >
          <KeyRound style={{ width: 14, height: 14 }} />
          {t("setup.enterUrlAndKey")}
        </span>
      )}
    </div>
  );
}

function statusDotColor(
  status: ImportInstance["status"],
  idleColor: string,
): string {
  switch (status) {
    case "connected":
      return "var(--scry-success-text-soft)";
    case "testing":
      return "var(--scry-accent)";
    case "error":
      return "var(--scry-danger-text-soft)";
    default:
      return idleColor;
  }
}

export default SetupImportConnectView;
