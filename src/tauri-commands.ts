import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AppStatus,
  AudioDevice,
  LiveNudgeEvent,
  MeetingHistoryDetail,
  MeetingHistoryPage,
  MeetingNotesResult,
  MeetingTrendsResult,
  MetricsCalculationResult,
  PrivacySettingsUpdateResult,
  RecordingMetadata,
  RecordingStarted,
  ResonanceSettings,
  TranscriptionResult,
  TranscriptStreamEvent,
  TranscriptStreamSummary,
} from './contracts';

export const TRANSCRIPT_SEGMENT_EVENT = 'resonance://transcript-segment';
export const TRANSCRIPT_STREAM_COMPLETE_EVENT = 'resonance://transcript-stream-complete';
export const LIVE_NUDGE_EVENT = 'resonance://live-nudge';

export type {
  AppStatus,
  AudioDevice,
  LiveNudgeEvent,
  MeetingActionItem,
  MeetingHistoryDetail,
  MeetingHistoryItem,
  MeetingHistoryPage,
  MeetingNotesResult,
  MeetingSummary,
  MeetingTrendPoint,
  MeetingTrendsResult,
  MetricsCalculationResult,
  PipelineFailureRecord,
  PrivacySettingsUpdateResult,
  RecordingMetadata,
  RecordingStarted,
  ResonanceSettings,
  TranscriptionResult,
  TranscriptSegment,
  TranscriptStreamEvent,
  TranscriptStreamSummary,
} from './contracts';

interface TauriInternalsWindow {
  __TAURI_INTERNALS__?: {
    invoke?: unknown;
  };
}

export const isTauriRuntime = (): boolean => {
  const tauriInternals = (globalThis as TauriInternalsWindow).__TAURI_INTERNALS__;
  return typeof tauriInternals?.invoke === 'function';
};

const assertTauriRuntime = (): void => {
  if (!isTauriRuntime()) {
    throw new Error(
      'Open Scribe from the native window with `bun run tauri dev`; this page cannot reach the recording commands from a normal browser tab.',
    );
  }
};

const invokeNative = async <Response>(command: string, args?: Record<string, unknown>): Promise<Response> => {
  assertTauriRuntime();
  return invoke<Response>(command, args);
};

export interface TranscriptStreamListeners {
  onSegment: (event: TranscriptStreamEvent) => void;
  onComplete: (summary: TranscriptStreamSummary) => void;
}

export interface LiveNudgeListeners {
  onNudge: (event: LiveNudgeEvent) => void;
}

export const getAppStatus = async (): Promise<AppStatus> => invokeNative<AppStatus>('get_app_status');

export const listAudioDevices = async (): Promise<AudioDevice[]> => invokeNative<AudioDevice[]>('list_audio_devices');

export const listMeetingHistory = async (
  searchQuery: string | null,
  limit: number,
  offset: number,
): Promise<MeetingHistoryPage> =>
  invokeNative<MeetingHistoryPage>('list_meeting_history', { searchQuery, limit, offset });

export const getMeetingHistoryDetail = async (meetingId: string): Promise<MeetingHistoryDetail> =>
  invokeNative<MeetingHistoryDetail>('get_meeting_history_detail', { meetingId });

export const listMeetingTrends = async (limit: number): Promise<MeetingTrendsResult> =>
  invokeNative<MeetingTrendsResult>('list_meeting_trends', { limit });

export const startRecording = async (meetingId: string, deviceId?: string): Promise<RecordingStarted> =>
  invokeNative<RecordingStarted>('start_recording', { meetingId, deviceId: deviceId ?? null });

export const stopRecording = async (): Promise<RecordingMetadata> => invokeNative<RecordingMetadata>('stop_recording');

export const transcribeMeeting = async (meetingId: string): Promise<TranscriptionResult> =>
  invokeNative<TranscriptionResult>('transcribe_meeting', { meetingId });

export const calculateMetrics = async (meetingId: string): Promise<MetricsCalculationResult> =>
  invokeNative<MetricsCalculationResult>('calculate_metrics', { meetingId });

export const summarizeMeeting = async (meetingId: string, model?: string): Promise<MeetingNotesResult> =>
  invokeNative<MeetingNotesResult>('summarize_meeting', { meetingId, model: model ?? null });

export const updateAudioProcessingSettings = async (
  enableSystemAudio: boolean,
  enableEchoCancellation: boolean,
): Promise<ResonanceSettings> =>
  invokeNative<ResonanceSettings>('update_audio_processing_settings', { enableSystemAudio, enableEchoCancellation });

export const updatePrivacySettings = async (
  rawAudioRetentionDays: number,
  analyzerProvider: ResonanceSettings['analyzerProvider'],
  cloudAnalysisEnabled: boolean,
): Promise<PrivacySettingsUpdateResult> =>
  invokeNative<PrivacySettingsUpdateResult>('update_privacy_settings', {
    rawAudioRetentionDays,
    analyzerProvider,
    cloudAnalysisEnabled,
  });

export const updateTranscriberSettings = async (
  transcriberBinPath: string | null,
  transcriberModelPath: string | null,
  speakerEmbeddingModelPath: string | null,
  speakerSegmentationModelPath: string | null,
): Promise<ResonanceSettings> =>
  invokeNative<ResonanceSettings>('update_transcriber_settings', {
    transcriberBinPath,
    transcriberModelPath,
    speakerEmbeddingModelPath,
    speakerSegmentationModelPath,
  });

export const sendCompletionNotification = async (title: string, body: string): Promise<void> =>
  invokeNative<void>('send_completion_notification', { title, body });

export const listenToTranscriptStream = async (listeners: TranscriptStreamListeners): Promise<UnlistenFn> => {
  assertTauriRuntime();
  const unlistenSegment = await listen<TranscriptStreamEvent>(TRANSCRIPT_SEGMENT_EVENT, (event) =>
    listeners.onSegment(event.payload),
  );
  const unlistenComplete = await listen<TranscriptStreamSummary>(TRANSCRIPT_STREAM_COMPLETE_EVENT, (event) =>
    listeners.onComplete(event.payload),
  );
  return () => {
    unlistenSegment();
    unlistenComplete();
  };
};

export const listenToLiveNudges = async (listeners: LiveNudgeListeners): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen<LiveNudgeEvent>(LIVE_NUDGE_EVENT, (event) => listeners.onNudge(event.payload));
};
