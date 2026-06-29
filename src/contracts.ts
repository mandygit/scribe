export type MeetingId = string;
export type SegmentId = string;
export type MetricId = string;
export type ReportId = string;
export type SummaryId = string;
export type PracticeRecordingId = string;
export type PracticeReviewReportId = string;
export type PracticeAnnotationId = string;

export type ImportedSpeakingImprovementsSource = 'none' | 'mainSpeaker' | 'voiceMatch';

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
  deletedPracticeFileCount: number;
  removedAudioMetadataCount: number;
  skippedAudioFileCount: number;
  skippedPracticeFileCount: number;
}

export interface PrivacySettingsUpdateResult {
  settings: ResonanceSettings;
  cleanup: RetentionCleanupSummary;
}

export interface CameraDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export type PracticeRecordingSourceKind = 'camera' | 'imported';
export type PracticeReviewStatus =
  | 'recorded'
  | 'extracting'
  | 'transcribing'
  | 'reviewing'
  | 'complete'
  | 'failed_partial';
export type PracticeAnnotationSeverity = 'info' | 'caution' | 'strong';
export type PracticeAnnotationSource = 'audioLocal' | 'videoCloud' | 'videoLocal';

export interface PracticeRecording {
  id: PracticeRecordingId;
  title: string | null;
  sourceKind: PracticeRecordingSourceKind;
  videoFilePath: string;
  extractedAudioFilePath: string | null;
  durationMs: number | null;
  byteSize: number | null;
  recordedAtMs: number;
  createdAtMs: number;
  updatedAtMs: number;
  analysisStatus: PracticeReviewStatus;
  cloudVideoUsed: boolean;
  pipelineFailureCode: string | null;
  pipelineFailureMessage: string | null;
}

export interface PracticeTimelineAnnotation {
  id: PracticeAnnotationId;
  practiceRecordingId: PracticeRecordingId;
  startedAtMs: number;
  endedAtMs: number;
  category: string;
  severity: PracticeAnnotationSeverity;
  evidence: string;
  suggestion: string;
  source: PracticeAnnotationSource;
}

export interface PracticeReviewBody {
  summary: string;
  audioSummary: string;
  visualSummary: string;
  suggestions: string[];
  privacyNote: string;
}

export interface PracticeReviewReport {
  id: PracticeReviewReportId;
  practiceRecordingId: PracticeRecordingId;
  overallScore: number | null;
  audioScore: number | null;
  visualScore: number | null;
  body: PracticeReviewBody;
  generatedAtMs: number;
}

export interface PracticeReviewResult {
  recording: PracticeRecording;
  report: PracticeReviewReport;
  annotations: PracticeTimelineAnnotation[];
}

export interface PracticeRecordingsPage {
  items: PracticeRecording[];
  nextOffset: number | null;
}

export interface PracticeReviewDetail {
  recording: PracticeRecording;
  report: PracticeReviewReport | null;
  annotations: PracticeTimelineAnnotation[];
}

export interface VoiceProfileStatus {
  isEnrolled: boolean;
  enrolledAtMs: number | null;
  sampleDurationMs: number | null;
  sampleByteSize: number | null;
  matchingReady: boolean;
}

export interface VoiceMatcherStatus {
  modelConfigured: boolean;
  modelPath: string | null;
  extractorReady: boolean;
  embeddingDimension: number | null;
  message: string;
}

export interface VoiceDiarizationStatus {
  modelConfigured: boolean;
  modelPath: string | null;
  diarizationReady: boolean;
  message: string;
}

export interface DiarizedSpeakerSegment {
  startedAtMs: number;
  endedAtMs: number;
  speaker: number;
}

export interface VoiceDiarizationResult {
  speakerCount: number;
  segmentCount: number;
  segments: DiarizedSpeakerSegment[];
}

export interface DiarizedSpeakerMatch {
  speaker: number;
  isMatch: boolean;
  similarityScore: number;
  threshold: number;
}

export interface DiarizedVoiceMatchWindow {
  startedAtMs: number;
  endedAtMs: number;
  speaker: number;
  similarityScore: number;
  threshold: number;
}

export interface VoiceDiarizationMatchResult {
  speakerCount: number;
  segmentCount: number;
  matchedWindowCount: number;
  speakerMatches: DiarizedSpeakerMatch[];
  matchedWindows: DiarizedVoiceMatchWindow[];
}

export interface VoiceMatchResult {
  isMatch: boolean;
  similarityScore: number;
  threshold: number;
  message: string;
}

export type MeetingLifecycleState =
  | { type: 'idle' }
  | { type: 'recording'; meetingId: MeetingId; startedAtMs: number }
  | {
      type: 'stopping';
      meetingId: MeetingId;
      startedAtMs: number;
      stoppedAtMs: number;
    }
  | {
      type: 'transcribing';
      meetingId: MeetingId;
      startedAtMs: number;
      stoppedAtMs: number;
      audioPath: string;
    }
  | {
      type: 'analyzing';
      meetingId: MeetingId;
      startedAtMs: number;
      stoppedAtMs: number;
      audioPath: string;
      segmentCount: number;
    }
  | {
      type: 'complete';
      meetingId: MeetingId;
      startedAtMs: number;
      stoppedAtMs: number;
      audioPath: string;
      segmentCount: number;
      reportId: ReportId;
    }
  | {
      type: 'failedPartial';
      meetingId: MeetingId | null;
      failedStage: ProcessingStage;
      error: AppError;
    };

export interface MeetingReportSummary {
  reportId: ReportId;
  meetingId: MeetingId;
  overallScore: number;
  generatedAtMs: number;
}

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

export interface CoachingObservation {
  category: string;
  score: number;
  quote: string;
  speakerLabel: string | null;
  contextQuote: string | null;
  contextSpeakerLabel: string | null;
  suggestion: string;
}

export interface CoachingAnalysis {
  overallScore: number;
  observations: CoachingObservation[];
}

export interface ScoreDimension {
  score: number | null;
  unavailableReason: string | null;
}

export interface Scorecard {
  filler: ScoreDimension;
  pace: ScoreDimension;
  clarity: ScoreDimension;
  talkTime: ScoreDimension;
  analysis: ScoreDimension;
  overall: ScoreDimension;
}

export interface AnalysisResult {
  meetingId: MeetingId;
  reportId: ReportId;
  analysis: CoachingAnalysis;
  scorecard: Scorecard;
  generatedAtMs: number;
}

export interface MeetingActionItem {
  owner: string | null;
  task: string;
  due: string | null;
}

export interface MeetingSummary {
  executiveSummary: string;
  actionItems: MeetingActionItem[];
  decisions: string[];
  openQuestions: string[];
  speakingImprovements: SpeakingImprovement[];
}

export interface SpeakingImprovement {
  category: string;
  quote: string;
  suggestion: string;
}

export interface ImportedRecordingSummaryResult {
  meetingId: MeetingId;
  summaryId: SummaryId;
  sourceFilePath: string;
  extractedAudioFilePath: string;
  segmentCount: number;
  speakingImprovementsRequested: boolean;
  speakingImprovementsSource: ImportedSpeakingImprovementsSource;
  summary: MeetingSummary;
  visualReview: ImportedMeetingVisualReview | null;
  generatedAtMs: number;
}

export type ImportedMeetingVisualReviewStatus = 'notRequested' | 'audioOnly' | 'userNotVisible' | 'complete';

export interface ImportedMeetingVisualAnnotation {
  startedAtMs: number;
  endedAtMs: number;
  category: string;
  severity: PracticeAnnotationSeverity;
  evidence: string;
  suggestion: string;
  source: PracticeAnnotationSource;
}

export interface ImportedMeetingVisualReview {
  status: ImportedMeetingVisualReviewStatus;
  visualScore: number | null;
  summary: string;
  privacyNote: string;
  annotations: ImportedMeetingVisualAnnotation[];
}

export interface ImportedMeetingSummaryHistory {
  summaryId: SummaryId;
  sourceFilePath: string;
  extractedAudioFilePath: string;
  speakingImprovementsSource: ImportedSpeakingImprovementsSource;
  summary: MeetingSummary;
  generatedAtMs: number;
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
  report: AnalysisResult | null;
  importedSummary: ImportedMeetingSummaryHistory | null;
  audioFilePath: string | null;
  systemAudioFilePath: string | null;
  pipelineFailure: PipelineFailureRecord | null;
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
