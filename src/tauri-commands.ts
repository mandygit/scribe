import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AppStatus,
  AudioDevice,
  DictationSessionPage,
  DictationStatsSummary,
  LiveNudgeEvent,
  MeetingHistoryDetail,
  MeetingHistoryPage,
  MeetingNotesResult,
  MeetingTrendsResult,
  MetricsCalculationResult,
  PermissionsSnapshot,
  PrivacySettingsUpdateResult,
  RecordingMetadata,
  RecordingStarted,
  ScribeSettings,
  SummarizerProvider,
  TranscriptionResult,
  TranscriptStreamEvent,
  TranscriptStreamSummary,
} from './contracts';

export const TRANSCRIPT_SEGMENT_EVENT = 'scribe://transcript-segment';
export const TRANSCRIPT_STREAM_COMPLETE_EVENT = 'scribe://transcript-stream-complete';
export const LIVE_NUDGE_EVENT = 'scribe://live-nudge';
export const DICTATION_STATE_EVENT = 'scribe://dictation-state';

export type DictationState = 'idle' | 'listening' | 'transcribing';

export type {
  AppStatus,
  AudioDevice,
  DictationSessionPage,
  DictationSessionRecord,
  DictationStatsSummary,
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
  PermissionsSnapshot,
  PipelineFailureRecord,
  PrivacySettingsUpdateResult,
  RecordingMetadata,
  RecordingStarted,
  ScribeSettings,
  SummarizerProvider,
  ThemePreference,
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

export const deleteMeeting = async (meetingId: string): Promise<void> =>
  invokeNative<void>('delete_meeting', { meetingId });

export const updateMeetingTitle = async (meetingId: string, title: string): Promise<void> =>
  invokeNative<void>('update_meeting_title', { meetingId, title });

export const listMeetingTrends = async (limit: number): Promise<MeetingTrendsResult> =>
  invokeNative<MeetingTrendsResult>('list_meeting_trends', { limit });

export const listDictationSessions = async (limit: number, offset: number): Promise<DictationSessionPage> =>
  invokeNative<DictationSessionPage>('list_dictation_sessions', { limit, offset });

export const getDictationStatsSummary = async (): Promise<DictationStatsSummary> =>
  invokeNative<DictationStatsSummary>('get_dictation_stats_summary');

export const deleteDictationSession = async (sessionId: string): Promise<void> =>
  invokeNative<void>('delete_dictation_session', { sessionId });

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
): Promise<ScribeSettings> =>
  invokeNative<ScribeSettings>('update_audio_processing_settings', { enableSystemAudio, enableEchoCancellation });

export const updateThemePreference = async (
  themePreference: ScribeSettings['themePreference'],
): Promise<ScribeSettings> =>
  invokeNative<ScribeSettings>('update_theme_preference', { themePreference });

export const updatePrivacySettings = async (
  rawAudioRetentionDays: number,
  analyzerProvider: ScribeSettings['analyzerProvider'],
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
): Promise<ScribeSettings> =>
  invokeNative<ScribeSettings>('update_transcriber_settings', {
    transcriberBinPath,
    transcriberModelPath,
    speakerEmbeddingModelPath,
    speakerSegmentationModelPath,
  });

export const updateDictationSettings = async (
  dictationHotkey: string,
  dictationPolishEnabled: boolean,
): Promise<ScribeSettings> =>
  invokeNative<ScribeSettings>('update_dictation_settings', { dictationHotkey, dictationPolishEnabled });

export const updateSummarizerSettings = async (
  summarizerProvider: SummarizerProvider,
  summarizerHost: string,
  summarizerPort: number,
  summarizerModel: string | null,
): Promise<ScribeSettings> =>
  invokeNative<ScribeSettings>('update_summarizer_settings', {
    summarizerProvider,
    summarizerHost,
    summarizerPort,
    summarizerModel,
  });

export const listSummarizerModels = async (
  summarizerProvider: SummarizerProvider,
  summarizerHost: string,
  summarizerPort: number,
): Promise<string[]> =>
  invokeNative<string[]>('list_summarizer_models', { summarizerProvider, summarizerHost, summarizerPort });

export const checkPermissions = async (): Promise<PermissionsSnapshot> =>
  invokeNative<PermissionsSnapshot>('check_permissions');

export const openPermissionSettings = async (pane: 'Microphone' | 'ScreenCapture' | 'Accessibility'): Promise<void> =>
  invokeNative<void>('open_permission_settings', { pane });

export const sendCompletionNotification = async (title: string, body: string): Promise<void> =>
  invokeNative<void>('send_completion_notification', { title, body });

export const toggleDictation = async (): Promise<void> => invokeNative<void>('toggle_dictation');

export const listenToDictationState = async (onState: (state: DictationState) => void): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen<{ state: DictationState }>(DICTATION_STATE_EVENT, (event) => onState(event.payload.state));
};

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
