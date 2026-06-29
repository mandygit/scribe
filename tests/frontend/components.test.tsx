import { describe, expect, it } from 'bun:test';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import { CoachDock } from '../../src/components/CoachDock';
import { FeatureGrid } from '../../src/components/FeatureGrid';
import { ImportedRecordingPanel } from '../../src/components/ImportedRecordingPanel';
import { LiveNudgePanel } from '../../src/components/LiveNudgePanel';
import { LiveTranscriptPanel } from '../../src/components/LiveTranscriptPanel';
import { ManualVerificationPanel } from '../../src/components/ManualVerificationPanel';
import { MeetingHistoryPanel } from '../../src/components/MeetingHistoryPanel';
import { MetricsPanel } from '../../src/components/MetricsPanel';
import { PracticeReviewReport } from '../../src/components/PracticeReviewReport';
import { PrivacySettingsPanel } from '../../src/components/PrivacySettingsPanel';
import { RecordReviewPanel } from '../../src/components/RecordReviewPanel';
import { ScorecardReport } from '../../src/components/ScorecardReport';
import { SetupGuidePanel } from '../../src/components/SetupGuidePanel';
import { StatusCard } from '../../src/components/StatusCard';
import { TrendsDashboard } from '../../src/components/TrendsDashboard';
import type {
  AnalysisResult,
  AppStatus,
  ImportedRecordingSummaryResult,
  LiveNudgeEvent,
  MeetingHistoryDetail,
  MeetingHistoryItem,
  MeetingTrendPoint,
  MetricsCalculationResult,
  PracticeRecording,
  PracticeReviewResult,
  RecordingMetadata,
  ResonanceSettings,
  TranscriptionResult,
  TranscriptStreamEvent,
  TranscriptStreamSummary,
  VoiceDiarizationMatchResult,
  VoiceDiarizationResult,
  VoiceDiarizationStatus,
  VoiceMatcherStatus,
  VoiceMatchResult,
  VoiceProfileStatus,
} from '../../src/tauri-commands';

const noop = (): void => {};
const noopValue = (_value: string): void => {};
const noopBoolean = (_value: boolean): void => {};
const noopNumber = (_value: number): void => {};
const noopAnalyzerProvider = (_value: ResonanceSettings['analyzerProvider']): void => {};

const voiceProfileStatus: VoiceProfileStatus = {
  isEnrolled: true,
  enrolledAtMs: 4_000,
  sampleDurationMs: 15_000,
  sampleByteSize: 18,
  matchingReady: true,
};

const voiceMatcherStatus: VoiceMatcherStatus = {
  modelConfigured: true,
  modelPath: '/models/speaker.onnx',
  extractorReady: true,
  embeddingDimension: 256,
  message: 'Speaker embedding extractor is ready. Full voice matching will be wired in a later slice.',
};

const voiceDiarizationStatus: VoiceDiarizationStatus = {
  modelConfigured: true,
  modelPath: '/models/segmentation.onnx',
  diarizationReady: true,
  message: 'Speaker segmentation model is configured. Segment-level diarization will be wired in the next slice.',
};

const voiceMatchResult: VoiceMatchResult = {
  isMatch: true,
  similarityScore: 0.91,
  threshold: 0.75,
  message: 'Candidate audio matches the prepared local voice profile.',
};

const importedVoiceMatchResult: VoiceMatchResult = {
  isMatch: true,
  similarityScore: 0.84,
  threshold: 0.75,
  message: 'Candidate audio matches the prepared local voice profile.',
};

const voiceDiarizationResult: VoiceDiarizationResult = {
  speakerCount: 2,
  segmentCount: 3,
  segments: [
    { startedAtMs: 1_000, endedAtMs: 2_500, speaker: 0 },
    { startedAtMs: 2_700, endedAtMs: 4_000, speaker: 1 },
    { startedAtMs: 4_200, endedAtMs: 5_500, speaker: 0 },
  ],
};

const voiceDiarizationMatchResult: VoiceDiarizationMatchResult = {
  speakerCount: 2,
  segmentCount: 3,
  matchedWindowCount: 2,
  speakerMatches: [
    { speaker: 0, isMatch: true, similarityScore: 0.86, threshold: 0.75 },
    { speaker: 1, isMatch: false, similarityScore: 0.42, threshold: 0.75 },
  ],
  matchedWindows: [
    { startedAtMs: 1_000, endedAtMs: 2_500, speaker: 0, similarityScore: 0.86, threshold: 0.75 },
    { startedAtMs: 4_200, endedAtMs: 5_500, speaker: 0, similarityScore: 0.86, threshold: 0.75 },
  ],
};

const render = (element: ReactElement): string => renderToStaticMarkup(element);

const expectMarkupIncludes = (markup: string, expectedText: string): void => {
  expect(markup).toContain(expectedText);
};

const appStatus: AppStatus = {
  state: 'Ready',
  detail: 'Native bridge connected.',
  currentLifecycle: { type: 'idle' },
  defaultSettings: {
    microphoneDeviceId: null,
    enableSystemAudio: true,
    enableEchoCancellation: true,
    enableRealtimeNudges: true,
    rawAudioRetentionDays: 7,
    analyzerProvider: 'localOllama',
    cloudAnalysisEnabled: false,
    cloudVideoReviewEnabled: false,
    transcriberBinPath: '/opt/homebrew/bin/whisper-cli',
    transcriberModelPath: '/models/ggml-base.bin',
    speakerEmbeddingModelPath: '/models/speaker.onnx',
    speakerSegmentationModelPath: '/models/segmentation.onnx',
  },
};

const liveNudge: LiveNudgeEvent = {
  id: 'nudge-1',
  meetingId: 'meeting-1',
  category: 'fillerWords',
  severity: 'caution',
  message: 'Filler words are rising',
  suggestion: 'Pause before answering.',
  evidence: 'um I think we can ship',
  occurredAtMs: 1_000,
};

const metrics: MetricsCalculationResult = {
  meetingId: 'meeting-1',
  summary: {
    fillerWordCount: 2,
    fillerWordRate: 0.1,
    hedgingPhraseCount: 1,
    wordCount: 20,
    durationMs: 60_000,
    wordsPerMinute: 120,
    userTalkTimeMs: 40_000,
    longestMonologueMs: 15_000,
  },
  metrics: [
    {
      id: 'metric-1',
      meetingId: 'meeting-1',
      name: 'words_per_minute',
      value: 120,
      unit: 'wpm',
      createdAtMs: 1_000,
    },
  ],
};

const transcription: TranscriptionResult = {
  meetingId: 'meeting-1',
  segmentCount: 1,
  segments: [
    {
      sequenceNumber: 0,
      speakerLabel: 'User',
      text: 'We can ship this today.',
      startedAtMs: 0,
      endedAtMs: 1_000,
    },
  ],
};

const scorecard: AnalysisResult['scorecard'] = {
  filler: { score: 80, unavailableReason: null },
  pace: { score: 90, unavailableReason: null },
  clarity: { score: 70, unavailableReason: null },
  talkTime: { score: 85, unavailableReason: null },
  analysis: { score: 88, unavailableReason: null },
  overall: { score: 86, unavailableReason: null },
};

const analysis: AnalysisResult = {
  meetingId: 'meeting-1',
  reportId: 'report-1',
  generatedAtMs: 2_000,
  scorecard,
  analysis: {
    overallScore: 88,
    observations: [
      {
        category: 'filler',
        score: 80,
        quote: 'um I think we can ship',
        speakerLabel: 'User',
        contextQuote: 'Can we ship today?',
        contextSpeakerLabel: 'Alex',
        suggestion: 'Lead with the decision, then add context.',
      },
    ],
  },
};

const importedRecordingSummary: ImportedRecordingSummaryResult = {
  meetingId: 'imported-1',
  summaryId: 'imported-1-summary',
  sourceFilePath: '/Users/example/Downloads/team-sync.mp4',
  extractedAudioFilePath: '/Users/example/Library/Application Support/Resonance/imported-recordings/imported-1.wav',
  segmentCount: 12,
  speakingImprovementsRequested: true,
  speakingImprovementsSource: 'voiceMatch',
  visualReview: {
    status: 'userNotVisible',
    visualScore: null,
    summary: 'Audio-only review: your camera was not visible in sampled meeting frames.',
    privacyNote: 'Sampled meeting frames were sent to OpenAI after explicit consent, but the user was not visible.',
    annotations: [],
  },
  generatedAtMs: 3_000,
  summary: {
    executiveSummary: 'The team aligned on the launch checklist and remaining release blockers.',
    actionItems: [{ owner: 'Sam', task: 'Send the rollout checklist', due: 'Friday' }],
    decisions: ['Ship behind a staged rollout flag.'],
    openQuestions: ['Who owns support handoff?'],
    speakingImprovements: [
      {
        category: 'clarity',
        quote: 'um I think we can ship',
        suggestion: 'Lead with the decision before adding uncertainty.',
      },
    ],
  },
};

const recording: RecordingMetadata = {
  meetingId: 'meeting-1',
  filePath: '/tmp/meeting-1.wav',
  systemAudioFilePath: '/tmp/meeting-1.system.m4a',
  durationMs: 60_000,
  sampleRateHz: 48_000,
  byteSize: 1_024,
  systemAudioByteSize: 512,
  startedAtMs: 1_000,
  stoppedAtMs: 61_000,
  droppedSampleCount: 0,
  streamError: null,
  systemAudioStreamError: null,
};

const practiceRecording: PracticeRecording = {
  id: 'practice-1',
  title: 'Pitch rehearsal',
  sourceKind: 'imported',
  videoFilePath: '/Users/example/Library/Application Support/Resonance/practice-recordings/practice-1.mp4',
  extractedAudioFilePath:
    '/Users/example/Library/Application Support/Resonance/practice-recordings/practice-1.audio.wav',
  durationMs: 90_000,
  byteSize: 1_024,
  recordedAtMs: 1_000,
  createdAtMs: 1_000,
  updatedAtMs: 2_000,
  analysisStatus: 'complete',
  cloudVideoUsed: false,
  pipelineFailureCode: null,
  pipelineFailureMessage: null,
};

const practiceReviewResult: PracticeReviewResult = {
  recording: practiceRecording,
  report: {
    id: 'practice-1-review',
    practiceRecordingId: 'practice-1',
    overallScore: 82,
    audioScore: 82,
    visualScore: null,
    generatedAtMs: 3_000,
    body: {
      summary: 'Local audio review completed for this practice recording.',
      audioSummary: 'Pace averaged 148 WPM. Detected 2 filler words.',
      visualSummary: 'Visual review has not run.',
      suggestions: ['Use a silent pause instead of filler words.'],
      privacyNote: 'Audio was extracted and reviewed locally. No video was sent to a cloud reviewer.',
    },
  },
  annotations: [
    {
      id: 'practice-1-annotation-0',
      practiceRecordingId: 'practice-1',
      startedAtMs: 10_000,
      endedAtMs: 15_000,
      category: 'fillerWords',
      severity: 'caution',
      evidence: 'um I think this helps',
      suggestion: 'Replace fillers with a silent pause before continuing.',
      source: 'audioLocal',
    },
  ],
};

const historyItem: MeetingHistoryItem = {
  meetingId: 'meeting-1',
  title: 'Design review',
  startedAtMs: 1_000,
  stoppedAtMs: 61_000,
  updatedAtMs: 62_000,
  durationMs: 60_000,
  audioFilePath: '/tmp/meeting-1.wav',
  status: 'analyzed',
  transcriptSegmentCount: 250,
  latestReportId: 'report-1',
  latestReportScore: 86,
  latestReportGeneratedAtMs: 2_000,
  pipelineFailure: null,
};

const historyDetail: MeetingHistoryDetail = {
  meeting: historyItem,
  transcriptSegments: transcription.segments,
  transcriptTruncated: true,
  report: analysis,
  importedSummary: {
    summaryId: 'imported-1-summary',
    sourceFilePath: '/Users/example/Downloads/team-sync.mp4',
    extractedAudioFilePath: '/Users/example/Library/Application Support/Resonance/imported-recordings/imported-1.wav',
    speakingImprovementsSource: 'voiceMatch',
    summary: importedRecordingSummary.summary,
    generatedAtMs: 64_000,
  },
  audioFilePath: '/tmp/meeting-1.wav',
  systemAudioFilePath: '/tmp/meeting-1.system.m4a',
  pipelineFailure: null,
};

const transcriptEvent: TranscriptStreamEvent = {
  meetingId: 'meeting-1',
  segment: transcription.segments[0],
  isFinal: true,
};

const streamSummary: TranscriptStreamSummary = {
  meetingId: 'meeting-1',
  segmentCount: 1,
  droppedEventCount: 2,
};

const trendPoints: MeetingTrendPoint[] = [
  {
    meetingId: 'meeting-0',
    title: 'Planning sync',
    startedAtMs: 1_000,
    fillerWordCount: 5,
    wordsPerMinute: 108,
    overallScore: 72,
  },
  {
    meetingId: 'meeting-1',
    title: 'Design review',
    startedAtMs: 2_000,
    fillerWordCount: null,
    wordsPerMinute: 120,
    overallScore: 86,
  },
];

describe('extracted UI components', () => {
  it('renders coach dock recording state and latest nudge', () => {
    const markup = render(
      <CoachDock
        recordingMeetingId="meeting-1"
        isRecorderBusy={false}
        recordingIndicatorLabel="Recording"
        statusMessage="Recording in progress."
        latestVisibleNudge={liveNudge}
        onToggleRecording={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Recording');
    expectMarkupIncludes(markup, 'Filler words are rising');
    expectMarkupIncludes(markup, 'Stop session');
  });

  it('renders status and feature cards', () => {
    const statusMarkup = render(<StatusCard status={appStatus} isChecking={false} onStatusCheck={noop} />);
    const featureMarkup = render(
      <FeatureGrid features={[{ title: 'Local-first', description: 'Keeps meeting data on device.' }]} />,
    );

    expectMarkupIncludes(statusMarkup, 'Native bridge connected.');
    expectMarkupIncludes(statusMarkup, 'Check native status');
    expectMarkupIncludes(featureMarkup, 'Local-first');
  });

  it('renders the dock review state while the report is building', () => {
    const markup = render(
      <CoachDock
        recordingMeetingId={null}
        isRecorderBusy={true}
        recordingIndicatorLabel="Reviewing"
        statusMessage="Saved the session. Transcribing and replaying coaching nudges..."
        latestVisibleNudge={null}
        onToggleRecording={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Reviewing');
    expectMarkupIncludes(markup, 'Building your report');
    expectMarkupIncludes(markup, 'Transcribing and replaying coaching nudges');
  });

  it('renders metrics and streamed transcript panels', () => {
    const metricsMarkup = render(<MetricsPanel lastMetrics={metrics} lastTranscription={transcription} />);
    const transcriptMarkup = render(
      <LiveTranscriptPanel
        streamedSegments={[transcriptEvent]}
        streamSummary={streamSummary}
        maxVisibleSegments={30}
      />,
    );

    expectMarkupIncludes(metricsMarkup, '120 wpm');
    expectMarkupIncludes(metricsMarkup, 'Fillers');
    expectMarkupIncludes(transcriptMarkup, 'We can ship this today.');
    expectMarkupIncludes(transcriptMarkup, 'Dropped 2 UI event(s)');
  });

  it('renders live nudge evidence and dismiss action', () => {
    const markup = render(
      <LiveNudgePanel visibleLiveNudges={[liveNudge]} liveNudges={[liveNudge]} onDismissNudge={noopValue} />,
    );

    expectMarkupIncludes(markup, 'Fillers');
    expectMarkupIncludes(markup, 'um I think we can ship');
    expectMarkupIncludes(markup, 'Dismiss');
  });

  it('renders scorecard observations with context evidence', () => {
    const markup = render(<ScorecardReport lastAnalysis={analysis} lastMetrics={metrics} />);

    expectMarkupIncludes(markup, '86/100');
    expectMarkupIncludes(markup, 'um I think we can ship');
    expectMarkupIncludes(markup, 'Can we ship today?');
    expectMarkupIncludes(markup, 'Lead with the decision');
  });

  it('renders scorecard partial and missing signal warnings', () => {
    const partialAnalysis: AnalysisResult = {
      ...analysis,
      scorecard: {
        ...scorecard,
        clarity: { score: null, unavailableReason: 'Hedging metric unavailable.' },
        overall: { score: null, unavailableReason: 'Report needs more transcript evidence.' },
      },
    };
    const markup = render(<ScorecardReport lastAnalysis={partialAnalysis} lastMetrics={metrics} />);

    expectMarkupIncludes(markup, 'Report is partial');
    expectMarkupIncludes(markup, 'Report needs more transcript evidence.');
    expectMarkupIncludes(markup, 'Missing signal');
    expectMarkupIncludes(markup, 'Hedging metric unavailable.');
  });

  it('renders meeting history list and selected detail preview', () => {
    const markup = render(
      <MeetingHistoryPanel
        historyMessage="History loaded."
        historyItems={[historyItem]}
        historyNextOffset={10}
        selectedHistoryDetail={historyDetail}
        historySearch="design"
        isHistoryLoading={false}
        onSearchChange={noopValue}
        onLoadPage={noopBoolean}
        onSelectMeeting={noopValue}
      />,
    );

    expectMarkupIncludes(markup, 'Design review');
    expectMarkupIncludes(markup, '86/100');
    expectMarkupIncludes(markup, 'Transcript preview');
    expectMarkupIncludes(markup, 'Imported summary imported-1-summary');
    expectMarkupIncludes(markup, 'Speaking coaching source: voice match');
    expectMarkupIncludes(markup, 'Load more');
  });

  it('renders persisted pipeline failure details in meeting history', () => {
    const failedHistoryItem: MeetingHistoryItem = {
      ...historyItem,
      status: 'failed_partial',
      latestReportId: null,
      latestReportScore: null,
      latestReportGeneratedAtMs: null,
      pipelineFailure: {
        meetingId: 'meeting-1',
        failedStage: 'analyzing',
        errorCode: 'analyzer_failed',
        errorMessage: 'Local analysis failed.',
        errorDetails: null,
        failedAtMs: 64_000,
      },
    };
    const failedHistoryDetail: MeetingHistoryDetail = {
      ...historyDetail,
      meeting: failedHistoryItem,
      report: null,
      pipelineFailure: failedHistoryItem.pipelineFailure,
    };

    const markup = render(
      <MeetingHistoryPanel
        historyMessage="History loaded."
        historyItems={[failedHistoryItem]}
        historyNextOffset={null}
        selectedHistoryDetail={failedHistoryDetail}
        historySearch=""
        isHistoryLoading={false}
        onSearchChange={noopValue}
        onLoadPage={noopBoolean}
        onSelectMeeting={noopValue}
      />,
    );

    expectMarkupIncludes(markup, 'Needs retry');
    expectMarkupIncludes(markup, 'Pipeline stopped at analyzing');
    expectMarkupIncludes(markup, 'Raw audio and completed local artifacts were preserved for retry.');
  });

  it('renders trends with sparse datapoints and range controls', () => {
    const markup = render(
      <TrendsDashboard
        points={trendPoints}
        selectedLimit={10}
        availableLimits={[5, 10, 25]}
        message="Loaded 2 local meeting trend point(s)."
        isLoading={false}
        onLimitChange={noopNumber}
        onRefresh={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Recent meeting trajectory');
    expectMarkupIncludes(markup, 'Design review');
    expectMarkupIncludes(markup, '86/100');
    expectMarkupIncludes(markup, 'Missing');
    expectMarkupIncludes(markup, 'Last 25');
  });

  it('renders privacy settings with local defaults and cloud opt-in copy', () => {
    const markup = render(
      <PrivacySettingsPanel
        settings={appStatus.defaultSettings}
        retentionDays="7"
        analyzerProvider="localOllama"
        cloudAnalysisEnabled={false}
        isBusy={false}
        onRetentionDaysChange={noopValue}
        onAnalyzerProviderChange={noopAnalyzerProvider}
        onCloudAnalysisEnabledChange={noopBoolean}
        onSave={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Local data controls');
    expectMarkupIncludes(markup, 'Raw audio retention');
    expectMarkupIncludes(markup, 'Local Ollama');
    expectMarkupIncludes(markup, 'Explicitly allow cloud analysis');
    expectMarkupIncludes(markup, 'Local-first default');
  });

  it('renders first-run packaging, permission, and Ollama guidance', () => {
    const firstRunSettings: AppStatus['defaultSettings'] = {
      ...appStatus.defaultSettings,
      transcriberBinPath: null,
      transcriberModelPath: null,
    };

    const markup = render(
      <SetupGuidePanel
        settings={firstRunSettings}
        transcriberBinPath="/opt/homebrew/bin/whisper-cli"
        transcriberModelPath=""
      />,
    );

    expectMarkupIncludes(markup, 'Ready Resonance for local meetings');
    expectMarkupIncludes(markup, 'System Settings &gt; Privacy &amp; Security &gt; Microphone');
    expectMarkupIncludes(markup, 'Screen &amp; System Audio Recording permission');
    expectMarkupIncludes(markup, 'Privacy &amp; Security &gt; Camera');
    expectMarkupIncludes(markup, 'brew install ollama');
    expectMarkupIncludes(markup, 'ollama serve');
    expectMarkupIncludes(markup, 'Install whisper.cpp');
    expectMarkupIncludes(markup, 'bun run package:mac');
  });

  it('renders imported recording summary inputs and extracted outputs', () => {
    const markup = render(
      <ImportedRecordingPanel
        sourcePath="/Users/example/Downloads/team-sync.mp4"
        ffmpegBinPath="/opt/homebrew/bin/ffmpeg"
        isTranscriberModelConfigured={true}
        includeSpeakingImprovements={true}
        useMatchedVoiceCoaching={true}
        cloudVideoReviewEnabled={true}
        allowCloudVideoForThisReview={true}
        voiceProfileStatus={voiceProfileStatus}
        voiceMatcherStatus={voiceMatcherStatus}
        voiceDiarizationStatus={voiceDiarizationStatus}
        voiceDiarizationResult={voiceDiarizationResult}
        voiceDiarizationMatchResult={voiceDiarizationMatchResult}
        voiceMatchResult={voiceMatchResult}
        importedVoiceMatchResult={importedVoiceMatchResult}
        lastRecording={recording}
        result={importedRecordingSummary}
        message="Created local imported-recording summary imported-1-summary."
        isBusy={false}
        onSourcePathChange={noopValue}
        onFfmpegBinPathChange={noopValue}
        onIncludeSpeakingImprovementsChange={noopBoolean}
        onUseMatchedVoiceCoachingChange={noopBoolean}
        onCloudVideoReviewEnabledChange={noopBoolean}
        onAllowCloudVideoForThisReviewChange={noopBoolean}
        onEnrollVoiceProfile={noop}
        onPrepareVoiceProfileForMatching={noop}
        onTestVoiceProfileMatch={noop}
        onMatchImportedRecordingVoice={noop}
        onDiarizeImportedRecordingSpeakers={noop}
        onMatchImportedRecordingSpeakerSegments={noop}
        onDeleteVoiceProfile={noop}
        onImport={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Summarize a missed meeting');
    expectMarkupIncludes(markup, 'I am the main speaker/presenter');
    expectMarkupIncludes(markup, 'Use my matched voice profile for speaking coaching');
    expectMarkupIncludes(markup, 'I confirm this meeting review may send sampled frames to OpenAI');
    expectMarkupIncludes(markup, 'Recommended: coach only the speech that matches your local voice profile.');
    expectMarkupIncludes(markup, 'Fallback: only use this when you are clearly the primary speaker.');
    expectMarkupIncludes(markup, 'Voice profile prepared for matching');
    expectMarkupIncludes(markup, 'Speaker embedding extractor is ready');
    expectMarkupIncludes(markup, 'Speaker segmentation model is configured');
    expectMarkupIncludes(markup, 'Preview speaker segments');
    expectMarkupIncludes(markup, '3 diarized speaker segment(s) across 2 speaker(s).');
    expectMarkupIncludes(markup, 'Match my speaker segments');
    expectMarkupIncludes(markup, '2 likely user speaker segment(s) matched across 2 diarized speaker(s).');
    expectMarkupIncludes(markup, 'Prepare matching');
    expectMarkupIncludes(markup, 'Test match with last mic test');
    expectMarkupIncludes(markup, '91% match');
    expectMarkupIncludes(markup, 'Check voice in recording');
    expectMarkupIncludes(markup, '84% imported-recording match');
    expectMarkupIncludes(markup, 'Extract, transcribe, and summarize');
    expectMarkupIncludes(markup, 'Executive summary');
    expectMarkupIncludes(markup, 'Speaking improvements');
    expectMarkupIncludes(markup, 'Visual delivery');
    expectMarkupIncludes(markup, 'your camera was not visible');
    expectMarkupIncludes(markup, 'Lead with the decision before adding uncertainty.');
    expectMarkupIncludes(markup, 'Sam');
    expectMarkupIncludes(markup, 'Ship behind a staged rollout flag.');
    expectMarkupIncludes(markup, 'Who owns support handoff?');
  });

  it('blocks imported recording summary until a whisper model path is saved', () => {
    const markup = render(
      <ImportedRecordingPanel
        sourcePath="/Users/example/Downloads/team-sync.mp4"
        ffmpegBinPath="/opt/homebrew/bin/ffmpeg"
        isTranscriberModelConfigured={false}
        includeSpeakingImprovements={false}
        useMatchedVoiceCoaching={false}
        cloudVideoReviewEnabled={false}
        allowCloudVideoForThisReview={false}
        voiceProfileStatus={{ ...voiceProfileStatus, isEnrolled: false, matchingReady: false }}
        voiceMatcherStatus={{ ...voiceMatcherStatus, modelConfigured: false, extractorReady: false }}
        voiceDiarizationStatus={{ ...voiceDiarizationStatus, modelConfigured: false, diarizationReady: false }}
        voiceDiarizationResult={null}
        voiceDiarizationMatchResult={null}
        voiceMatchResult={null}
        importedVoiceMatchResult={null}
        lastRecording={null}
        result={null}
        message="Ready to summarize a downloaded recording locally."
        isBusy={false}
        onSourcePathChange={noopValue}
        onFfmpegBinPathChange={noopValue}
        onIncludeSpeakingImprovementsChange={noopBoolean}
        onUseMatchedVoiceCoachingChange={noopBoolean}
        onCloudVideoReviewEnabledChange={noopBoolean}
        onAllowCloudVideoForThisReviewChange={noopBoolean}
        onEnrollVoiceProfile={noop}
        onPrepareVoiceProfileForMatching={noop}
        onTestVoiceProfileMatch={noop}
        onMatchImportedRecordingVoice={noop}
        onDiarizeImportedRecordingSpeakers={noop}
        onMatchImportedRecordingSpeakerSegments={noop}
        onDeleteVoiceProfile={noop}
        onImport={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Save a whisper.cpp model path');
    expectMarkupIncludes(markup, 'Whisper transcription settings');
    expect(markup).toContain('Extract, transcribe, and summarize');
    expect(markup).toContain('disabled=""');
  });

  it('renders record and review camera import controls with cloud video warning', () => {
    const markup = render(
      <RecordReviewPanel
        title="Pitch rehearsal"
        importPath="/Users/example/Movies/pitch.mp4"
        ffmpegBinPath="/opt/homebrew/bin/ffmpeg"
        cloudVideoReviewEnabled={true}
        allowCloudVideoForThisReview={false}
        cameraDevices={[{ id: 'camera-1', name: 'FaceTime HD Camera', isDefault: true }]}
        currentRecording={practiceRecording}
        history={[practiceRecording]}
        result={practiceReviewResult}
        message="Ready to record or import a practice video."
        isBusy={false}
        isCameraRecording={false}
        onTitleChange={noopValue}
        onImportPathChange={noopValue}
        onFfmpegBinPathChange={noopValue}
        onCloudVideoReviewEnabledChange={noopBoolean}
        onAllowCloudVideoForThisReviewChange={noopBoolean}
        onLoadCameras={noop}
        onStartCameraRecording={noop}
        onStopCameraRecording={noop}
        onImportVideo={noop}
        onAnalyzeAudio={noop}
        onAnalyzeCombined={noop}
        onRefreshHistory={noop}
        videoPreviewRef={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Record and Review');
    expectMarkupIncludes(markup, 'Practice on camera');
    expectMarkupIncludes(markup, 'FaceTime HD Camera');
    expectMarkupIncludes(markup, 'Import practice video');
    expectMarkupIncludes(markup, 'Run local audio review');
    expectMarkupIncludes(markup, 'I confirm this review may send sampled practice-video frames');
    expectMarkupIncludes(markup, 'Visual review sends sampled frames');
    expectMarkupIncludes(markup, 'Practice history');
    expectMarkupIncludes(markup, 'OpenAI');
    expect(markup).toContain('class="practice-camera-preview"');
    expect(markup).toContain('autoPlay=""');
  });

  it('renders practice report scores privacy and timeline annotations', () => {
    const markup = render(<PracticeReviewReport result={practiceReviewResult} />);

    expectMarkupIncludes(markup, 'Practice review report');
    expectMarkupIncludes(markup, 'Local-only audio review');
    expectMarkupIncludes(markup, 'Overall: 82/100');
    expectMarkupIncludes(markup, 'Audio delivery: 82/100');
    expectMarkupIncludes(markup, 'Visual delivery: Not run/100');
    expectMarkupIncludes(markup, 'Timeline annotations');
    expectMarkupIncludes(markup, 'fillerWords');
    expectMarkupIncludes(markup, 'Replace fillers with a silent pause');
    expectMarkupIncludes(markup, 'No video was sent to a cloud reviewer.');
  });

  it('skips imported speaking coaching unless explicitly requested', () => {
    const markup = render(
      <ImportedRecordingPanel
        sourcePath="/Users/example/Downloads/team-sync.mp4"
        ffmpegBinPath="/opt/homebrew/bin/ffmpeg"
        isTranscriberModelConfigured={true}
        includeSpeakingImprovements={false}
        useMatchedVoiceCoaching={false}
        cloudVideoReviewEnabled={false}
        allowCloudVideoForThisReview={false}
        voiceProfileStatus={{ ...voiceProfileStatus, isEnrolled: false, matchingReady: false }}
        voiceMatcherStatus={{ ...voiceMatcherStatus, modelConfigured: false, extractorReady: false }}
        voiceDiarizationStatus={{ ...voiceDiarizationStatus, modelConfigured: false, diarizationReady: false }}
        voiceDiarizationResult={null}
        voiceDiarizationMatchResult={null}
        voiceMatchResult={null}
        importedVoiceMatchResult={null}
        lastRecording={null}
        result={{
          ...importedRecordingSummary,
          speakingImprovementsRequested: false,
          speakingImprovementsSource: 'none',
          summary: { ...importedRecordingSummary.summary, speakingImprovements: [] },
        }}
        message="Created local imported-recording summary imported-1-summary."
        isBusy={false}
        onSourcePathChange={noopValue}
        onFfmpegBinPathChange={noopValue}
        onIncludeSpeakingImprovementsChange={noopBoolean}
        onUseMatchedVoiceCoachingChange={noopBoolean}
        onCloudVideoReviewEnabledChange={noopBoolean}
        onAllowCloudVideoForThisReviewChange={noopBoolean}
        onEnrollVoiceProfile={noop}
        onPrepareVoiceProfileForMatching={noop}
        onTestVoiceProfileMatch={noop}
        onMatchImportedRecordingVoice={noop}
        onDiarizeImportedRecordingSpeakers={noop}
        onMatchImportedRecordingSpeakerSegments={noop}
        onDeleteVoiceProfile={noop}
        onImport={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Speaking coaching was skipped');
    expectMarkupIncludes(markup, 'No local voice profile yet');
    expectMarkupIncludes(markup, 'no user speech source was selected or matched');
    expect(markup).not.toContain('Lead with the decision before adding uncertainty.');
  });

  it('does not mark Whisper ready when only the model path is present', () => {
    const settingsWithOnlyModelPath: AppStatus['defaultSettings'] = {
      ...appStatus.defaultSettings,
      transcriberBinPath: null,
      transcriberModelPath: '/models/ggml-base.bin',
    };

    const markup = render(
      <SetupGuidePanel
        settings={settingsWithOnlyModelPath}
        transcriberBinPath=""
        transcriberModelPath="/models/ggml-base.bin"
      />,
    );

    expectMarkupIncludes(markup, 'Install whisper.cpp');
    expect(markup).not.toContain('Transcription paths are configured for this install.');
  });

  it('renders trend empty state', () => {
    const markup = render(
      <TrendsDashboard
        points={[]}
        selectedLimit={5}
        availableLimits={[5, 10]}
        message="No trend datapoints yet."
        isLoading={false}
        onLimitChange={noopNumber}
        onRefresh={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Run metrics and analysis on a few meetings');
  });

  it('renders the manual verification shell with nested workflow panels', () => {
    const markup = render(
      <ManualVerificationPanel
        status={appStatus}
        recorderMessage="Ready to test microphone capture."
        lastRecording={recording}
        lastTranscription={transcription}
        isRecorderBusy={false}
        recordingMeetingId={null}
        transcriberBinPath="/opt/homebrew/bin/whisper-cli"
        transcriberModelPath="/models/ggml-base.bin"
        speakerEmbeddingModelPath="/models/speaker.onnx"
        speakerSegmentationModelPath="/models/segmentation.onnx"
        devices={[{ id: 'device-1', name: 'MacBook Microphone', isDefaultInput: true }]}
        lastMetrics={metrics}
        lastAnalysis={analysis}
        streamedSegments={[transcriptEvent]}
        streamSummary={streamSummary}
        visibleLiveNudges={[liveNudge]}
        liveNudges={[liveNudge]}
        maxVisibleSegments={30}
        historyMessage="History loaded."
        historyItems={[historyItem]}
        historyNextOffset={null}
        selectedHistoryDetail={historyDetail}
        historySearch=""
        isHistoryLoading={false}
        trendPoints={trendPoints}
        trendLimit={10}
        trendLimits={[5, 10, 25]}
        trendMessage="Loaded 2 local meeting trend point(s)."
        isTrendsLoading={false}
        privacyRetentionDays="7"
        privacyAnalyzerProvider="localOllama"
        privacyCloudAnalysisEnabled={false}
        practiceTitle="Pitch rehearsal"
        practiceImportPath="/Users/example/Movies/pitch.mp4"
        practiceFfmpegPath="/opt/homebrew/bin/ffmpeg"
        practiceCloudVideoReviewEnabled={false}
        practiceAllowCloudVideoForThisReview={false}
        practiceCameraDevices={[{ id: 'camera-1', name: 'FaceTime HD Camera', isDefault: true }]}
        practiceCurrentRecording={practiceRecording}
        practiceHistory={[practiceRecording]}
        practiceResult={practiceReviewResult}
        practiceMessage="Ready to record or import a practice video."
        isPracticeCameraRecording={false}
        importedRecordingSourcePath="/Users/example/Downloads/team-sync.mp4"
        importedRecordingFfmpegPath="/opt/homebrew/bin/ffmpeg"
        importedRecordingTranscriberModelConfigured={true}
        importedRecordingIncludeSpeakingImprovements={true}
        importedRecordingUseMatchedVoiceCoaching={true}
        importedRecordingCloudVideoReviewEnabled={false}
        importedRecordingAllowCloudVideoForThisReview={false}
        voiceProfileStatus={voiceProfileStatus}
        voiceMatcherStatus={voiceMatcherStatus}
        voiceDiarizationStatus={voiceDiarizationStatus}
        voiceDiarizationResult={voiceDiarizationResult}
        voiceDiarizationMatchResult={voiceDiarizationMatchResult}
        voiceMatchResult={voiceMatchResult}
        importedVoiceMatchResult={importedVoiceMatchResult}
        importedRecordingResult={importedRecordingSummary}
        importedRecordingMessage="Created local imported-recording summary imported-1-summary."
        onListDevices={noop}
        onStartRecording={noop}
        onStopRecording={noop}
        onTranscribeRecording={noop}
        onCalculateMetrics={noop}
        onAnalyzeMeeting={noop}
        onSaveTranscriberSettings={noop}
        onTranscriberBinPathChange={noopValue}
        onTranscriberModelPathChange={noopValue}
        onSpeakerEmbeddingModelPathChange={noopValue}
        onSpeakerSegmentationModelPathChange={noopValue}
        onAudioProcessingSettingChange={noop}
        onDismissNudge={noopValue}
        onHistorySearchChange={noopValue}
        onHistoryLoadPage={noopBoolean}
        onHistorySelectMeeting={noopValue}
        onTrendLimitChange={noopNumber}
        onTrendsRefresh={noop}
        onPrivacyRetentionDaysChange={noopValue}
        onPrivacyAnalyzerProviderChange={noopAnalyzerProvider}
        onPrivacyCloudAnalysisEnabledChange={noopBoolean}
        onSavePrivacySettings={noop}
        onPracticeTitleChange={noopValue}
        onPracticeImportPathChange={noopValue}
        onPracticeFfmpegPathChange={noopValue}
        onPracticeCloudVideoReviewEnabledChange={noopBoolean}
        onPracticeAllowCloudVideoForThisReviewChange={noopBoolean}
        onPracticeLoadCameras={noop}
        onPracticeStartCameraRecording={noop}
        onPracticeStopCameraRecording={noop}
        onPracticeImportVideo={noop}
        onPracticeAnalyzeAudio={noop}
        onPracticeAnalyzeCombined={noop}
        onPracticeRefreshHistory={noop}
        practiceVideoPreviewRef={noop}
        onImportedRecordingSourcePathChange={noopValue}
        onImportedRecordingFfmpegPathChange={noopValue}
        onImportedRecordingIncludeSpeakingImprovementsChange={noopBoolean}
        onImportedRecordingUseMatchedVoiceCoachingChange={noopBoolean}
        onImportedRecordingCloudVideoReviewEnabledChange={noopBoolean}
        onImportedRecordingAllowCloudVideoForThisReviewChange={noopBoolean}
        onEnrollVoiceProfile={noop}
        onPrepareVoiceProfileForMatching={noop}
        onTestVoiceProfileMatch={noop}
        onMatchImportedRecordingVoice={noop}
        onDiarizeImportedRecordingSpeakers={noop}
        onMatchImportedRecordingSpeakerSegments={noop}
        onDeleteVoiceProfile={noop}
        onImportRecordingSummary={noop}
      />,
    );

    expectMarkupIncludes(markup, 'Meeting review workspace');
    expectMarkupIncludes(markup, 'Ready Resonance for local meetings');
    expectMarkupIncludes(markup, 'One-tap session capture lives in the dock above');
    expectMarkupIncludes(markup, 'MacBook Microphone');
    expectMarkupIncludes(markup, 'Meeting history');
    expectMarkupIncludes(markup, 'Local data controls');
    expectMarkupIncludes(markup, 'Recent meeting trajectory');
    expectMarkupIncludes(markup, 'Meeting report');
    expectMarkupIncludes(markup, 'Summarize a missed meeting');
    expectMarkupIncludes(markup, 'Record and Review');
    expectMarkupIncludes(markup, 'Practice review report');
    expectMarkupIncludes(markup, 'Speaker segmentation model path');
    expectMarkupIncludes(markup, 'Speaker segmentation model is configured');
    expectMarkupIncludes(markup, '3 diarized speaker segment(s) across 2 speaker(s).');
    expectMarkupIncludes(markup, '2 likely user speaker segment(s) matched across 2 diarized speaker(s).');
  });
});
