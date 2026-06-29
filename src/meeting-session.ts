import type {
  AnalysisResult,
  LiveNudgeEvent,
  MetricsCalculationResult,
  RecordingMetadata,
  TranscriptionResult,
  TranscriptStreamEvent,
  TranscriptStreamSummary,
} from './contracts';
import { messageFromUnknownError } from './error-utils';

export interface TranscriptStreamListeners {
  onSegment: (event: TranscriptStreamEvent) => void;
  onComplete: (summary: TranscriptStreamSummary) => void;
}

export interface LiveNudgeListeners {
  onNudge: (event: LiveNudgeEvent) => void;
}

export interface CompleteMeetingSessionDeps {
  stopRecording: () => Promise<RecordingMetadata>;
  listenToTranscriptStream: (listeners: TranscriptStreamListeners) => Promise<() => void>;
  listenToLiveNudges: (listeners: LiveNudgeListeners) => Promise<() => void>;
  transcribeMeeting: (meetingId: string) => Promise<TranscriptionResult>;
  calculateMetrics: (meetingId: string) => Promise<MetricsCalculationResult>;
  analyzeMeeting: (meetingId: string) => Promise<AnalysisResult>;
}

export type MeetingReviewPhase = 'transcribing' | 'calculatingMetrics' | 'analyzing';

export interface CompleteMeetingSessionCallbacks {
  onPhaseChange?: (phase: MeetingReviewPhase) => void;
  onRecordingSaved?: (recording: RecordingMetadata) => void;
  onTranscriptionReady?: (transcription: TranscriptionResult) => void;
  onMetricsReady?: (metrics: MetricsCalculationResult) => void;
  listeners?: {
    onSegment?: (event: TranscriptStreamEvent) => void;
    onComplete?: (summary: TranscriptStreamSummary) => void;
    onNudge?: (event: LiveNudgeEvent) => void;
  };
}

export interface CompleteMeetingSessionResult {
  recording: RecordingMetadata;
  transcription: TranscriptionResult;
  metrics: MetricsCalculationResult;
  analysis: AnalysisResult;
}

export type MeetingReviewFailurePhase = 'stop' | MeetingReviewPhase;

export class MeetingReviewError extends Error {
  phase: MeetingReviewFailurePhase;
  recording: RecordingMetadata | null;
  override cause: unknown;

  constructor(phase: MeetingReviewFailurePhase, recording: RecordingMetadata | null, cause: unknown) {
    super(messageFromUnknownError(cause, 'Meeting review failed.'));
    this.name = 'MeetingReviewError';
    this.phase = phase;
    this.recording = recording;
    this.cause = cause;
  }
}

export const completeMeetingSession = async (
  deps: CompleteMeetingSessionDeps,
  callbacks: CompleteMeetingSessionCallbacks = {},
): Promise<CompleteMeetingSessionResult> => {
  const recording = await deps.stopRecording().catch((error: unknown) => {
    throw new MeetingReviewError('stop', null, error);
  });
  callbacks.onRecordingSaved?.(recording);

  const cleanups: Array<() => void> = [];
  const cleanupListeners = (): void => {
    for (const cleanup of cleanups) {
      cleanup();
    }
  };
  const meetingId = recording.meetingId;

  try {
    cleanups.push(
      await deps
        .listenToTranscriptStream({
          onSegment: (event) => {
            if (event.meetingId === meetingId) {
              callbacks.listeners?.onSegment?.(event);
            }
          },
          onComplete: (summary) => {
            if (summary.meetingId === meetingId) {
              callbacks.listeners?.onComplete?.(summary);
            }
          },
        })
        .catch((error: unknown) => {
          throw new MeetingReviewError('transcribing', recording, error);
        }),
    );
    cleanups.push(
      await deps
        .listenToLiveNudges({
          onNudge: (event) => {
            if (event.meetingId === meetingId) {
              callbacks.listeners?.onNudge?.(event);
            }
          },
        })
        .catch((error: unknown) => {
          throw new MeetingReviewError('transcribing', recording, error);
        }),
    );

    callbacks.onPhaseChange?.('transcribing');
    const transcription = await deps.transcribeMeeting(meetingId).catch((error: unknown) => {
      throw new MeetingReviewError('transcribing', recording, error);
    });
    callbacks.onTranscriptionReady?.(transcription);

    callbacks.onPhaseChange?.('calculatingMetrics');
    const metrics = await deps.calculateMetrics(meetingId).catch((error: unknown) => {
      throw new MeetingReviewError('calculatingMetrics', recording, error);
    });
    callbacks.onMetricsReady?.(metrics);

    callbacks.onPhaseChange?.('analyzing');
    const analysis = await deps.analyzeMeeting(meetingId).catch((error: unknown) => {
      throw new MeetingReviewError('analyzing', recording, error);
    });

    return {
      recording,
      transcription,
      metrics,
      analysis,
    };
  } finally {
    cleanupListeners();
  }
};
