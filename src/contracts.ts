export type MeetingId = string;
export type SegmentId = string;
export type MetricId = string;
export type ReportId = string;
export type SummaryId = string;

export interface AppError {
  code: string;
  message: string;
  details: string | null;
}

export type ProcessingStage = 'recording' | 'transcribing' | 'metrics' | 'analyzing';

export type AnalyzerProvider = 'localOllama' | 'cloudOpenAi' | 'cloudClaude';

export interface ResonanceSettings {
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
  speakerEmbeddingModelPath: string | null;
  speakerSegmentationModelPath: string | null;
}

export interface RetentionCleanupSummary {
  deletedAudioFileCount: number;
  removedAudioMetadataCount: number;
  skippedAudioFileCount: number;
}

export interface PrivacySettingsUpdateResult {
  settings: ResonanceSettings;
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
  defaultSettings: ResonanceSettings;
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

export interface MeetingSummary {
  executiveSummary: string;
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
