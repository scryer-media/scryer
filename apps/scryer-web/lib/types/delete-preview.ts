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

export type DeleteTitlePreviewResult = {
  titleId: string;
  preview: DeletePreview | null;
  error: string | null;
};

export type DeleteTitlesPreview = {
  preview: DeletePreview;
  items: DeleteTitlePreviewResult[];
  failedCount: number;
};

export type DeleteEpisodeFilePreviewResult = {
  fileId: string;
  episodeId: string;
  error: string | null;
};

export type DeleteEpisodeFilesPreview = {
  preview: DeletePreview;
  items: DeleteEpisodeFilePreviewResult[];
  fileCount: number;
  failedCount: number;
};
