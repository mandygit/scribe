import type {
  AnalysisResult,
  AppStatus,
  AudioDevice,
  CameraDevice,
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
} from '../tauri-commands';
import { ImportedRecordingPanel } from './ImportedRecordingPanel';
import { LiveNudgePanel } from './LiveNudgePanel';
import { LiveTranscriptPanel } from './LiveTranscriptPanel';
import { MeetingHistoryPanel } from './MeetingHistoryPanel';
import { MetricsPanel } from './MetricsPanel';
import { PrivacySettingsPanel } from './PrivacySettingsPanel';
import { RecordReviewPanel } from './RecordReviewPanel';
import { ScorecardReport } from './ScorecardReport';
import { SetupGuidePanel } from './SetupGuidePanel';
import { TrendsDashboard } from './TrendsDashboard';

interface ManualVerificationPanelProps {
  status: AppStatus;
  recorderMessage: string;
  lastRecording: RecordingMetadata | null;
  lastTranscription: TranscriptionResult | null;
  isRecorderBusy: boolean;
  recordingMeetingId: string | null;
  transcriberBinPath: string;
  transcriberModelPath: string;
  speakerEmbeddingModelPath: string;
  speakerSegmentationModelPath: string;
  devices: AudioDevice[];
  lastMetrics: MetricsCalculationResult | null;
  lastAnalysis: AnalysisResult | null;
  streamedSegments: TranscriptStreamEvent[];
  streamSummary: TranscriptStreamSummary | null;
  visibleLiveNudges: LiveNudgeEvent[];
  liveNudges: LiveNudgeEvent[];
  maxVisibleSegments: number;
  historyMessage: string;
  historyItems: MeetingHistoryItem[];
  historyNextOffset: number | null;
  selectedHistoryDetail: MeetingHistoryDetail | null;
  historySearch: string;
  isHistoryLoading: boolean;
  trendPoints: MeetingTrendPoint[];
  trendLimit: number;
  trendLimits: number[];
  trendMessage: string;
  isTrendsLoading: boolean;
  privacyRetentionDays: string;
  privacyAnalyzerProvider: ResonanceSettings['analyzerProvider'];
  privacyCloudAnalysisEnabled: boolean;
  practiceTitle: string;
  practiceImportPath: string;
  practiceFfmpegPath: string;
  practiceCloudVideoReviewEnabled: boolean;
  practiceAllowCloudVideoForThisReview: boolean;
  practiceCameraDevices: CameraDevice[];
  practiceCurrentRecording: PracticeRecording | null;
  practiceHistory: PracticeRecording[];
  practiceResult: PracticeReviewResult | null;
  practiceMessage: string;
  isPracticeCameraRecording: boolean;
  importedRecordingSourcePath: string;
  importedRecordingFfmpegPath: string;
  importedRecordingTranscriberModelConfigured: boolean;
  importedRecordingIncludeSpeakingImprovements: boolean;
  importedRecordingUseMatchedVoiceCoaching: boolean;
  importedRecordingCloudVideoReviewEnabled: boolean;
  importedRecordingAllowCloudVideoForThisReview: boolean;
  voiceProfileStatus: VoiceProfileStatus;
  voiceMatcherStatus: VoiceMatcherStatus;
  voiceDiarizationStatus: VoiceDiarizationStatus;
  voiceDiarizationResult: VoiceDiarizationResult | null;
  voiceDiarizationMatchResult: VoiceDiarizationMatchResult | null;
  voiceMatchResult: VoiceMatchResult | null;
  importedVoiceMatchResult: VoiceMatchResult | null;
  importedRecordingResult: ImportedRecordingSummaryResult | null;
  importedRecordingMessage: string;
  onListDevices: () => void;
  onStartRecording: () => void;
  onStopRecording: () => void;
  onTranscribeRecording: () => void;
  onCalculateMetrics: () => void;
  onAnalyzeMeeting: () => void;
  onSaveTranscriberSettings: () => void;
  onTranscriberBinPathChange: (value: string) => void;
  onTranscriberModelPathChange: (value: string) => void;
  onSpeakerEmbeddingModelPathChange: (value: string) => void;
  onSpeakerSegmentationModelPathChange: (value: string) => void;
  onAudioProcessingSettingChange: (
    nextSettings: Pick<AppStatus['defaultSettings'], 'enableSystemAudio' | 'enableEchoCancellation'>,
  ) => void;
  onDismissNudge: (nudgeId: string) => void;
  onHistorySearchChange: (value: string) => void;
  onHistoryLoadPage: (reset: boolean) => void;
  onHistorySelectMeeting: (meetingId: string) => void;
  onTrendLimitChange: (limit: number) => void;
  onTrendsRefresh: () => void;
  onPrivacyRetentionDaysChange: (value: string) => void;
  onPrivacyAnalyzerProviderChange: (value: ResonanceSettings['analyzerProvider']) => void;
  onPrivacyCloudAnalysisEnabledChange: (value: boolean) => void;
  onSavePrivacySettings: () => void;
  onPracticeTitleChange: (value: string) => void;
  onPracticeImportPathChange: (value: string) => void;
  onPracticeFfmpegPathChange: (value: string) => void;
  onPracticeCloudVideoReviewEnabledChange: (value: boolean) => void;
  onPracticeAllowCloudVideoForThisReviewChange: (value: boolean) => void;
  onPracticeLoadCameras: () => void;
  onPracticeStartCameraRecording: () => void;
  onPracticeStopCameraRecording: () => void;
  onPracticeImportVideo: () => void;
  onPracticeAnalyzeAudio: () => void;
  onPracticeAnalyzeCombined: () => void;
  onPracticeRefreshHistory: () => void;
  practiceVideoPreviewRef: (element: HTMLVideoElement | null) => void;
  onImportedRecordingSourcePathChange: (value: string) => void;
  onImportedRecordingFfmpegPathChange: (value: string) => void;
  onImportedRecordingIncludeSpeakingImprovementsChange: (value: boolean) => void;
  onImportedRecordingUseMatchedVoiceCoachingChange: (value: boolean) => void;
  onImportedRecordingCloudVideoReviewEnabledChange: (value: boolean) => void;
  onImportedRecordingAllowCloudVideoForThisReviewChange: (value: boolean) => void;
  onEnrollVoiceProfile: () => void;
  onPrepareVoiceProfileForMatching: () => void;
  onTestVoiceProfileMatch: () => void;
  onMatchImportedRecordingVoice: () => void;
  onDiarizeImportedRecordingSpeakers: () => void;
  onMatchImportedRecordingSpeakerSegments: () => void;
  onDeleteVoiceProfile: () => void;
  onImportRecordingSummary: () => void;
}

export const ManualVerificationPanel = ({
  status,
  recorderMessage,
  lastRecording,
  lastTranscription,
  isRecorderBusy,
  recordingMeetingId,
  transcriberBinPath,
  transcriberModelPath,
  speakerEmbeddingModelPath,
  speakerSegmentationModelPath,
  devices,
  lastMetrics,
  lastAnalysis,
  streamedSegments,
  streamSummary,
  visibleLiveNudges,
  liveNudges,
  maxVisibleSegments,
  historyMessage,
  historyItems,
  historyNextOffset,
  selectedHistoryDetail,
  historySearch,
  isHistoryLoading,
  trendPoints,
  trendLimit,
  trendLimits,
  trendMessage,
  isTrendsLoading,
  privacyRetentionDays,
  privacyAnalyzerProvider,
  privacyCloudAnalysisEnabled,
  practiceTitle,
  practiceImportPath,
  practiceFfmpegPath,
  practiceCloudVideoReviewEnabled,
  practiceAllowCloudVideoForThisReview,
  practiceCameraDevices,
  practiceCurrentRecording,
  practiceHistory,
  practiceResult,
  practiceMessage,
  isPracticeCameraRecording,
  importedRecordingSourcePath,
  importedRecordingFfmpegPath,
  importedRecordingTranscriberModelConfigured,
  importedRecordingIncludeSpeakingImprovements,
  importedRecordingUseMatchedVoiceCoaching,
  importedRecordingCloudVideoReviewEnabled,
  importedRecordingAllowCloudVideoForThisReview,
  voiceProfileStatus,
  voiceMatcherStatus,
  voiceDiarizationStatus,
  voiceDiarizationResult,
  voiceDiarizationMatchResult,
  voiceMatchResult,
  importedVoiceMatchResult,
  importedRecordingResult,
  importedRecordingMessage,
  onListDevices,
  onStartRecording,
  onStopRecording,
  onTranscribeRecording,
  onCalculateMetrics,
  onAnalyzeMeeting,
  onSaveTranscriberSettings,
  onTranscriberBinPathChange,
  onTranscriberModelPathChange,
  onSpeakerEmbeddingModelPathChange,
  onSpeakerSegmentationModelPathChange,
  onAudioProcessingSettingChange,
  onDismissNudge,
  onHistorySearchChange,
  onHistoryLoadPage,
  onHistorySelectMeeting,
  onTrendLimitChange,
  onTrendsRefresh,
  onPrivacyRetentionDaysChange,
  onPrivacyAnalyzerProviderChange,
  onPrivacyCloudAnalysisEnabledChange,
  onSavePrivacySettings,
  onPracticeTitleChange,
  onPracticeImportPathChange,
  onPracticeFfmpegPathChange,
  onPracticeCloudVideoReviewEnabledChange,
  onPracticeAllowCloudVideoForThisReviewChange,
  onPracticeLoadCameras,
  onPracticeStartCameraRecording,
  onPracticeStopCameraRecording,
  onPracticeImportVideo,
  onPracticeAnalyzeAudio,
  onPracticeAnalyzeCombined,
  onPracticeRefreshHistory,
  practiceVideoPreviewRef,
  onImportedRecordingSourcePathChange,
  onImportedRecordingFfmpegPathChange,
  onImportedRecordingIncludeSpeakingImprovementsChange,
  onImportedRecordingUseMatchedVoiceCoachingChange,
  onImportedRecordingCloudVideoReviewEnabledChange,
  onImportedRecordingAllowCloudVideoForThisReviewChange,
  onEnrollVoiceProfile,
  onPrepareVoiceProfileForMatching,
  onTestVoiceProfileMatch,
  onMatchImportedRecordingVoice,
  onDiarizeImportedRecordingSpeakers,
  onMatchImportedRecordingSpeakerSegments,
  onDeleteVoiceProfile,
  onImportRecordingSummary,
}: ManualVerificationPanelProps) => (
  <section className="mic-test-card" aria-labelledby="mic-test-title">
    <div>
      <span className="status-label">Advanced controls</span>
      <h2 id="mic-test-title">Meeting review workspace</h2>
      <p aria-live="polite">{recorderMessage}</p>
      <p className="panel-intro">
        One-tap session capture lives in the dock above. Use the controls below for setup checks, manual retries, and
        advanced review tools.
      </p>
      {lastRecording ? (
        <p className="recording-path">
          Playback: afplay "{lastRecording.filePath}"
          {lastRecording.systemAudioFilePath ? (
            <>
              <br />
              System audio: afplay "{lastRecording.systemAudioFilePath}"
            </>
          ) : null}
          {lastRecording.systemAudioStreamError ? (
            <>
              <br />
              System audio note: {lastRecording.systemAudioStreamError}
            </>
          ) : null}
        </p>
      ) : null}
      {lastTranscription ? (
        <p className="recording-path">
          Transcript preview: {lastTranscription.segments.map((segment) => segment.text).join(' ')}
        </p>
      ) : null}
      <section className="audio-processing-panel" aria-label="Audio processing settings">
        <label>
          <input
            type="checkbox"
            checked={status.defaultSettings.enableSystemAudio}
            disabled={isRecorderBusy || recordingMeetingId !== null}
            onChange={(event) =>
              onAudioProcessingSettingChange({
                enableSystemAudio: event.currentTarget.checked,
                enableEchoCancellation: status.defaultSettings.enableEchoCancellation,
              })
            }
          />
          Capture system audio
        </label>
        <label>
          <input
            type="checkbox"
            checked={status.defaultSettings.enableEchoCancellation}
            disabled={isRecorderBusy}
            onChange={(event) =>
              onAudioProcessingSettingChange({
                enableSystemAudio: status.defaultSettings.enableSystemAudio,
                enableEchoCancellation: event.currentTarget.checked,
              })
            }
          />
          Echo cancellation before transcription
        </label>
        <p>
          AEC writes a derived cleaned mic file when a compatible reference channel exists; raw channels stay untouched.
        </p>
      </section>
    </div>

    <SetupGuidePanel
      settings={status.defaultSettings}
      transcriberBinPath={transcriberBinPath}
      transcriberModelPath={transcriberModelPath}
    />

    <div className="mic-actions">
      <button type="button" onClick={onListDevices} disabled={isRecorderBusy}>
        Refresh microphones
      </button>
      <button type="button" onClick={onStartRecording} disabled={isRecorderBusy || recordingMeetingId !== null}>
        Start manual capture
      </button>
      <button type="button" onClick={onStopRecording} disabled={isRecorderBusy || recordingMeetingId === null}>
        Stop manual capture
      </button>
      <button type="button" onClick={onTranscribeRecording} disabled={isRecorderBusy || lastRecording === null}>
        Replay transcript
      </button>
      <button
        type="button"
        onClick={onCalculateMetrics}
        disabled={isRecorderBusy || lastTranscription === null || lastMetrics !== null}
      >
        Recalculate metrics
      </button>
      <button
        type="button"
        onClick={onAnalyzeMeeting}
        disabled={isRecorderBusy || lastMetrics === null || lastAnalysis !== null}
      >
        Regenerate report
      </button>
    </div>

    <section className="transcriber-settings" aria-label="Whisper transcription settings">
      <label>
        whisper-cli path
        <input
          type="text"
          value={transcriberBinPath}
          onChange={(event) => onTranscriberBinPathChange(event.currentTarget.value)}
          placeholder="/opt/homebrew/bin/whisper-cli"
        />
      </label>
      <label>
        Model path
        <input
          type="text"
          value={transcriberModelPath}
          onChange={(event) => onTranscriberModelPathChange(event.currentTarget.value)}
          placeholder="/absolute/path/to/ggml-base-q5_1.bin"
        />
      </label>
      <label>
        Speaker embedding model path
        <input
          type="text"
          value={speakerEmbeddingModelPath}
          onChange={(event) => onSpeakerEmbeddingModelPathChange(event.currentTarget.value)}
          placeholder="/absolute/path/to/speaker-embedding.onnx"
        />
      </label>
      <label>
        Speaker segmentation model path
        <input
          type="text"
          value={speakerSegmentationModelPath}
          onChange={(event) => onSpeakerSegmentationModelPathChange(event.currentTarget.value)}
          placeholder="/absolute/path/to/speaker-segmentation.onnx"
        />
      </label>
      <button type="button" onClick={onSaveTranscriberSettings} disabled={isRecorderBusy}>
        Save transcription and speaker paths
      </button>
    </section>

    {devices.length > 0 ? (
      <ul className="device-list" aria-label="Microphone input devices">
        {devices.map((device) => (
          <li key={device.id}>
            {device.name}
            {device.isDefaultInput ? ' · Default' : ''}
          </li>
        ))}
      </ul>
    ) : null}

    <MetricsPanel lastMetrics={lastMetrics} lastTranscription={lastTranscription} />
    <LiveTranscriptPanel
      streamedSegments={streamedSegments}
      streamSummary={streamSummary}
      maxVisibleSegments={maxVisibleSegments}
    />
    <LiveNudgePanel visibleLiveNudges={visibleLiveNudges} liveNudges={liveNudges} onDismissNudge={onDismissNudge} />
    <ScorecardReport lastAnalysis={lastAnalysis} lastMetrics={lastMetrics} />
    <ImportedRecordingPanel
      sourcePath={importedRecordingSourcePath}
      ffmpegBinPath={importedRecordingFfmpegPath}
      isTranscriberModelConfigured={importedRecordingTranscriberModelConfigured}
      includeSpeakingImprovements={importedRecordingIncludeSpeakingImprovements}
      useMatchedVoiceCoaching={importedRecordingUseMatchedVoiceCoaching}
      cloudVideoReviewEnabled={importedRecordingCloudVideoReviewEnabled}
      allowCloudVideoForThisReview={importedRecordingAllowCloudVideoForThisReview}
      voiceProfileStatus={voiceProfileStatus}
      voiceMatcherStatus={voiceMatcherStatus}
      voiceDiarizationStatus={voiceDiarizationStatus}
      voiceDiarizationResult={voiceDiarizationResult}
      voiceDiarizationMatchResult={voiceDiarizationMatchResult}
      voiceMatchResult={voiceMatchResult}
      importedVoiceMatchResult={importedVoiceMatchResult}
      lastRecording={lastRecording}
      result={importedRecordingResult}
      message={importedRecordingMessage}
      isBusy={isRecorderBusy}
      onSourcePathChange={onImportedRecordingSourcePathChange}
      onFfmpegBinPathChange={onImportedRecordingFfmpegPathChange}
      onIncludeSpeakingImprovementsChange={onImportedRecordingIncludeSpeakingImprovementsChange}
      onUseMatchedVoiceCoachingChange={onImportedRecordingUseMatchedVoiceCoachingChange}
      onCloudVideoReviewEnabledChange={onImportedRecordingCloudVideoReviewEnabledChange}
      onAllowCloudVideoForThisReviewChange={onImportedRecordingAllowCloudVideoForThisReviewChange}
      onEnrollVoiceProfile={onEnrollVoiceProfile}
      onPrepareVoiceProfileForMatching={onPrepareVoiceProfileForMatching}
      onTestVoiceProfileMatch={onTestVoiceProfileMatch}
      onMatchImportedRecordingVoice={onMatchImportedRecordingVoice}
      onDiarizeImportedRecordingSpeakers={onDiarizeImportedRecordingSpeakers}
      onMatchImportedRecordingSpeakerSegments={onMatchImportedRecordingSpeakerSegments}
      onDeleteVoiceProfile={onDeleteVoiceProfile}
      onImport={onImportRecordingSummary}
    />
    <RecordReviewPanel
      title={practiceTitle}
      importPath={practiceImportPath}
      ffmpegBinPath={practiceFfmpegPath}
      cloudVideoReviewEnabled={practiceCloudVideoReviewEnabled}
      allowCloudVideoForThisReview={practiceAllowCloudVideoForThisReview}
      cameraDevices={practiceCameraDevices}
      currentRecording={practiceCurrentRecording}
      history={practiceHistory}
      result={practiceResult}
      message={practiceMessage}
      isBusy={isRecorderBusy}
      isCameraRecording={isPracticeCameraRecording}
      onTitleChange={onPracticeTitleChange}
      onImportPathChange={onPracticeImportPathChange}
      onFfmpegBinPathChange={onPracticeFfmpegPathChange}
      onCloudVideoReviewEnabledChange={onPracticeCloudVideoReviewEnabledChange}
      onAllowCloudVideoForThisReviewChange={onPracticeAllowCloudVideoForThisReviewChange}
      onLoadCameras={onPracticeLoadCameras}
      onStartCameraRecording={onPracticeStartCameraRecording}
      onStopCameraRecording={onPracticeStopCameraRecording}
      onImportVideo={onPracticeImportVideo}
      onAnalyzeAudio={onPracticeAnalyzeAudio}
      onAnalyzeCombined={onPracticeAnalyzeCombined}
      onRefreshHistory={onPracticeRefreshHistory}
      videoPreviewRef={practiceVideoPreviewRef}
    />
    <PrivacySettingsPanel
      settings={status.defaultSettings}
      retentionDays={privacyRetentionDays}
      analyzerProvider={privacyAnalyzerProvider}
      cloudAnalysisEnabled={privacyCloudAnalysisEnabled}
      isBusy={isRecorderBusy}
      onRetentionDaysChange={onPrivacyRetentionDaysChange}
      onAnalyzerProviderChange={onPrivacyAnalyzerProviderChange}
      onCloudAnalysisEnabledChange={onPrivacyCloudAnalysisEnabledChange}
      onSave={onSavePrivacySettings}
    />
    <TrendsDashboard
      points={trendPoints}
      selectedLimit={trendLimit}
      availableLimits={trendLimits}
      message={trendMessage}
      isLoading={isTrendsLoading}
      onLimitChange={onTrendLimitChange}
      onRefresh={onTrendsRefresh}
    />
    <MeetingHistoryPanel
      historyMessage={historyMessage}
      historyItems={historyItems}
      historyNextOffset={historyNextOffset}
      selectedHistoryDetail={selectedHistoryDetail}
      historySearch={historySearch}
      isHistoryLoading={isHistoryLoading}
      onSearchChange={onHistorySearchChange}
      onLoadPage={onHistoryLoadPage}
      onSelectMeeting={onHistorySelectMeeting}
    />
  </section>
);
