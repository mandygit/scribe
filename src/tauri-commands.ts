import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AppStatus,
  AudioDevice,
  DictationSessionPage,
  DictationStatsSummary,
  LastDictationRecovery,
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
export const DICTATION_PASTE_FAILED_EVENT = 'scribe://dictation-paste-failed';
export const DICTATION_LEVEL_EVENT = 'scribe://dictation-level';
export const DICTATION_PILL_HOVER_EVENT = 'scribe://dictation-pill-hover';
export const POLISH_SELECTION_NOTICE_EVENT = 'scribe://polish-selection-notice';
export const MEETING_DETECTED_EVENT = 'scribe://meeting-detected';
export const MEETING_CALL_ENDED_EVENT = 'scribe://meeting-call-ended';
export const RECORDING_STARTED_EVENT = 'scribe://recording-started';
export const RECORDING_STOPPED_EVENT = 'scribe://recording-stopped';

export type DictationState = 'idle' | 'listening' | 'transcribing';

/**
 * Visual layouts of the dictation pill. Superset of {@link DictationState}:
 * `hover` is the idle sliver expanded under the cursor, `notice` is the
 * transient polish-selection feedback text.
 */
export type PillLayout = DictationState | 'hover' | 'notice';

export type {
  AppStatus,
  AudioDevice,
  DictationSessionPage,
  DictationSessionRecord,
  DictationStatsSummary,
  LastDictationRecovery,
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

export const setMeetingPlaceholderTitle = async (meetingId: string, title: string): Promise<void> =>
  invokeNative<void>('set_meeting_placeholder_title', { meetingId, title });

export const updateMeetingUserNotes = async (meetingId: string, content: string): Promise<void> =>
  invokeNative<void>('update_meeting_user_notes', { meetingId, content });

export const listMeetingTrends = async (limit: number): Promise<MeetingTrendsResult> =>
  invokeNative<MeetingTrendsResult>('list_meeting_trends', { limit });

export const listDictationSessions = async (limit: number, offset: number): Promise<DictationSessionPage> =>
  invokeNative<DictationSessionPage>('list_dictation_sessions', { limit, offset });

export const getDictationStatsSummary = async (): Promise<DictationStatsSummary> =>
  invokeNative<DictationStatsSummary>('get_dictation_stats_summary');

export const deleteDictationSession = async (sessionId: string): Promise<void> =>
  invokeNative<void>('delete_dictation_session', { sessionId });

export const getLastDictationRecovery = async (): Promise<LastDictationRecovery | null> =>
  invokeNative<LastDictationRecovery | null>('get_last_dictation_recovery');

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

export const updateMeetingDetectionSettings = async (promptOnTeamsMeeting: boolean): Promise<ScribeSettings> =>
  invokeNative<ScribeSettings>('update_meeting_detection_settings', { promptOnTeamsMeeting });

export const dismissMeetingPrompt = async (): Promise<void> => invokeNative<void>('dismiss_meeting_prompt');

export const updateThemePreference = async (
  themePreference: ScribeSettings['themePreference'],
): Promise<ScribeSettings> => invokeNative<ScribeSettings>('update_theme_preference', { themePreference });

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
  transcriberVocabulary: string | null,
  speakerEmbeddingModelPath: string | null,
  speakerSegmentationModelPath: string | null,
): Promise<ScribeSettings> =>
  invokeNative<ScribeSettings>('update_transcriber_settings', {
    transcriberBinPath,
    transcriberModelPath,
    transcriberVocabulary,
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

/**
 * Resizes the pill window to hug the given visual layout (see the Rust
 * `set_pill_layout` command): the window is transparent, so any area beyond
 * the painted content is an invisible click-trap over the user's screen.
 */
export const setPillLayout = async (layout: PillLayout): Promise<void> =>
  invokeNative<void>('set_pill_layout', { layout });

export const listenToDictationState = async (onState: (state: DictationState) => void): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen<{ state: DictationState }>(DICTATION_STATE_EVENT, (event) => onState(event.payload.state));
};

/**
 * Fires ~30x/s while a dictation records, with the live microphone input
 * level (RMS, 0..1) that drives the pill's waveform.
 */
export const listenToDictationLevel = async (onLevel: (level: number) => void): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen<{ level: number }>(DICTATION_LEVEL_EVENT, (event) => onLevel(event.payload.level));
};

/**
 * Fires when the cursor enters or leaves the pill window. Hover is detected
 * on the Rust side (cursor polled against the pill frame) because DOM mouse
 * tracking never fires inside the pill's non-activating, never-key panel.
 */
export const listenToDictationPillHover = async (onHover: (hovering: boolean) => void): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen<{ hovering: boolean }>(DICTATION_PILL_HOVER_EVENT, (event) => onHover(event.payload.hovering));
};

/**
 * Fires when a dictation's paste didn't land (e.g. missing Accessibility
 * permission, or the target app's focus handoff failed). The recovered text
 * itself isn't in the payload — fetch it via `getLastDictationRecovery`.
 */
export const listenToDictationPasteFailed = async (onFailed: () => void): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen(DICTATION_PASTE_FAILED_EVENT, () => onFailed());
};

export const listenToPolishSelectionNotice = async (onNotice: (message: string) => void): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen<{ message: string }>(POLISH_SELECTION_NOTICE_EVENT, (event) => onNotice(event.payload.message));
};

export const listenToMeetingDetected = async (onDetected: (meetingId: string) => void): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen<{ meetingId: string }>(MEETING_DETECTED_EVENT, (event) => onDetected(event.payload.meetingId));
};

export const listenToMeetingCallEnded = async (onEnded: () => void): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen(MEETING_CALL_ENDED_EVENT, () => onEnded());
};

/**
 * Fires whenever a recording starts, from any window (the main window's
 * Start button or the meeting-detection popup's Record button) — lets every
 * window stay in sync with actual recording state instead of only the one
 * that initiated it.
 */
export const listenToRecordingStarted = async (onStarted: (started: RecordingStarted) => void): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen<RecordingStarted>(RECORDING_STARTED_EVENT, (event) => onStarted(event.payload));
};

/** Fires whenever a recording stops, from any window. See `listenToRecordingStarted`. */
export const listenToRecordingStopped = async (
  onStopped: (metadata: RecordingMetadata) => void,
): Promise<UnlistenFn> => {
  assertTauriRuntime();
  return listen<RecordingMetadata>(RECORDING_STOPPED_EVENT, (event) => onStopped(event.payload));
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
