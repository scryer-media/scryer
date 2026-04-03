export type DeletePreview = {
  fingerprint: string;
  totalFileCount: number;
  mediaCount: number;
  subtitleCount: number;
  imageCount: number;
  otherCount: number;
  directoryCount: number;
  requiresTypedConfirmation: boolean;
  typedConfirmationPrompt: string | null;
  targetLabel: string;
  samplePaths: string[];
};
