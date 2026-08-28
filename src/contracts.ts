export type MeetingId = string;
export type SegmentId = string;
export type MetricId = string;
export type ReportId = string;
export type SummaryId = string;
export type DictationSessionId = string;

export interface AppError {
  code: string;
  message: string;
  details: string | null;
}

export type ProcessingStage = 'recording' | 'transcribing' | 'metrics' | 'analyzing';

export type AnalyzerProvider = 'localOllama' | 'cloudOpenAi' | 'cloudClaude';

export type SummarizerProvider = 'lmStudio' | 'ollama' | 'custom';

export type ThemePreference = 'system' | 'light' | 'dark';

export interface ScribeSettings {
  microphoneDeviceId: string | null;
  enableSystemAudio: boolean;
  enableEchoCancellation: boolean;
  enableRealtimeNudges: boolean;
  rawAudioRetentionDays: number;
  analyzerProvider: AnalyzerProvider;
  cloudAnalysisEnabled: boolean;
  cloudVideoReviewEnabled: boolean;
  transcriberBinPath: string | null;
  transcriberModelPath: string | null;
  transcriberVocabulary: string | null;
  speakerEmbeddingModelPath: string | null;
  speakerSegmentationModelPath: string | null;
  dictationHotkey: string;
  dictationPolishEnabled: boolean;
  polishSelectionHotkey: string;
  summarizerProvider: SummarizerProvider;
  summarizerHost: string;
  summarizerPort: number;
  summarizerModel: string | null;
  themePreference: ThemePreference;
  promptOnTeamsMeeting: boolean;
}

/** Selectable appearance preferences shown in Settings. */
export const THEME_PREFERENCES = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
] as const satisfies ReadonlyArray<{ value: ThemePreference; label: string }>;

/** Provider presets: default host/port to prefill when a dev switches providers. */
export const SUMMARIZER_PROVIDERS = [
  { value: 'lmStudio', label: 'LM Studio', defaultHost: '127.0.0.1', defaultPort: 1234 },
  { value: 'ollama', label: 'Ollama', defaultHost: '127.0.0.1', defaultPort: 11434 },
  { value: 'custom', label: 'Custom (OpenAI-compatible)', defaultHost: '127.0.0.1', defaultPort: 8080 },
] as const satisfies ReadonlyArray<{
  value: SummarizerProvider;
  label: string;
  defaultHost: string;
  defaultPort: number;
}>;

/** Selectable dictation hotkeys, matching the Rust allowlist. */
export const DICTATION_HOTKEYS = [
  { value: 'cmd+shift+d', label: '⌘⇧D (double-press)' },
  { value: 'ctrl+option+d', label: '⌃⌥D (double-press)' },
  { value: 'cmd+shift+space', label: '⌘⇧Space (double-press)' },
] as const;

/**
 * Polish-selection hotkeys, matching the Rust allowlist. Only one binding is
 * supported today (not user-configurable), shown read-only in Settings.
 */
export const POLISH_SELECTION_HOTKEYS = [{ value: 'ctrl+option+p', label: '⌃⌥P (single-press)' }] as const;

export interface RetentionCleanupSummary {
  deletedAudioFileCount: number;
  removedAudioMetadataCount: number;
  skippedAudioFileCount: number;
}

export interface PrivacySettingsUpdateResult {
  settings: ScribeSettings;
  cleanup: RetentionCleanupSummary;
}

export type MeetingLifecycleState =
  | { type: 'idle' }
  | { type: 'recording'; meetingId: MeetingId; startedAtMs: number }
  | { type: 'stopping'; meetingId: MeetingId; startedAtMs: number; stoppedAtMs: number }
  | {
      type: 'transcribing';
      meetingId: MeetingId;
      startedAtMs: number;
      stoppedAtMs: number;
      audioPath: string;
    }
  | {
      type: 'failedPartial';
      meetingId: MeetingId | null;
      failedStage: ProcessingStage;
      error: AppError;
    };

export interface PipelineFailureRecord {
  meetingId: MeetingId;
  failedStage: ProcessingStage;
  errorCode: string;
  errorMessage: string;
  errorDetails: string | null;
  failedAtMs: number;
}

export interface AppStatus {
  state: string;
  detail: string;
  currentLifecycle: MeetingLifecycleState;
  defaultSettings: ScribeSettings;
}

export interface AudioDevice {
  id: string;
  name: string;
  isDefaultInput: boolean;
}

export interface RecordingStarted {
  meetingId: MeetingId;
  filePath: string;
  systemAudioFilePath: string | null;
  startedAtMs: number;
  sampleRateHz: number;
}

export interface RecordingMetadata {
  meetingId: MeetingId;
  filePath: string;
  systemAudioFilePath: string | null;
  durationMs: number;
  sampleRateHz: number;
  byteSize: number;
  systemAudioByteSize: number | null;
  startedAtMs: number;
  stoppedAtMs: number;
  droppedSampleCount: number;
  streamError: string | null;
  systemAudioStreamError: string | null;
}

export interface TranscriptSegment {
  sequenceNumber: number;
  speakerLabel: string | null;
  text: string;
  startedAtMs: number;
  endedAtMs: number;
}

export interface MetricRecord {
  id: MetricId;
  meetingId: MeetingId;
  name: string;
  value: number;
  unit: string | null;
  createdAtMs: number;
}

export interface MetricsSummary {
  fillerWordCount: number;
  fillerWordRate: number;
  hedgingPhraseCount: number;
  wordCount: number;
  durationMs: number;
  wordsPerMinute: number;
  userTalkTimeMs: number;
  longestMonologueMs: number;
}

export interface MetricsCalculationResult {
  meetingId: MeetingId;
  summary: MetricsSummary;
  metrics: MetricRecord[];
}

export interface MeetingActionItem {
  owner: string | null;
  task: string;
  due: string | null;
}

export interface SpeakingImprovement {
  category: string;
  quote: string;
  suggestion: string;
}

export interface KeyTopic {
  topic: string;
  points: string[];
}

export interface MeetingSummary {
  executiveSummary: string;
  keyTopics: KeyTopic[];
  actionItems: MeetingActionItem[];
  decisions: string[];
  openQuestions: string[];
  speakingImprovements: SpeakingImprovement[];
}

export type MeetingHistoryStatus = 'recording' | 'recorded' | 'transcribed' | 'analyzed' | 'failed_partial';

export interface MeetingHistoryItem {
  meetingId: MeetingId;
  title: string | null;
  startedAtMs: number;
  stoppedAtMs: number | null;
  updatedAtMs: number;
  durationMs: number | null;
  audioFilePath: string | null;
  status: MeetingHistoryStatus;
  transcriptSegmentCount: number;
  latestReportId: ReportId | null;
  latestReportScore: number | null;
  latestReportGeneratedAtMs: number | null;
  pipelineFailure: PipelineFailureRecord | null;
}

export interface MeetingHistoryPage {
  items: MeetingHistoryItem[];
  nextOffset: number | null;
}

export interface MeetingHistoryDetail {
  meeting: MeetingHistoryItem;
  transcriptSegments: TranscriptSegment[];
  transcriptTruncated: boolean;
  summary: MeetingSummary | null;
  summaryGeneratedAtMs: number | null;
  userNotes: string | null;
  audioFilePath: string | null;
  systemAudioFilePath: string | null;
  pipelineFailure: PipelineFailureRecord | null;
}

export interface MeetingNotesResult {
  meetingId: MeetingId;
  summary: MeetingSummary;
  generatedAtMs: number;
}

export interface MeetingTrendPoint {
  meetingId: MeetingId;
  title: string | null;
  startedAtMs: number;
  fillerWordCount: number | null;
  wordsPerMinute: number | null;
  overallScore: number | null;
}

export interface MeetingTrendsResult {
  points: MeetingTrendPoint[];
}

export interface TranscriptionResult {
  meetingId: MeetingId;
  segmentCount: number;
  segments: TranscriptSegment[];
}

export interface TranscriptStreamEvent {
  meetingId: MeetingId;
  segment: TranscriptSegment;
  isFinal: boolean;
}

export interface TranscriptStreamSummary {
  meetingId: MeetingId;
  segmentCount: number;
  droppedEventCount: number;
}

export type NudgeCategory = 'fillerWords' | 'hedging' | 'pace' | 'talkTime';
export type NudgeSeverity = 'info' | 'caution' | 'urgent';

export interface DictationSessionRecord {
  id: DictationSessionId;
  startedAtMs: number;
  endedAtMs: number;
  durationMs: number;
  wordCount: number;
  wordsPerMinute: number;
  createdAtMs: number;
  /** Null once this session has aged past the raw-audio retention window (see ScribeSettings.rawAudioRetentionDays) — the stats row survives, only the text is cleared. */
  text: string | null;
}

export interface DictationSessionPage {
  items: DictationSessionRecord[];
  nextOffset: number | null;
}

export interface DictationStatsSummary {
  totalSessions: number;
  totalWords: number;
  averageWordsPerMinute: number;
  totalDurationMs: number;
}

/**
 * The most recently dictated text, kept in memory only on the Rust side (see
 * `AppState::last_dictation`) so it can be recovered from the app if the
 * auto-paste didn't land — never written to disk, unlike the stats-only
 * `DictationSessionRecord` history.
 */
export interface LastDictationRecovery {
  text: string;
  pasted: boolean;
  atMs: number;
}

/** Why a dictation's paste didn't land. See `listenToDictationPasteFailed`. */
export type DictationPasteFailureReason =
  | 'no_target'
  | 'target_not_frontmost'
  | 'secure_input_active'
  | 'paste_did_not_land'
  | 'keystroke_failed'
  | 'accessibility_denied';

/**
 * Payload of the paste-failed event: the transcript that had nowhere to go,
 * plus why. Carried on the event rather than fetched separately so the pill's
 * recovery widget can render the text the moment the failure happens.
 */
export interface DictationPasteFailure {
  text: string;
  reason: DictationPasteFailureReason;
}

export type PermissionStatus = 'granted' | 'denied';

export interface PermissionsSnapshot {
  microphone: PermissionStatus;
  screenRecording: PermissionStatus;
  accessibility: PermissionStatus;
}

/** Permission rows for onboarding, in the order they're checked and shown. */
export const PERMISSION_ROWS = [
  {
    key: 'microphone',
    pane: 'Microphone',
    label: 'Microphone',
    description: 'Needed to record your side of a meeting.',
  },
  {
    key: 'screenRecording',
    pane: 'ScreenCapture',
    label: 'Screen Recording',
    description: "Needed to capture other participants' audio. Without it, meetings still record mic-only.",
  },
  {
    key: 'accessibility',
    pane: 'Accessibility',
    label: 'Accessibility',
    description: 'Needed for dictation to insert text into other apps.',
  },
] as const satisfies ReadonlyArray<{
  key: keyof PermissionsSnapshot;
  pane: 'Microphone' | 'ScreenCapture' | 'Accessibility';
  label: string;
  description: string;
}>;

export interface LiveNudgeEvent {
  id: string;
  meetingId: MeetingId;
  category: NudgeCategory;
  severity: NudgeSeverity;
  message: string;
  suggestion: string;
  evidence: string;
  occurredAtMs: number;
}
