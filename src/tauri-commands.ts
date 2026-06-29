import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AnalysisResult,
  AppStatus,
  AudioDevice,
  CameraDevice,
  ImportedRecordingSummaryResult,
  LiveNudgeEvent,
  MeetingHistoryDetail,
  MeetingHistoryItem,
  MeetingHistoryPage,
  MeetingTrendPoint,
  MeetingTrendsResult,
  MetricsCalculationResult,
  PipelineFailureRecord,
  PracticeRecording,
  PracticeRecordingsPage,
  PracticeReviewDetail,
  PracticeReviewResult,
  PrivacySettingsUpdateResult,
  RecordingMetadata,
  RecordingStarted,
  ResonanceSettings,
  Scorecard,
  ScoreDimension,
  TranscriptionResult,
  TranscriptStreamEvent,
  TranscriptStreamSummary,
  VoiceDiarizationMatchResult,
  VoiceDiarizationResult,
  VoiceDiarizationStatus,
  VoiceMatcherStatus,
  VoiceMatchResult,
  VoiceProfileStatus,
} from './contracts';

export const TRANSCRIPT_SEGMENT_EVENT = 'resonance://transcript-segment';
export const TRANSCRIPT_STREAM_COMPLETE_EVENT = 'resonance://transcript-stream-complete';
export const LIVE_NUDGE_EVENT = 'resonance://live-nudge';

export type {
  AnalysisResult,
  AppStatus,
  AudioDevice,
  CameraDevice,
  ImportedRecordingSummaryResult,
  LiveNudgeEvent,
  MeetingHistoryDetail,
  MeetingHistoryItem,
  MeetingHistoryPage,
  MeetingTrendPoint,
  MeetingTrendsResult,
  MetricsCalculationResult,
  PipelineFailureRecord,
  PracticeRecording,
  PracticeRecordingsPage,
  PracticeReviewDetail,
  PracticeReviewResult,
  PrivacySettingsUpdateResult,
  RecordingMetadata,
  RecordingStarted,
  ResonanceSettings,
  Scorecard,
  ScoreDimension,
  TranscriptionResult,
  TranscriptStreamEvent,
  TranscriptStreamSummary,
  VoiceDiarizationMatchResult,
  VoiceDiarizationResult,
  VoiceDiarizationStatus,
  VoiceMatcherStatus,
  VoiceMatchResult,
  VoiceProfileStatus,
};

export const updateAudioProcessingSettings = async (
  enableSystemAudio: boolean,
  enableEchoCancellation: boolean,
): Promise<ResonanceSettings> => {
  return invokeNative<ResonanceSettings>('update_audio_processing_settings', {
    enableSystemAudio,
    enableEchoCancellation,
  });
};

export const updatePrivacySettings = async (
  rawAudioRetentionDays: number,
  analyzerProvider: ResonanceSettings['analyzerProvider'],
  cloudAnalysisEnabled: boolean,
): Promise<PrivacySettingsUpdateResult> => {
  return invokeNative<PrivacySettingsUpdateResult>('update_privacy_settings', {
    rawAudioRetentionDays,
    analyzerProvider,
    cloudAnalysisEnabled,
  });
};

export const updateVideoReviewSettings = async (cloudVideoReviewEnabled: boolean): Promise<ResonanceSettings> => {
  return invokeNative<ResonanceSettings>('update_video_review_settings', { cloudVideoReviewEnabled });
};

export const sendCompletionNotification = async (title: string, body: string): Promise<void> => {
  return invokeNative<void>('send_completion_notification', { title, body });
};

interface TauriInternalsWindow {
  __TAURI_INTERNALS__?: {
    invoke?: unknown;
  };
}

const assertTauriRuntime = (): void => {
  const tauriInternals = (globalThis as TauriInternalsWindow).__TAURI_INTERNALS__;

  if (typeof tauriInternals?.invoke !== 'function') {
    throw new Error(
      'Open Resonance from the native Tauri window with `bun run tauri dev`; this page cannot call microphone commands from a normal browser tab.',
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

export const getAppStatus = async (): Promise<AppStatus> => {
  return invokeNative<AppStatus>('get_app_status');
};

export const listAudioDevices = async (): Promise<AudioDevice[]> => {
  return invokeNative<AudioDevice[]>('list_audio_devices');
};

export const listCameraDevices = async (): Promise<CameraDevice[]> => {
  return invokeNative<CameraDevice[]>('list_camera_devices');
};

export const importPracticeVideo = async (sourcePath: string, title: string | null): Promise<PracticeRecording> => {
  return invokeNative<PracticeRecording>('import_practice_video', { sourcePath, title });
};

export const savePracticeCameraRecording = async (
  title: string | null,
  videoBytes: number[],
  durationMs: number,
  extension: string | null,
): Promise<PracticeRecording> => {
  return invokeNative<PracticeRecording>('save_practice_camera_recording', {
    title,
    videoBytes,
    durationMs,
    extension,
  });
};

export const analyzePracticeRecordingAudio = async (
  practiceRecordingId: string,
  ffmpegBinPath: string | null,
): Promise<PracticeReviewResult> => {
  return invokeNative<PracticeReviewResult>('analyze_practice_recording_audio', {
    practiceRecordingId,
    ffmpegBinPath,
  });
};

export const analyzePracticeRecording = async (
  practiceRecordingId: string,
  ffmpegBinPath: string | null,
  allowCloudVideoForThisReview: boolean,
): Promise<PracticeReviewResult> => {
  return invokeNative<PracticeReviewResult>('analyze_practice_recording', {
    practiceRecordingId,
    ffmpegBinPath,
    allowCloudVideoForThisReview,
  });
};

export const listPracticeRecordings = async (limit: number, offset: number): Promise<PracticeRecordingsPage> => {
  return invokeNative<PracticeRecordingsPage>('list_practice_recordings', { limit, offset });
};

export const getPracticeReviewDetail = async (practiceRecordingId: string): Promise<PracticeReviewDetail> => {
  return invokeNative<PracticeReviewDetail>('get_practice_review_detail', { practiceRecordingId });
};

export const listMeetingHistory = async (
  searchQuery: string | null,
  limit: number,
  offset: number,
): Promise<MeetingHistoryPage> => {
  return invokeNative<MeetingHistoryPage>('list_meeting_history', {
    searchQuery,
    limit,
    offset,
  });
};

export const getMeetingHistoryDetail = async (meetingId: string): Promise<MeetingHistoryDetail> => {
  return invokeNative<MeetingHistoryDetail>('get_meeting_history_detail', { meetingId });
};

export const listMeetingTrends = async (limit: number): Promise<MeetingTrendsResult> => {
  return invokeNative<MeetingTrendsResult>('list_meeting_trends', { limit });
};

export const startRecording = async (meetingId: string, deviceId?: string): Promise<RecordingStarted> => {
  return invokeNative<RecordingStarted>('start_recording', {
    meetingId,
    deviceId: deviceId ?? null,
  });
};

export const stopRecording = async (): Promise<RecordingMetadata> => {
  return invokeNative<RecordingMetadata>('stop_recording');
};

export const transcribeMeeting = async (meetingId: string): Promise<TranscriptionResult> => {
  return invokeNative<TranscriptionResult>('transcribe_meeting', { meetingId });
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

export const calculateMetrics = async (meetingId: string): Promise<MetricsCalculationResult> => {
  return invokeNative<MetricsCalculationResult>('calculate_metrics', { meetingId });
};

export const analyzeMeeting = async (meetingId: string): Promise<AnalysisResult> => {
  return invokeNative<AnalysisResult>('analyze_meeting', { meetingId });
};

export const getVoiceProfileStatus = async (): Promise<VoiceProfileStatus> => {
  return invokeNative<VoiceProfileStatus>('get_voice_profile_status');
};

export const getVoiceMatcherStatus = async (): Promise<VoiceMatcherStatus> => {
  return invokeNative<VoiceMatcherStatus>('get_voice_matcher_status');
};

export const getVoiceDiarizationStatus = async (): Promise<VoiceDiarizationStatus> => {
  return invokeNative<VoiceDiarizationStatus>('get_voice_diarization_status');
};

export const enrollVoiceProfileFromMeeting = async (meetingId: string): Promise<VoiceProfileStatus> => {
  return invokeNative<VoiceProfileStatus>('enroll_voice_profile_from_meeting', { meetingId });
};

export const prepareVoiceProfileForMatching = async (): Promise<VoiceProfileStatus> => {
  return invokeNative<VoiceProfileStatus>('prepare_voice_profile_for_matching');
};

export const matchVoiceProfileFromMeeting = async (
  meetingId: string,
  threshold: number | null = null,
): Promise<VoiceMatchResult> => {
  return invokeNative<VoiceMatchResult>('match_voice_profile_from_meeting', { meetingId, threshold });
};

export const matchImportedRecordingVoice = async (
  sourcePath: string,
  ffmpegBinPath: string | null,
  threshold: number | null = null,
): Promise<VoiceMatchResult> => {
  return invokeNative<VoiceMatchResult>('match_imported_recording_voice', { sourcePath, ffmpegBinPath, threshold });
};

export const diarizeImportedRecordingSpeakers = async (
  sourcePath: string,
  ffmpegBinPath: string | null,
): Promise<VoiceDiarizationResult> => {
  return invokeNative<VoiceDiarizationResult>('diarize_imported_recording_speakers', { sourcePath, ffmpegBinPath });
};

export const matchImportedRecordingSpeakerSegments = async (
  sourcePath: string,
  ffmpegBinPath: string | null,
  threshold: number | null = null,
): Promise<VoiceDiarizationMatchResult> => {
  return invokeNative<VoiceDiarizationMatchResult>('match_imported_recording_speaker_segments', {
    sourcePath,
    ffmpegBinPath,
    threshold,
  });
};

export const deleteVoiceProfile = async (): Promise<VoiceProfileStatus> => {
  return invokeNative<VoiceProfileStatus>('delete_voice_profile');
};

export const importRecordingSummary = async (
  sourcePath: string,
  ffmpegBinPath: string | null,
  includeSpeakingImprovements: boolean,
  useVoiceMatchedSpeakingImprovements: boolean,
  allowCloudVideoForThisReview: boolean,
): Promise<ImportedRecordingSummaryResult> => {
  return invokeNative<ImportedRecordingSummaryResult>('import_recording_summary', {
    sourcePath,
    ffmpegBinPath,
    includeSpeakingImprovements,
    useVoiceMatchedSpeakingImprovements,
    allowCloudVideoForThisReview,
  });
};

export const updateTranscriberSettings = async (
  transcriberBinPath: string | null,
  transcriberModelPath: string | null,
  speakerEmbeddingModelPath: string | null,
  speakerSegmentationModelPath: string | null,
): Promise<AppStatus['defaultSettings']> => {
  return invokeNative<AppStatus['defaultSettings']>('update_transcriber_settings', {
    transcriberBinPath,
    transcriberModelPath,
    speakerEmbeddingModelPath,
    speakerSegmentationModelPath,
  });
};
