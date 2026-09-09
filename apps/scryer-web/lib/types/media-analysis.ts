// Typed fields shared by media-file queries and details views.
export type MediaProvenance = "UNKNOWN" | "CONTAINER" | "BITSTREAM" | "OBSERVED" | "ESTIMATED" | "LEGACY";
export type MediaProbeStatus = "UNKNOWN" | "COMPLETE" | "INCOMPLETE" | "UNSUPPORTED" | "ENCRYPTED" | "MALFORMED";
export type MediaStreamKind = "VIDEO" | "AUDIO" | "SUBTITLE";
export type MediaProbeReport = {
  status: MediaProbeStatus;
  bytesRead: number | string;
  seeks: number | string;
  elapsedMs: number | string;
  budgetExhausted: boolean;
  warnings: (MediaProbeWarning)[];
};

export type MediaAnalysisAttempt = {
  revision: number;
  attemptedAt: string;
  succeeded: boolean;
  report: MediaProbeReport;
  disc?: MediaDiscMetadata | null;
};

export type MediaStructuralDiagnostics = {
  report: MediaProbeReport;
  sourceLength: number | string;
  coverage: { offset: number | string; length: number | string }[];
  coverageTruncated: boolean;
  warningsTruncated: boolean;
  packetsSampled: number | string;
  frameHeadersSampled: number | string;
  checks: string[];
};

export type MediaProbeWarning = {
  code: string;
  message: string;
  streamId: string | null;
  offset: number | string | null;
};

export type MediaRational = {
  numerator: number | string;
  denominator: number | string;
};

export type MediaStreamDisposition = {
  default: boolean | null;
  forced: boolean | null;
  original: boolean | null;
  commentary: boolean | null;
  hearingImpaired: boolean | null;
  visualImpaired: boolean | null;
  attachedPicture: boolean | null;
  stillImage: boolean | null;
};

export type MediaColorMetadata = {
  primaries: number | null;
  transfer: number | null;
  matrix: number | null;
  fullRange: boolean | null;
  provenance: MediaProvenance;
  masteringDisplay: MediaMasteringDisplay | null;
  contentLight: MediaContentLight | null;
};

export type MediaMasteringDisplay = {
  redX: number | null;
  redY: number | null;
  greenX: number | null;
  greenY: number | null;
  blueX: number | null;
  blueY: number | null;
  whiteX: number | null;
  whiteY: number | null;
  minLuminance: number | null;
  maxLuminance: number | null;
};

export type MediaContentLight = {
  maxCll: number | null;
  maxFall: number | null;
};

export type MediaDolbyVision = {
  profile: number | null;
  level: number | null;
  baseLayerCompatibilityId: number | null;
  rpuPresent: boolean | null;
  enhancementLayerPresent: boolean | null;
  baseLayerPresent: boolean | null;
};

export type MediaHdrCapabilities = {
  dolbyVision: boolean | null;
  hdr10plus: boolean | null;
  hdr10: boolean | null;
  hlg: boolean | null;
  pq: boolean | null;
  dovi: MediaDolbyVision | null;
};

export type MediaStreamMetadata = {
  id: string | null;
  programId: number | null;
  originalLanguage: string | null;
  languageProvenance: MediaProvenance;
  disposition: MediaStreamDisposition;
  durationSeconds: number | null;
  bitrateBps: number | string | null;
  bitrateProvenance: MediaProvenance;
  estimatedBitrateBps: number | string | null;
  sampleRate: number | null;
  sampleFormat: string | null;
  sampleBitDepth: number | null;
  channelLayout: string | null;
  pixelFormat: string | null;
  profile: string | null;
  level: number | null;
  bitDepth: number | null;
  fieldOrder: string | null;
  sampleAspectRatio: MediaRational | null;
  displayAspectRatio: MediaRational | null;
  rotationDegrees: number | null;
  declaredFrameRate: MediaRational | null;
  observedFrameRate: MediaRational | null;
  variableFrameRate: boolean | null;
  color: MediaColorMetadata;
  hdr: MediaHdrCapabilities;
};

export type MediaStreamDetail = {
  kind: MediaStreamKind;
  codec: string | null;
  width: number | null;
  height: number | null;
  channels: number | null;
  language: string | null;
  name: string | null;
  metadata: MediaStreamMetadata;
};

export type MediaChapter = {
  id: string;
  title: string | null;
  startSeconds: number;
  endSeconds: number | null;
};

export type MediaAttachment = {
  id: string;
  name: string | null;
  mediaType: string | null;
  sizeBytes: number | string;
};

export type MediaCaptionService = {
  streamId: string;
  standard: string;
  serviceNumber: number | null;
  language: string | null;
};

export type MediaProgram = {
  id: number;
  name: string | null;
  streamIds: (string)[];
  durationSeconds: number | null;
};

export type MediaDiscSegment = {
  path: string;
  inSeconds: number;
  outSeconds: number;
  angle: number;
  sequenceId: number | null;
};

export type MediaDiscTitle = {
  id: string;
  aliases: (string)[];
  durationSeconds: number | null;
  angleCount: number;
  segments: (MediaDiscSegment)[];
  chapters: (MediaChapter)[];
  streams: (MediaStreamDetail)[];
  report: MediaProbeReport;
};

export type MediaDiscEpisodeMapping = {
  discTitleId: string;
  episodeIds: (string)[];
};

export type MediaDiscSelection = {
  titleId: string | null;
  episodeMappings: (MediaDiscEpisodeMapping)[];
};

export type MediaDiscMetadata = {
  discType: string;
  filesystem: string;
  volumeLabel: string | null;
  titles: (MediaDiscTitle)[];
  selectedTitleId: string | null;
  automaticSelection: boolean;
  selection: MediaDiscSelection;
};

export type MediaAnalysisDetails = {
  revision: number;
  durationSeconds: number | null;
  durationProvenance: MediaProvenance;
  overallBitrateBps: number | string | null;
  selectedVideoId: string | null;
  selectedProgramId: number | null;
  streams: (MediaStreamDetail)[];
  programs: (MediaProgram)[];
  chapters: (MediaChapter)[];
  attachments: (MediaAttachment)[];
  captionServices: (MediaCaptionService)[];
  disc: MediaDiscMetadata | null;
  report: MediaProbeReport;
};

export const MEDIA_DISC_FIELDS = `
  discType
  filesystem
  volumeLabel
  titles {
    id
    aliases
    durationSeconds
    angleCount
    segments {
      path
      inSeconds
      outSeconds
      angle
      sequenceId
    }
    chapters {
      id
      title
      startSeconds
      endSeconds
    }
    streams {
      kind
      codec
      width
      height
      channels
      language
      name
      metadata {
        id
        programId
        originalLanguage
        languageProvenance
        disposition {
          default
          forced
          original
          commentary
          hearingImpaired
          visualImpaired
          attachedPicture
          stillImage
        }
        durationSeconds
        bitrateBps
        bitrateProvenance
        estimatedBitrateBps
        sampleRate
        sampleFormat
        sampleBitDepth
        channelLayout
        pixelFormat
        profile
        level
        bitDepth
        fieldOrder
        sampleAspectRatio {
          numerator
          denominator
        }
        displayAspectRatio {
          numerator
          denominator
        }
        rotationDegrees
        declaredFrameRate {
          numerator
          denominator
        }
        observedFrameRate {
          numerator
          denominator
        }
        variableFrameRate
        color {
          primaries
          transfer
          matrix
          fullRange
          provenance
          masteringDisplay {
            redX
            redY
            greenX
            greenY
            blueX
            blueY
            whiteX
            whiteY
            minLuminance
            maxLuminance
          }
          contentLight {
            maxCll
            maxFall
          }
        }
        hdr {
          dolbyVision
          hdr10plus: hdr10Plus
          hdr10
          hlg
          pq
          dovi {
            profile
            level
            baseLayerCompatibilityId
            rpuPresent
            enhancementLayerPresent
            baseLayerPresent
          }
        }
      }
    }
    report {
      status
      bytesRead
      seeks
      elapsedMs
      budgetExhausted
      warnings {
        code
        message
        streamId
        offset
      }
    }
  }
  selectedTitleId
  automaticSelection
  selection {
    titleId
    episodeMappings {
      discTitleId
      episodeIds
    }
  }
`;

export const MEDIA_ANALYSIS_FIELDS = `
  revision
  durationSeconds
  durationProvenance
  overallBitrateBps
  selectedVideoId
  selectedProgramId
  streams {
    kind
    codec
    width
    height
    channels
    language
    name
    metadata {
      id
      programId
      originalLanguage
      languageProvenance
      disposition {
        default
        forced
        original
        commentary
        hearingImpaired
        visualImpaired
        attachedPicture
        stillImage
      }
      durationSeconds
      bitrateBps
      bitrateProvenance
      estimatedBitrateBps
      sampleRate
      sampleFormat
      sampleBitDepth
      channelLayout
      pixelFormat
      profile
      level
      bitDepth
      fieldOrder
      sampleAspectRatio {
        numerator
        denominator
      }
      displayAspectRatio {
        numerator
        denominator
      }
      rotationDegrees
      declaredFrameRate {
        numerator
        denominator
      }
      observedFrameRate {
        numerator
        denominator
      }
      variableFrameRate
      color {
        primaries
        transfer
        matrix
        fullRange
        provenance
        masteringDisplay {
          redX
          redY
          greenX
          greenY
          blueX
          blueY
          whiteX
          whiteY
          minLuminance
          maxLuminance
        }
        contentLight {
          maxCll
          maxFall
        }
      }
      hdr {
        dolbyVision
        hdr10plus: hdr10Plus
        hdr10
        hlg
        pq
        dovi {
          profile
          level
          baseLayerCompatibilityId
          rpuPresent
          enhancementLayerPresent
          baseLayerPresent
        }
      }
    }
  }
  programs {
    id
    name
    streamIds
    durationSeconds
  }
  chapters {
    id
    title
    startSeconds
    endSeconds
  }
  attachments {
    id
    name
    mediaType
    sizeBytes
  }
  captionServices {
    streamId
    standard
    serviceNumber
    language
  }
  disc { ${MEDIA_DISC_FIELDS} }
  report {
    status
    bytesRead
    seeks
    elapsedMs
    budgetExhausted
    warnings {
      code
      message
      streamId
      offset
    }
  }
`;
