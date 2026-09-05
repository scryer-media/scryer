import { useState } from "react";
import { FolderOpen } from "lucide-react";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { InfoHelp } from "@/components/common/info-help";
import { Button } from "@/components/ui/button";
import { Checkbox, CheckboxField } from "@/components/ui/checkbox";
import { FilterableSelect } from "@/components/ui/filterable-select";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useTranslate } from "@/lib/context/translate-context";
import type { ConfigFieldDef } from "@/lib/types";
import { isFileBackedDownloadClientConfigField } from "@/lib/utils/download-clients";
import { selectorId } from "@/lib/utils/dom-ids";

function splitConfigTagValue(value: string): string[] {
  return value
    .split(/[,\n;]/)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function joinConfigTagValue(values: string[]): string {
  return values.join(",");
}

export function DownloadClientConfigField({
  field,
  value,
  hasStoredSecretValue = false,
  idPrefix = "download-client-field",
  onChange,
  onClearStoredSecret,
}: {
  field: ConfigFieldDef;
  value: string;
  hasStoredSecretValue?: boolean;
  idPrefix?: string;
  onChange: (key: string, value: string) => void;
  onClearStoredSecret?: (key: string) => void;
}) {
  const t = useTranslate();
  const [browserOpen, setBrowserOpen] = useState(false);
  const fieldId = selectorId(idPrefix, field.key);
  const optionIdPrefix = `${idPrefix}-option`;
  const isFileBackedField = isFileBackedDownloadClientConfigField(field);
  const help = field.helpText ? (
    <InfoHelp text={field.helpText} ariaLabel={`About ${field.label}`} />
  ) : null;
  const requiredMarker = field.required ? (
    <span aria-hidden="true" className="text-destructive">
      *
    </span>
  ) : null;

  if (field.fieldType === "BOOL") {
    return (
      <CheckboxField
        id={fieldId}
        checked={value === "true"}
        onCheckedChange={(checked) =>
          onChange(field.key, checked === true ? "true" : "false")
        }
        label={field.label}
        labelAccessory={
          <>
            {requiredMarker}
            {help}
          </>
        }
        className="items-center"
        checkboxClassName="mt-0"
      />
    );
  }

  if (field.fieldType === "SELECT" && field.options.length > 0) {
    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
          {help}
        </Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(next) => onChange(field.key, next)}
        >
          <SelectTrigger id={fieldId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </label>
    );
  }

  if (field.fieldType === "FILTERED_SELECT" && field.options.length > 0) {
    return (
      <div>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
          {help}
        </Label>
        <FilterableSelect
          id={fieldId}
          value={value || field.defaultValue || ""}
          onValueChange={(next) => onChange(field.key, next)}
          ariaLabel={field.label}
          optionIdPrefix={optionIdPrefix}
          options={field.options.map((option) => ({
            value: option.value,
            label: option.label,
          }))}
        />
      </div>
    );
  }

  if (field.fieldType === "TAG" && field.options.length > 0) {
    const selectedValues = splitConfigTagValue(value);
    const selected = new Set(selectedValues);
    const optionValues = new Set(field.options.map((option) => option.value));

    return (
      <div>
        <Label className="mb-2 inline-flex items-center gap-2">
          {field.label}
          {requiredMarker}
          {help}
        </Label>
        <div className="grid gap-2 sm:grid-cols-2">
          {field.options.map((option) => {
            const optionId = selectorId(optionIdPrefix, field.key, option.value);
            return (
              <label
                key={option.value}
                className="flex items-center gap-2 text-sm"
              >
                <Checkbox
                  id={optionId}
                  checked={selected.has(option.value)}
                  onCheckedChange={(checked) => {
                    const next = new Set(selectedValues);
                    if (checked === true) {
                      next.add(option.value);
                    } else {
                      next.delete(option.value);
                    }
                    const orderedOptions = field.options
                      .map((candidate) => candidate.value)
                      .filter((candidate) => next.has(candidate));
                    const customValues = selectedValues.filter(
                      (candidate) =>
                        next.has(candidate) && !optionValues.has(candidate),
                    );
                    onChange(
                      field.key,
                      joinConfigTagValue([...orderedOptions, ...customValues]),
                    );
                  }}
                />
                <span>{option.label}</span>
              </label>
            );
          })}
        </div>
      </div>
    );
  }

  if (field.fieldType === "MULTILINE") {
    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
          {help}
        </Label>
        <Textarea
          id={fieldId}
          value={value}
          onChange={(event) => onChange(field.key, event.target.value)}
          required={field.required && !hasStoredSecretValue}
          placeholder={field.defaultValue ?? ""}
          rows={5}
        />
      </label>
    );
  }

  if (isFileBackedField && field.fieldType !== "NUMBER" && field.fieldType !== "PASSWORD") {
    return (
      <div>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
          {help}
        </Label>
        <div className="flex gap-2">
          <Input
            id={fieldId}
            value={value}
            onChange={(event) => onChange(field.key, event.target.value)}
            required={field.required && !hasStoredSecretValue}
            placeholder={
              hasStoredSecretValue
                ? t("form.apiKeyStoredPlaceholder")
                : field.defaultValue ?? ""
            }
          />
          <Button
            type="button"
            variant="outline"
            className="shrink-0 px-3"
            aria-label={`Browse ${field.label}`}
            title={`Browse ${field.label}`}
            onClick={() => setBrowserOpen(true)}
          >
            <FolderOpen className="h-4 w-4" aria-hidden="true" />
          </Button>
        </div>
        <FolderBrowserDialog
          open={browserOpen}
          onOpenChange={setBrowserOpen}
          selectionTypes={["folder"]}
          initialPath={value || field.defaultValue || "/"}
          title={`Select ${field.label}`}
          onSelect={(path) => onChange(field.key, path)}
        />
      </div>
    );
  }

  return (
    <div>
      <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
        {field.label}
        {requiredMarker}
        {help}
      </Label>
      <div className="flex gap-2">
        <Input
          id={fieldId}
          value={value}
          onChange={(event) => onChange(field.key, event.target.value)}
          {...(field.fieldType === "NUMBER" ? signedIntegerInputProps : {})}
          type={
            field.fieldType === "PASSWORD"
              ? "password"
              : field.fieldType === "NUMBER"
                ? "number"
                : "text"
          }
          required={field.required && !hasStoredSecretValue}
          placeholder={
            hasStoredSecretValue
              ? t("form.apiKeyStoredPlaceholder")
              : field.defaultValue ?? ""
          }
        />
        {field.fieldType === "PASSWORD" && hasStoredSecretValue && onClearStoredSecret ? (
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            onClick={() => onClearStoredSecret(field.key)}
          >
            {t("label.clear")}
          </Button>
        ) : null}
      </div>
    </div>
  );
}
