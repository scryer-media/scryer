/** Public, dependency-free contract for Arr custom-format imports. */
export type ArrSource = "sonarr" | "radarr";

export type DiagnosticLevel = "error" | "warning" | "info";

export type Diagnostic = {
  code: string;
  level: DiagnosticLevel;
  message: string;
  /** JSON-path-like location in the pasted document, when available. */
  location?: string;
  formatId?: string;
  specificationIndex?: number;
};

export type ArrSpecification = {
  implementation: string;
  name: string;
  negate: boolean;
  required: boolean;
  fields: Record<string, unknown>;
  index: number;
};

export type ImportedCustomFormat = {
  id: string;
  name: string;
  source: ArrSource;
  specifications: ArrSpecification[];
  suggestedScores: Record<string, number>;
  description?: string;
  /** Structural import errors retained so the compiler emits a disabled stub. */
  inspectionDiagnostics?: Diagnostic[];
};

export type InspectionResult = {
  formats: ImportedCustomFormat[];
  diagnostics: Diagnostic[];
  /** A malformed document has no safe selection for the translation dialog. */
  fatal: boolean;
};

export type TranslatedFormat = {
  id: string;
  name: string;
  status: "translated" | "disabled";
  diagnostics: Diagnostic[];
};

export type TranslationResult = {
  regoSource: string;
  name: string;
  description: string;
  appliedFacets: string[];
  formats: TranslatedFormat[];
  diagnostics: Diagnostic[];
};
