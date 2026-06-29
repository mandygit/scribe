import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { CoachDock, FeatureGrid, ManualVerificationPanel, StatusCard } from './components';
import { messageFromUnknownError } from './error-utils';
import { completeMeetingSession, MeetingReviewError } from './meeting-session';
import { notificationPayloadFromAnalysis, notifyAnalysisComplete } from './notifications';
import {
  type AnalysisResult,
  type AppStatus,
  type AudioDevice,
  analyzeMeeting,
  analyzePracticeRecording,
  analyzePracticeRecordingAudio,
  type CameraDevice,
  calculateMetrics,
  deleteVoiceProfile,
  diarizeImportedRecordingSpeakers,
  enrollVoiceProfileFromMeeting,
  getAppStatus,
  getMeetingHistoryDetail,
  getPracticeReviewDetail,
  getVoiceDiarizationStatus,
  getVoiceMatcherStatus,
  getVoiceProfileStatus,
  type ImportedRecordingSummaryResult,
  importPracticeVideo,
  importRecordingSummary,
  type LiveNudgeEvent,
  listAudioDevices,
  listCameraDevices,
  listenToLiveNudges,
  listenToTranscriptStream,
  listMeetingHistory,
  listMeetingTrends,
  listPracticeRecordings,
  type MeetingHistoryDetail,
  type MeetingHistoryItem,
  type MeetingTrendPoint,
  type MetricsCalculationResult,
  matchImportedRecordingSpeakerSegments,
  matchImportedRecordingVoice,
  matchVoiceProfileFromMeeting,
  type PracticeRecording,
  type PracticeReviewResult,
  prepareVoiceProfileForMatching,
  type RecordingMetadata,
  type ResonanceSettings,
  savePracticeCameraRecording,
  startRecording,
  stopRecording,
  type TranscriptionResult,
  type TranscriptStreamEvent,
  type TranscriptStreamSummary,
  transcribeMeeting,
  updateAudioProcessingSettings,
  updatePrivacySettings,
  updateTranscriberSettings,
  updateVideoReviewSettings,
  type VoiceDiarizationMatchResult,
  type VoiceDiarizationResult,
  type VoiceDiarizationStatus,
  type VoiceMatcherStatus,
  type VoiceMatchResult,
  type VoiceProfileStatus,
} from './tauri-commands';

const FEATURES = [
  {
    title: 'One-tap capture',
    description: 'Start once, then let the session flow into transcript, metrics, and coaching without manual hops.',
  },
  {
    title: 'Transcript-backed nudges',
    description: 'Replay the strongest filler, pace, hedging, and talk-time prompts from the saved conversation.',
  },
  {
    title: 'Local-first memory',
    description: 'Keep recordings, transcripts, scores, and meeting history on device unless you explicitly opt in.',
  },
];

const MAX_STREAMED_SEGMENTS_VISIBLE = 30;
const MAX_LIVE_NUDGES_VISIBLE = 12;
const HISTORY_PAGE_LIMIT = 10;
const TREND_LIMITS = [5, 10, 25, 50];
const DEFAULT_TREND_LIMIT = 10;

const FALLBACK_STATUS: AppStatus = {
  state: 'Preview mode',
  detail: 'Open with Tauri to reach the native command bridge.',
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
    transcriberBinPath: null,
    transcriberModelPath: null,
    speakerEmbeddingModelPath: null,
    speakerSegmentationModelPath: null,
  },
};

const INITIAL_STATUS: AppStatus = {
  ...FALLBACK_STATUS,
  state: 'Ready',
  detail: 'Resonance is standing by.',
};

const INITIAL_VOICE_PROFILE_STATUS: VoiceProfileStatus = {
  isEnrolled: false,
  enrolledAtMs: null,
  sampleDurationMs: null,
  sampleByteSize: null,
  matchingReady: false,
};

const INITIAL_VOICE_MATCHER_STATUS: VoiceMatcherStatus = {
  modelConfigured: false,
  modelPath: null,
  extractorReady: false,
  embeddingDimension: null,
  message: 'Speaker matching is not configured.',
};

const INITIAL_VOICE_DIARIZATION_STATUS: VoiceDiarizationStatus = {
  modelConfigured: false,
  modelPath: null,
  diarizationReady: false,
  message: 'Speaker diarization is not configured.',
};

export const App = () => {
  const isMountedRef = useRef(true);
  const unlistenTranscriptStreamRef = useRef<(() => void) | null>(null);
  const unlistenLiveNudgesRef = useRef<(() => void) | null>(null);
  const practiceVideoElementRef = useRef<HTMLVideoElement | null>(null);
  const practiceMediaStreamRef = useRef<MediaStream | null>(null);
  const practiceMediaRecorderRef = useRef<MediaRecorder | null>(null);
  const practiceRecordedChunksRef = useRef<Blob[]>([]);
  const practiceRecordingStartedAtRef = useRef<number | null>(null);
  const [status, setStatus] = useState<AppStatus>(INITIAL_STATUS);
  const [isChecking, setIsChecking] = useState(false);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [recordingMeetingId, setRecordingMeetingId] = useState<string | null>(null);
  const [lastRecording, setLastRecording] = useState<RecordingMetadata | null>(null);
  const [lastTranscription, setLastTranscription] = useState<TranscriptionResult | null>(null);
  const [streamedSegments, setStreamedSegments] = useState<TranscriptStreamEvent[]>([]);
  const [streamSummary, setStreamSummary] = useState<TranscriptStreamSummary | null>(null);
  const [liveNudges, setLiveNudges] = useState<LiveNudgeEvent[]>([]);
  const [dismissedNudgeIds, setDismissedNudgeIds] = useState<Set<string>>(() => new Set());
  const [lastMetrics, setLastMetrics] = useState<MetricsCalculationResult | null>(null);
  const [lastAnalysis, setLastAnalysis] = useState<AnalysisResult | null>(null);
  const [recorderMessage, setRecorderMessage] = useState('Ready to test microphone capture.');
  const [transcriberBinPath, setTranscriberBinPath] = useState('/opt/homebrew/bin/whisper-cli');
  const [transcriberModelPath, setTranscriberModelPath] = useState('');
  const [speakerEmbeddingModelPath, setSpeakerEmbeddingModelPath] = useState('');
  const [speakerSegmentationModelPath, setSpeakerSegmentationModelPath] = useState('');
  const [isRecorderBusy, setIsRecorderBusy] = useState(false);
  const [historySearch, setHistorySearch] = useState('');
  const [historyItems, setHistoryItems] = useState<MeetingHistoryItem[]>([]);
  const [historyNextOffset, setHistoryNextOffset] = useState<number | null>(null);
  const [selectedHistoryDetail, setSelectedHistoryDetail] = useState<MeetingHistoryDetail | null>(null);
  const [historyMessage, setHistoryMessage] = useState('Refresh to load locally stored meetings.');
  const [isHistoryLoading, setIsHistoryLoading] = useState(false);
  const [trendPoints, setTrendPoints] = useState<MeetingTrendPoint[]>([]);
  const [trendLimit, setTrendLimit] = useState(DEFAULT_TREND_LIMIT);
  const [trendMessage, setTrendMessage] = useState('Refresh to inspect local meeting trends.');
  const [isTrendsLoading, setIsTrendsLoading] = useState(false);
  const [privacyRetentionDays, setPrivacyRetentionDays] = useState(
    String(INITIAL_STATUS.defaultSettings.rawAudioRetentionDays),
  );
  const [privacyAnalyzerProvider, setPrivacyAnalyzerProvider] = useState<ResonanceSettings['analyzerProvider']>(
    INITIAL_STATUS.defaultSettings.analyzerProvider,
  );
  const [privacyCloudAnalysisEnabled, setPrivacyCloudAnalysisEnabled] = useState(
    INITIAL_STATUS.defaultSettings.cloudAnalysisEnabled,
  );
  const [practiceTitle, setPracticeTitle] = useState('');
  const [practiceImportPath, setPracticeImportPath] = useState('');
  const [practiceFfmpegPath, setPracticeFfmpegPath] = useState('/opt/homebrew/bin/ffmpeg');
  const [practiceCloudVideoReviewEnabled, setPracticeCloudVideoReviewEnabled] = useState(
    INITIAL_STATUS.defaultSettings.cloudVideoReviewEnabled,
  );
  const [practiceAllowCloudVideoForThisReview, setPracticeAllowCloudVideoForThisReview] = useState(false);
  const [practiceCameraDevices, setPracticeCameraDevices] = useState<CameraDevice[]>([]);
  const [practiceCurrentRecording, setPracticeCurrentRecording] = useState<PracticeRecording | null>(null);
  const [practiceHistory, setPracticeHistory] = useState<PracticeRecording[]>([]);
  const [practiceResult, setPracticeResult] = useState<PracticeReviewResult | null>(null);
  const [practiceMessage, setPracticeMessage] = useState('Ready to record or import a practice video.');
  const [isPracticeCameraRecording, setIsPracticeCameraRecording] = useState(false);
  const [importedRecordingSourcePath, setImportedRecordingSourcePath] = useState('');
  const [importedRecordingFfmpegPath, setImportedRecordingFfmpegPath] = useState('/opt/homebrew/bin/ffmpeg');
  const [importedRecordingIncludeSpeakingImprovements, setImportedRecordingIncludeSpeakingImprovements] =
    useState(false);
  const [importedRecordingUseMatchedVoiceCoaching, setImportedRecordingUseMatchedVoiceCoaching] = useState(false);
  const [voiceProfileStatus, setVoiceProfileStatus] = useState<VoiceProfileStatus>(INITIAL_VOICE_PROFILE_STATUS);
  const [voiceMatcherStatus, setVoiceMatcherStatus] = useState<VoiceMatcherStatus>(INITIAL_VOICE_MATCHER_STATUS);
  const [voiceDiarizationStatus, setVoiceDiarizationStatus] = useState<VoiceDiarizationStatus>(
    INITIAL_VOICE_DIARIZATION_STATUS,
  );
  const [voiceMatchResult, setVoiceMatchResult] = useState<VoiceMatchResult | null>(null);
  const [importedVoiceMatchResult, setImportedVoiceMatchResult] = useState<VoiceMatchResult | null>(null);
  const [voiceDiarizationResult, setVoiceDiarizationResult] = useState<VoiceDiarizationResult | null>(null);
  const [voiceDiarizationMatchResult, setVoiceDiarizationMatchResult] = useState<VoiceDiarizationMatchResult | null>(
    null,
  );
  const [importedRecordingResult, setImportedRecordingResult] = useState<ImportedRecordingSummaryResult | null>(null);
  const [importedRecordingMessage, setImportedRecordingMessage] = useState(
    'Ready to summarize a downloaded recording locally.',
  );

  const cleanupLiveListeners = useCallback((): void => {
    unlistenTranscriptStreamRef.current?.();
    unlistenTranscriptStreamRef.current = null;
    unlistenLiveNudgesRef.current?.();
    unlistenLiveNudgesRef.current = null;
  }, []);

  useEffect(() => {
    return () => {
      isMountedRef.current = false;
      cleanupLiveListeners();
      practiceMediaStreamRef.current?.getTracks().forEach((track) => {
        track.stop();
      });
    };
  }, [cleanupLiveListeners]);

  useEffect(() => {
    setTranscriberBinPath(status.defaultSettings.transcriberBinPath ?? '');
    setTranscriberModelPath(status.defaultSettings.transcriberModelPath ?? '');
    setSpeakerEmbeddingModelPath(status.defaultSettings.speakerEmbeddingModelPath ?? '');
    setSpeakerSegmentationModelPath(status.defaultSettings.speakerSegmentationModelPath ?? '');
    setPrivacyRetentionDays(String(status.defaultSettings.rawAudioRetentionDays));
    setPrivacyAnalyzerProvider(status.defaultSettings.analyzerProvider);
    setPrivacyCloudAnalysisEnabled(status.defaultSettings.cloudAnalysisEnabled);
    setPracticeCloudVideoReviewEnabled(status.defaultSettings.cloudVideoReviewEnabled);
  }, [status.defaultSettings]);

  useEffect(() => {
    const matchedVoiceCoachingReady =
      voiceProfileStatus.matchingReady && voiceMatcherStatus.extractorReady && voiceDiarizationStatus.diarizationReady;
    setImportedRecordingUseMatchedVoiceCoaching(matchedVoiceCoachingReady);
  }, [voiceProfileStatus.matchingReady, voiceMatcherStatus.extractorReady, voiceDiarizationStatus.diarizationReady]);

  const loadHistoryPage = async (reset: boolean): Promise<void> => {
    const offset = reset ? 0 : historyNextOffset;
    if (offset === null) {
      return;
    }

    setIsHistoryLoading(true);
    try {
      const page = await listMeetingHistory(
        historySearch.trim() === '' ? null : historySearch.trim(),
        HISTORY_PAGE_LIMIT,
        offset,
      );
      setHistoryItems((currentItems) => (reset ? page.items : [...currentItems, ...page.items]));
      setHistoryNextOffset(page.nextOffset);
      setHistoryMessage(
        page.items.length > 0 ? 'History loaded from local storage.' : 'No meetings match this search.',
      );
      if (reset && page.items.length === 0) {
        setSelectedHistoryDetail(null);
      }
    } catch (error) {
      setHistoryMessage(error instanceof Error ? error.message : 'Could not load meeting history.');
    } finally {
      setIsHistoryLoading(false);
    }
  };

  const loadTrends = async (limit = trendLimit): Promise<void> => {
    setIsTrendsLoading(true);
    try {
      const trends = await listMeetingTrends(limit);
      setTrendPoints(trends.points);
      setTrendMessage(
        trends.points.length > 0
          ? `Loaded ${trends.points.length} local meeting trend point(s).`
          : 'No trend datapoints yet. Run metrics and analysis on recent meetings.',
      );
    } catch (error) {
      setTrendMessage(error instanceof Error ? error.message : 'Could not load meeting trends.');
    } finally {
      setIsTrendsLoading(false);
    }
  };

  const handleTrendLimitChange = (limit: number): void => {
    setTrendLimit(limit);
    void loadTrends(limit);
  };

  const handleSelectHistoryMeeting = useCallback(async (meetingId: string): Promise<void> => {
    setIsHistoryLoading(true);
    try {
      const detail = await getMeetingHistoryDetail(meetingId);
      setSelectedHistoryDetail(detail);
      setHistoryMessage(`Opened ${detail.meeting.title ?? detail.meeting.meetingId}.`);
    } catch (error) {
      setHistoryMessage(error instanceof Error ? error.message : 'Could not open meeting history detail.');
    } finally {
      setIsHistoryLoading(false);
    }
  }, []);

  const resetMeetingReviewState = useCallback((): void => {
    setLastTranscription(null);
    setStreamedSegments([]);
    setStreamSummary(null);
    setLiveNudges([]);
    setDismissedNudgeIds(new Set());
    setLastMetrics(null);
    setLastAnalysis(null);
  }, []);

  const analysisCompletionMessage = useCallback(
    async (analysis: AnalysisResult): Promise<string> => {
      try {
        const notificationStatus = await notifyAnalysisComplete(
          notificationPayloadFromAnalysis(analysis),
          handleSelectHistoryMeeting,
        );
        return notificationStatus === 'sent'
          ? `Generated score card report ${analysis.reportId}. Notification sent.`
          : notificationStatus === 'fallback-sent'
            ? `Generated score card report ${analysis.reportId}. Basic macOS notification sent.`
            : `Generated score card report ${analysis.reportId}. Notification permission is denied.`;
      } catch (notificationError) {
        const notificationMessage =
          notificationError instanceof Error ? notificationError.message : 'Native notification could not be sent.';
        return `Generated score card report ${analysis.reportId}. ${notificationMessage}`;
      }
    },
    [handleSelectHistoryMeeting],
  );

  const meetingReviewFailureMessage = useCallback((error: MeetingReviewError): string => {
    const detail = messageFromUnknownError(error.cause, 'The next review step failed.');

    if (error.phase === 'stop') {
      return detail;
    }

    if (error.phase === 'transcribing') {
      return `Saved the session, but transcription could not finish. ${detail}`;
    }

    if (error.phase === 'calculatingMetrics') {
      return `Saved the session and transcript, but metric calculation could not finish. ${detail}`;
    }

    return `Saved the session, transcript, and metrics, but report generation could not finish. ${detail}`;
  }, []);

  const handleStatusCheck = async () => {
    setIsChecking(true);

    try {
      const [nextStatus, nextVoiceProfileStatus, nextVoiceMatcherStatus, nextVoiceDiarizationStatus] =
        await Promise.all([
          getAppStatus(),
          getVoiceProfileStatus(),
          getVoiceMatcherStatus(),
          getVoiceDiarizationStatus(),
        ]);
      setStatus(nextStatus);
      setVoiceProfileStatus(nextVoiceProfileStatus);
      setVoiceMatcherStatus(nextVoiceMatcherStatus);
      setVoiceDiarizationStatus(nextVoiceDiarizationStatus);
    } catch {
      setStatus(FALLBACK_STATUS);
      setVoiceProfileStatus(INITIAL_VOICE_PROFILE_STATUS);
      setVoiceMatcherStatus(INITIAL_VOICE_MATCHER_STATUS);
      setVoiceDiarizationStatus(INITIAL_VOICE_DIARIZATION_STATUS);
    } finally {
      setIsChecking(false);
    }
  };

  const handleListDevices = async () => {
    setIsRecorderBusy(true);

    try {
      const inputDevices = await listAudioDevices();
      setDevices(inputDevices);
      setRecorderMessage(`Found ${inputDevices.length} microphone input device(s).`);
    } catch (error) {
      setRecorderMessage(messageFromUnknownError(error, 'Could not list audio devices.'));
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleStartRecording = async () => {
    const meetingId = `manual-mic-test-${Date.now()}`;
    setIsRecorderBusy(true);

    try {
      const recording = await startRecording(meetingId);
      setRecordingMeetingId(recording.meetingId);
      setLastRecording(null);
      resetMeetingReviewState();
      setRecorderMessage(`Recording ${recording.meetingId}. Speak for 5-10 seconds, then stop.`);
    } catch (error) {
      setRecorderMessage(messageFromUnknownError(error, 'Could not start recording.'));
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleStopRecording = async (autoReview = false) => {
    let savedRecording: RecordingMetadata | null = null;
    setIsRecorderBusy(true);
    cleanupLiveListeners();
    resetMeetingReviewState();

    try {
      if (!autoReview) {
        const recording = await stopRecording();
        setRecordingMeetingId(null);
        setLastRecording(recording);
        setLastTranscription(null);
        setRecorderMessage(
          recording.systemAudioFilePath
            ? `Saved ${recording.durationMs}ms mic recording and separate system audio channel.`
            : `Saved ${recording.durationMs}ms recording to ${recording.filePath}`,
        );
        return;
      }

      setRecorderMessage('Stopping the session and building your review...');
      const result = await completeMeetingSession(
        {
          stopRecording,
          listenToTranscriptStream,
          listenToLiveNudges,
          transcribeMeeting,
          calculateMetrics,
          analyzeMeeting,
        },
        {
          onRecordingSaved: (recording) => {
            savedRecording = recording;
            if (!isMountedRef.current) {
              return;
            }
            setRecordingMeetingId(null);
            setLastRecording(recording);
          },
          onPhaseChange: (phase) => {
            if (!isMountedRef.current) {
              return;
            }

            if (phase === 'transcribing') {
              setRecorderMessage('Saved the session. Transcribing and replaying coaching nudges...');
              return;
            }

            if (phase === 'calculatingMetrics') {
              setRecorderMessage('Transcript ready. Calculating speaking metrics...');
              return;
            }

            setRecorderMessage('Metrics ready. Generating the meeting report...');
          },
          onTranscriptionReady: (transcription) => {
            if (!isMountedRef.current) {
              return;
            }
            setLastTranscription(transcription);
          },
          onMetricsReady: (metrics) => {
            if (!isMountedRef.current) {
              return;
            }
            setLastMetrics(metrics);
          },
          listeners: {
            onSegment: (event) => {
              if (!isMountedRef.current) {
                return;
              }
              setStreamedSegments((currentSegments) =>
                [...currentSegments, event].slice(-MAX_STREAMED_SEGMENTS_VISIBLE),
              );
            },
            onComplete: (summary) => {
              if (!isMountedRef.current) {
                return;
              }
              setStreamSummary(summary);
            },
            onNudge: (event) => {
              if (!isMountedRef.current) {
                return;
              }
              setLiveNudges((currentNudges) => [...currentNudges, event].slice(-MAX_LIVE_NUDGES_VISIBLE));
            },
          },
        },
      );
      if (!isMountedRef.current) {
        return;
      }

      setLastAnalysis(result.analysis);
      setRecorderMessage(await analysisCompletionMessage(result.analysis));
    } catch (error) {
      if (error instanceof MeetingReviewError && error.recording !== null) {
        savedRecording = error.recording;
      }
      setRecorderMessage(
        error instanceof MeetingReviewError
          ? meetingReviewFailureMessage(error)
          : messageFromUnknownError(error, 'Could not stop recording.'),
      );
    } finally {
      void loadHistoryPage(true);
      void loadTrends();
      if (isMountedRef.current) {
        if (savedRecording !== null) {
          setRecordingMeetingId(null);
          setLastRecording(savedRecording);
        }
        setIsRecorderBusy(false);
      }
    }
  };

  const handleSaveTranscriberSettings = async () => {
    setIsRecorderBusy(true);

    try {
      const settings = await updateTranscriberSettings(
        transcriberBinPath,
        transcriberModelPath,
        speakerEmbeddingModelPath,
        speakerSegmentationModelPath,
      );
      const [nextVoiceMatcherStatus, nextVoiceDiarizationStatus] = await Promise.all([
        getVoiceMatcherStatus(),
        getVoiceDiarizationStatus(),
      ]);
      setStatus((currentStatus) => ({
        ...currentStatus,
        defaultSettings: settings,
      }));
      setVoiceMatcherStatus(nextVoiceMatcherStatus);
      setVoiceDiarizationStatus(nextVoiceDiarizationStatus);
      setRecorderMessage('Saved transcription and speaker-matching paths.');
    } catch (error) {
      setRecorderMessage(messageFromUnknownError(error, 'Could not save transcription settings.'));
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleAudioProcessingSettingChange = async (
    nextSettings: Pick<AppStatus['defaultSettings'], 'enableSystemAudio' | 'enableEchoCancellation'>,
  ) => {
    setIsRecorderBusy(true);

    try {
      const settings = await updateAudioProcessingSettings(
        nextSettings.enableSystemAudio,
        nextSettings.enableEchoCancellation,
      );
      setStatus((currentStatus) => ({
        ...currentStatus,
        defaultSettings: settings,
      }));
      setRecorderMessage('Saved audio processing settings.');
    } catch (error) {
      setRecorderMessage(messageFromUnknownError(error, 'Could not save audio processing settings.'));
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handlePrivacyCloudAnalysisEnabledChange = (enabled: boolean): void => {
    setPrivacyCloudAnalysisEnabled(enabled);
    if (!enabled) {
      setPrivacyAnalyzerProvider('localOllama');
    }
  };

  const handleSavePrivacySettings = async (): Promise<void> => {
    if (privacyRetentionDays.trim() === '') {
      setRecorderMessage('Raw audio retention must be a whole number between 0 and 365 days.');
      return;
    }

    const retentionDays = Number(privacyRetentionDays);
    if (!Number.isInteger(retentionDays) || retentionDays < 0 || retentionDays > 365) {
      setRecorderMessage('Raw audio retention must be a whole number between 0 and 365 days.');
      return;
    }

    setIsRecorderBusy(true);
    try {
      const result = await updatePrivacySettings(retentionDays, privacyAnalyzerProvider, privacyCloudAnalysisEnabled);
      setStatus((currentStatus) => ({
        ...currentStatus,
        defaultSettings: result.settings,
      }));
      setRecorderMessage(
        result.cleanup.deletedAudioFileCount > 0
          ? `Saved privacy settings and deleted ${result.cleanup.deletedAudioFileCount} expired raw audio file(s).`
          : 'Saved privacy settings. No expired raw audio files needed cleanup.',
      );
      void loadHistoryPage(true);
    } catch (error) {
      setRecorderMessage(messageFromUnknownError(error, 'Could not save privacy settings.'));
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleTranscribeRecording = async () => {
    if (!lastRecording) {
      setRecorderMessage('Record a microphone sample before transcribing.');
      return;
    }

    setIsRecorderBusy(true);
    setStreamedSegments([]);
    setStreamSummary(null);
    setLiveNudges([]);
    setDismissedNudgeIds(new Set());
    cleanupLiveListeners();
    const activeMeetingId = lastRecording.meetingId;

    try {
      unlistenTranscriptStreamRef.current = await listenToTranscriptStream({
        onSegment: (event) => {
          if (isMountedRef.current && event.meetingId === activeMeetingId) {
            setStreamedSegments((currentSegments) => [...currentSegments, event].slice(-MAX_STREAMED_SEGMENTS_VISIBLE));
          }
        },
        onComplete: (summary) => {
          if (isMountedRef.current && summary.meetingId === activeMeetingId) {
            setStreamSummary(summary);
          }
        },
      });
      unlistenLiveNudgesRef.current = await listenToLiveNudges({
        onNudge: (event) => {
          if (isMountedRef.current && event.meetingId === activeMeetingId) {
            setLiveNudges((currentNudges) => [...currentNudges, event].slice(-MAX_LIVE_NUDGES_VISIBLE));
          }
        },
      });
      const transcription = await transcribeMeeting(activeMeetingId);
      if (!isMountedRef.current) {
        return;
      }
      setLastTranscription(transcription);
      setLastMetrics(null);
      setLastAnalysis(null);
      setRecorderMessage(`Transcribed ${transcription.segmentCount} segment(s) for ${transcription.meetingId}.`);
      void loadHistoryPage(true);
    } catch (error) {
      if (isMountedRef.current) {
        setRecorderMessage(messageFromUnknownError(error, 'Could not transcribe recording.'));
      }
    } finally {
      cleanupLiveListeners();
      if (isMountedRef.current) {
        setIsRecorderBusy(false);
      }
    }
  };

  const handleCalculateMetrics = async () => {
    if (!lastTranscription) {
      setRecorderMessage('Transcribe a microphone sample before calculating metrics.');
      return;
    }

    setIsRecorderBusy(true);

    try {
      const metrics = await calculateMetrics(lastTranscription.meetingId);
      setLastMetrics(metrics);
      setLastAnalysis(null);
      setRecorderMessage(`Calculated ${metrics.metrics.length} deterministic metric(s) for ${metrics.meetingId}.`);
      void loadTrends();
    } catch (error) {
      setRecorderMessage(messageFromUnknownError(error, 'Could not calculate metrics.'));
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleAnalyzeMeeting = async () => {
    if (!lastMetrics) {
      setRecorderMessage('Calculate deterministic metrics before analyzing a meeting.');
      return;
    }

    setIsRecorderBusy(true);

    try {
      const analysis = await analyzeMeeting(lastMetrics.meetingId);
      setLastAnalysis(analysis);
      setRecorderMessage(await analysisCompletionMessage(analysis));
      void loadHistoryPage(true);
      void loadTrends();
    } catch (error) {
      setRecorderMessage(messageFromUnknownError(error, 'Could not analyze meeting.'));
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleImportRecordingSummary = async () => {
    const sourcePath = importedRecordingSourcePath.trim();
    if (!sourcePath) {
      setImportedRecordingMessage('Paste an absolute path to a downloaded recording first.');
      return;
    }
    if (status.defaultSettings.transcriberModelPath === null) {
      setImportedRecordingMessage(
        transcriberModelPath.trim() === ''
          ? 'Save a whisper.cpp model path in Whisper transcription settings before extracting and summarizing imported recordings.'
          : 'Save your current whisper.cpp model path from Whisper transcription settings before extracting and summarizing imported recordings.',
      );
      return;
    }

    setIsRecorderBusy(true);
    setImportedRecordingMessage('Extracting audio, transcribing locally, and asking Ollama for a summary...');

    try {
      const settings = await updateVideoReviewSettings(practiceCloudVideoReviewEnabled);
      setStatus((currentStatus) => ({ ...currentStatus, defaultSettings: settings }));
      const result = await importRecordingSummary(
        sourcePath,
        importedRecordingFfmpegPath.trim() || null,
        importedRecordingIncludeSpeakingImprovements,
        importedRecordingUseMatchedVoiceCoaching,
        practiceAllowCloudVideoForThisReview,
      );
      setImportedRecordingResult(result);
      setImportedVoiceMatchResult(null);
      setVoiceDiarizationResult(null);
      setVoiceDiarizationMatchResult(null);
      setImportedRecordingMessage(`Created local imported-recording summary ${result.summaryId}.`);
      void loadHistoryPage(true);
    } catch (error) {
      setImportedRecordingMessage(error instanceof Error ? error.message : 'Could not summarize imported recording.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleEnrollVoiceProfile = async () => {
    if (!lastRecording) {
      setImportedRecordingMessage('Run a short mic test before enrolling your local voice profile.');
      return;
    }

    setIsRecorderBusy(true);
    setImportedRecordingMessage('Saving local voice enrollment sample from the last mic test...');

    try {
      const nextStatus = await enrollVoiceProfileFromMeeting(lastRecording.meetingId);
      setVoiceProfileStatus(nextStatus);
      setVoiceMatchResult(null);
      setImportedVoiceMatchResult(null);
      setVoiceDiarizationResult(null);
      setVoiceDiarizationMatchResult(null);
      setImportedRecordingMessage('Local voice profile enrolled. Matching will be enabled in a later slice.');
    } catch (error) {
      setImportedRecordingMessage(error instanceof Error ? error.message : 'Could not enroll local voice profile.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleDeleteVoiceProfile = async () => {
    setIsRecorderBusy(true);
    setImportedRecordingMessage('Clearing local voice profile...');

    try {
      const nextStatus = await deleteVoiceProfile();
      setVoiceProfileStatus(nextStatus);
      setVoiceMatchResult(null);
      setImportedVoiceMatchResult(null);
      setVoiceDiarizationResult(null);
      setVoiceDiarizationMatchResult(null);
      setImportedRecordingMessage('Local voice profile cleared.');
    } catch (error) {
      setImportedRecordingMessage(error instanceof Error ? error.message : 'Could not clear local voice profile.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handlePrepareVoiceProfileForMatching = async () => {
    setIsRecorderBusy(true);
    try {
      const nextVoiceProfileStatus = await prepareVoiceProfileForMatching();
      setVoiceProfileStatus(nextVoiceProfileStatus);
      setVoiceMatchResult(null);
      setImportedVoiceMatchResult(null);
      setVoiceDiarizationResult(null);
      setVoiceDiarizationMatchResult(null);
      setImportedRecordingMessage('Prepared your local voice profile for matching.');
    } catch (error) {
      setImportedRecordingMessage(error instanceof Error ? error.message : 'Could not prepare voice matching.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleTestVoiceProfileMatch = async () => {
    if (!lastRecording) {
      setImportedRecordingMessage('Record a short mic test before testing voice matching.');
      return;
    }

    setIsRecorderBusy(true);
    try {
      const result = await matchVoiceProfileFromMeeting(lastRecording.meetingId);
      setVoiceMatchResult(result);
      setImportedRecordingMessage(result.message);
    } catch (error) {
      setImportedRecordingMessage(error instanceof Error ? error.message : 'Could not test voice matching.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleMatchImportedRecordingVoice = async () => {
    const sourcePath = importedRecordingSourcePath.trim();
    if (!sourcePath) {
      setImportedRecordingMessage('Paste an absolute path to a downloaded recording first.');
      return;
    }

    setIsRecorderBusy(true);
    setImportedRecordingMessage('Extracting imported audio and checking for your voice...');

    try {
      const result = await matchImportedRecordingVoice(sourcePath, importedRecordingFfmpegPath.trim() || null);
      setImportedVoiceMatchResult(result);
      setImportedRecordingMessage(
        `${result.message} This is a coarse whole-recording signal until diarization is wired.`,
      );
    } catch (error) {
      setImportedRecordingMessage(error instanceof Error ? error.message : 'Could not check voice in recording.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleDiarizeImportedRecordingSpeakers = async () => {
    const sourcePath = importedRecordingSourcePath.trim();
    if (!sourcePath) {
      setImportedRecordingMessage('Paste an absolute path to a downloaded recording first.');
      return;
    }

    setIsRecorderBusy(true);
    setImportedRecordingMessage('Extracting imported audio and previewing local speaker segments...');

    try {
      const result = await diarizeImportedRecordingSpeakers(sourcePath, importedRecordingFfmpegPath.trim() || null);
      setVoiceDiarizationResult(result);
      setVoiceDiarizationMatchResult(null);
      setImportedRecordingMessage(
        `Found ${result.segmentCount} diarized speaker segment(s) across ${result.speakerCount} speaker(s).`,
      );
    } catch (error) {
      setImportedRecordingMessage(error instanceof Error ? error.message : 'Could not preview speaker diarization.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handleMatchImportedRecordingSpeakerSegments = async () => {
    const sourcePath = importedRecordingSourcePath.trim();
    if (!sourcePath) {
      setImportedRecordingMessage('Paste an absolute path to a downloaded recording first.');
      return;
    }

    setIsRecorderBusy(true);
    setImportedRecordingMessage('Diarizing the recording and matching speaker turns to your local voice profile...');

    try {
      const result = await matchImportedRecordingSpeakerSegments(
        sourcePath,
        importedRecordingFfmpegPath.trim() || null,
      );
      setVoiceDiarizationMatchResult(result);
      setImportedRecordingMessage(
        `Matched ${result.matchedWindowCount} likely user speaker segment(s) across ${result.speakerMatches.length} diarized speaker(s).`,
      );
    } catch (error) {
      setImportedRecordingMessage(
        error instanceof Error ? error.message : 'Could not match diarized speakers to your voice profile.',
      );
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handlePracticeLoadCameras = async (): Promise<void> => {
    setIsRecorderBusy(true);
    try {
      const backendDevices = await listCameraDevices();
      const browserDevices =
        typeof navigator !== 'undefined' && navigator.mediaDevices?.enumerateDevices
          ? await navigator.mediaDevices.enumerateDevices()
          : [];
      const cameraDevices = browserDevices
        .filter((device) => device.kind === 'videoinput')
        .map((device, index) => ({
          id: device.deviceId || `camera-${index}`,
          name: device.label || `Camera ${index + 1}`,
          isDefault: index === 0,
        }));
      setPracticeCameraDevices(cameraDevices.length > 0 ? cameraDevices : backendDevices);
      setPracticeMessage(
        cameraDevices.length > 0 || backendDevices.length > 0
          ? 'Camera path is available. macOS may request permission when recording starts.'
          : 'No camera devices were reported by this WebView.',
      );
    } catch (error) {
      setPracticeMessage(error instanceof Error ? error.message : 'Could not check camera availability.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handlePracticeStartCameraRecording = async (): Promise<void> => {
    if (typeof navigator === 'undefined' || !navigator.mediaDevices?.getUserMedia) {
      setPracticeMessage('Camera recording requires a WebView with media device support.');
      return;
    }
    if (typeof MediaRecorder === 'undefined') {
      setPracticeMessage('This WebView does not expose MediaRecorder for camera practice yet.');
      return;
    }

    setIsRecorderBusy(true);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true, video: true });
      practiceMediaStreamRef.current = stream;
      if (practiceVideoElementRef.current) {
        practiceVideoElementRef.current.srcObject = stream;
        await practiceVideoElementRef.current.play();
      }
      practiceRecordedChunksRef.current = [];
      const recorder = new MediaRecorder(stream);
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) {
          practiceRecordedChunksRef.current.push(event.data);
        }
      };
      recorder.start();
      practiceMediaRecorderRef.current = recorder;
      practiceRecordingStartedAtRef.current = Date.now();
      setIsPracticeCameraRecording(true);
      setPracticeMessage('Camera practice recording started. Stop within 15 minutes.');
    } catch (error) {
      setPracticeMessage(error instanceof Error ? error.message : 'Could not start camera recording.');
      practiceMediaStreamRef.current?.getTracks().forEach((track) => {
        track.stop();
      });
      practiceMediaStreamRef.current = null;
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handlePracticeStopCameraRecording = async (): Promise<void> => {
    const recorder = practiceMediaRecorderRef.current;
    if (!recorder) {
      setPracticeMessage('No active camera practice recording to stop.');
      return;
    }

    setIsRecorderBusy(true);
    const stoppedAt = Date.now();
    const startedAt = practiceRecordingStartedAtRef.current ?? stoppedAt;
    const durationMs = stoppedAt - startedAt;
    try {
      const stopped = new Promise<void>((resolve) => {
        recorder.onstop = () => resolve();
      });
      recorder.stop();
      await stopped;
      practiceMediaStreamRef.current?.getTracks().forEach((track) => {
        track.stop();
      });
      practiceMediaStreamRef.current = null;
      if (practiceVideoElementRef.current) {
        practiceVideoElementRef.current.srcObject = null;
      }
      const mimeType = recorder.mimeType || 'video/webm';
      const blob = new Blob(practiceRecordedChunksRef.current, { type: mimeType });
      const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
      const extension = mimeType.includes('mp4') ? 'mp4' : 'webm';
      const recording = await savePracticeCameraRecording(practiceTitle.trim() || null, bytes, durationMs, extension);
      setPracticeCurrentRecording(recording);
      setPracticeResult(null);
      setPracticeMessage(`Saved practice recording ${recording.id}.`);
      void handlePracticeRefreshHistory();
    } catch (error) {
      setPracticeMessage(error instanceof Error ? error.message : 'Could not save camera practice recording.');
    } finally {
      practiceMediaRecorderRef.current = null;
      practiceRecordedChunksRef.current = [];
      practiceRecordingStartedAtRef.current = null;
      setIsPracticeCameraRecording(false);
      setIsRecorderBusy(false);
    }
  };

  const handlePracticeImportVideo = async (): Promise<void> => {
    const sourcePath = practiceImportPath.trim();
    if (!sourcePath) {
      setPracticeMessage('Paste an absolute path to a practice video first.');
      return;
    }
    setIsRecorderBusy(true);
    try {
      const recording = await importPracticeVideo(sourcePath, practiceTitle.trim() || null);
      setPracticeCurrentRecording(recording);
      setPracticeResult(null);
      setPracticeMessage(`Imported practice video ${recording.id}.`);
      void handlePracticeRefreshHistory();
    } catch (error) {
      setPracticeMessage(error instanceof Error ? error.message : 'Could not import practice video.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handlePracticeAnalyzeAudio = async (): Promise<void> => {
    if (!practiceCurrentRecording) {
      setPracticeMessage('Import or record a practice video before analysis.');
      return;
    }
    setIsRecorderBusy(true);
    try {
      const result = await analyzePracticeRecordingAudio(
        practiceCurrentRecording.id,
        practiceFfmpegPath.trim() || null,
      );
      setPracticeCurrentRecording(result.recording);
      setPracticeResult(result);
      setPracticeMessage(`Generated practice audio review ${result.report.id}.`);
      void handlePracticeRefreshHistory();
    } catch (error) {
      setPracticeMessage(error instanceof Error ? error.message : 'Could not analyze practice audio.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handlePracticeAnalyzeCombined = async (): Promise<void> => {
    if (!practiceCurrentRecording) {
      setPracticeMessage('Import or record a practice video before analysis.');
      return;
    }
    setIsRecorderBusy(true);
    try {
      const settings = await updateVideoReviewSettings(practiceCloudVideoReviewEnabled);
      setStatus((currentStatus) => ({ ...currentStatus, defaultSettings: settings }));
      const result = await analyzePracticeRecording(
        practiceCurrentRecording.id,
        practiceFfmpegPath.trim() || null,
        practiceAllowCloudVideoForThisReview,
      );
      setPracticeCurrentRecording(result.recording);
      setPracticeResult(result);
      setPracticeMessage(`Generated practice review ${result.report.id}.`);
      void handlePracticeRefreshHistory();
    } catch (error) {
      setPracticeMessage(error instanceof Error ? error.message : 'Could not run combined practice review.');
    } finally {
      setIsRecorderBusy(false);
    }
  };

  const handlePracticeRefreshHistory = async (): Promise<void> => {
    try {
      const page = await listPracticeRecordings(10, 0);
      setPracticeHistory(page.items);
      if (practiceCurrentRecording) {
        const detail = await getPracticeReviewDetail(practiceCurrentRecording.id);
        if (detail.report) {
          setPracticeResult({
            recording: detail.recording,
            report: detail.report,
            annotations: detail.annotations,
          });
        }
      }
    } catch (error) {
      setPracticeMessage(error instanceof Error ? error.message : 'Could not refresh practice history.');
    }
  };

  const handlePracticeCloudVideoReviewEnabledChange = (enabled: boolean): void => {
    setPracticeCloudVideoReviewEnabled(enabled);
    if (!enabled) {
      setPracticeAllowCloudVideoForThisReview(false);
    }
  };

  const setPracticeVideoPreviewElement = useCallback((element: HTMLVideoElement | null): void => {
    practiceVideoElementRef.current = element;
    if (element && practiceMediaStreamRef.current) {
      element.srcObject = practiceMediaStreamRef.current;
    }
  }, []);

  const handleToggleRecording = async () => {
    if (recordingMeetingId) {
      await handleStopRecording(true);
      return;
    }

    await handleStartRecording();
  };

  const handleDismissNudge = (nudgeId: string): void => {
    setDismissedNudgeIds((currentIds) => new Set(currentIds).add(nudgeId));
  };

  const visibleLiveNudges = useMemo(
    () => liveNudges.filter((nudge) => !dismissedNudgeIds.has(nudge.id)),
    [dismissedNudgeIds, liveNudges],
  );
  const latestVisibleNudge = visibleLiveNudges.at(-1) ?? null;
  const recordingIndicatorLabel = recordingMeetingId ? 'Recording' : isRecorderBusy ? 'Reviewing' : 'Ready';

  return (
    <main className="app-shell">
      <section className="hero-card" aria-labelledby="resonance-title">
        <div className="eyebrow">macOS meeting coach</div>
        <header className="hero-header">
          <div>
            <h1 id="resonance-title">Resonance</h1>
            <p>Capture what happened. Improve what you said.</p>
          </div>
          <div className="orb" aria-hidden="true" />
        </header>

        <CoachDock
          recordingMeetingId={recordingMeetingId}
          isRecorderBusy={isRecorderBusy}
          recordingIndicatorLabel={recordingIndicatorLabel}
          statusMessage={recorderMessage}
          latestVisibleNudge={latestVisibleNudge}
          onToggleRecording={() => void handleToggleRecording()}
        />

        <StatusCard status={status} isChecking={isChecking} onStatusCheck={() => void handleStatusCheck()} />

        <FeatureGrid features={FEATURES} />

        <ManualVerificationPanel
          status={status}
          recorderMessage={recorderMessage}
          lastRecording={lastRecording}
          lastTranscription={lastTranscription}
          isRecorderBusy={isRecorderBusy}
          recordingMeetingId={recordingMeetingId}
          transcriberBinPath={transcriberBinPath}
          transcriberModelPath={transcriberModelPath}
          speakerEmbeddingModelPath={speakerEmbeddingModelPath}
          speakerSegmentationModelPath={speakerSegmentationModelPath}
          devices={devices}
          lastMetrics={lastMetrics}
          lastAnalysis={lastAnalysis}
          streamedSegments={streamedSegments}
          streamSummary={streamSummary}
          visibleLiveNudges={visibleLiveNudges}
          liveNudges={liveNudges}
          maxVisibleSegments={MAX_STREAMED_SEGMENTS_VISIBLE}
          historyMessage={historyMessage}
          historyItems={historyItems}
          historyNextOffset={historyNextOffset}
          selectedHistoryDetail={selectedHistoryDetail}
          historySearch={historySearch}
          isHistoryLoading={isHistoryLoading}
          trendPoints={trendPoints}
          trendLimit={trendLimit}
          trendLimits={TREND_LIMITS}
          trendMessage={trendMessage}
          isTrendsLoading={isTrendsLoading}
          privacyRetentionDays={privacyRetentionDays}
          privacyAnalyzerProvider={privacyAnalyzerProvider}
          privacyCloudAnalysisEnabled={privacyCloudAnalysisEnabled}
          practiceTitle={practiceTitle}
          practiceImportPath={practiceImportPath}
          practiceFfmpegPath={practiceFfmpegPath}
          practiceCloudVideoReviewEnabled={practiceCloudVideoReviewEnabled}
          practiceAllowCloudVideoForThisReview={practiceAllowCloudVideoForThisReview}
          practiceCameraDevices={practiceCameraDevices}
          practiceCurrentRecording={practiceCurrentRecording}
          practiceHistory={practiceHistory}
          practiceResult={practiceResult}
          practiceMessage={practiceMessage}
          isPracticeCameraRecording={isPracticeCameraRecording}
          importedRecordingSourcePath={importedRecordingSourcePath}
          importedRecordingFfmpegPath={importedRecordingFfmpegPath}
          importedRecordingTranscriberModelConfigured={status.defaultSettings.transcriberModelPath !== null}
          importedRecordingIncludeSpeakingImprovements={importedRecordingIncludeSpeakingImprovements}
          importedRecordingUseMatchedVoiceCoaching={importedRecordingUseMatchedVoiceCoaching}
          importedRecordingCloudVideoReviewEnabled={practiceCloudVideoReviewEnabled}
          importedRecordingAllowCloudVideoForThisReview={practiceAllowCloudVideoForThisReview}
          voiceProfileStatus={voiceProfileStatus}
          voiceMatcherStatus={voiceMatcherStatus}
          voiceDiarizationStatus={voiceDiarizationStatus}
          voiceDiarizationResult={voiceDiarizationResult}
          voiceDiarizationMatchResult={voiceDiarizationMatchResult}
          voiceMatchResult={voiceMatchResult}
          importedVoiceMatchResult={importedVoiceMatchResult}
          importedRecordingResult={importedRecordingResult}
          importedRecordingMessage={importedRecordingMessage}
          onListDevices={() => void handleListDevices()}
          onStartRecording={() => void handleStartRecording()}
          onStopRecording={() => void handleStopRecording()}
          onTranscribeRecording={() => void handleTranscribeRecording()}
          onCalculateMetrics={() => void handleCalculateMetrics()}
          onAnalyzeMeeting={() => void handleAnalyzeMeeting()}
          onSaveTranscriberSettings={() => void handleSaveTranscriberSettings()}
          onTranscriberBinPathChange={setTranscriberBinPath}
          onTranscriberModelPathChange={setTranscriberModelPath}
          onSpeakerEmbeddingModelPathChange={setSpeakerEmbeddingModelPath}
          onSpeakerSegmentationModelPathChange={setSpeakerSegmentationModelPath}
          onAudioProcessingSettingChange={(nextSettings) => void handleAudioProcessingSettingChange(nextSettings)}
          onDismissNudge={handleDismissNudge}
          onHistorySearchChange={setHistorySearch}
          onHistoryLoadPage={(reset) => void loadHistoryPage(reset)}
          onHistorySelectMeeting={(meetingId) => void handleSelectHistoryMeeting(meetingId)}
          onTrendLimitChange={handleTrendLimitChange}
          onTrendsRefresh={() => void loadTrends()}
          onPrivacyRetentionDaysChange={setPrivacyRetentionDays}
          onPrivacyAnalyzerProviderChange={setPrivacyAnalyzerProvider}
          onPrivacyCloudAnalysisEnabledChange={handlePrivacyCloudAnalysisEnabledChange}
          onSavePrivacySettings={() => void handleSavePrivacySettings()}
          onPracticeTitleChange={setPracticeTitle}
          onPracticeImportPathChange={setPracticeImportPath}
          onPracticeFfmpegPathChange={setPracticeFfmpegPath}
          onPracticeCloudVideoReviewEnabledChange={handlePracticeCloudVideoReviewEnabledChange}
          onPracticeAllowCloudVideoForThisReviewChange={setPracticeAllowCloudVideoForThisReview}
          onPracticeLoadCameras={() => void handlePracticeLoadCameras()}
          onPracticeStartCameraRecording={() => void handlePracticeStartCameraRecording()}
          onPracticeStopCameraRecording={() => void handlePracticeStopCameraRecording()}
          onPracticeImportVideo={() => void handlePracticeImportVideo()}
          onPracticeAnalyzeAudio={() => void handlePracticeAnalyzeAudio()}
          onPracticeAnalyzeCombined={() => void handlePracticeAnalyzeCombined()}
          onPracticeRefreshHistory={() => void handlePracticeRefreshHistory()}
          practiceVideoPreviewRef={setPracticeVideoPreviewElement}
          onImportedRecordingSourcePathChange={setImportedRecordingSourcePath}
          onImportedRecordingFfmpegPathChange={setImportedRecordingFfmpegPath}
          onImportedRecordingIncludeSpeakingImprovementsChange={setImportedRecordingIncludeSpeakingImprovements}
          onImportedRecordingUseMatchedVoiceCoachingChange={setImportedRecordingUseMatchedVoiceCoaching}
          onImportedRecordingCloudVideoReviewEnabledChange={handlePracticeCloudVideoReviewEnabledChange}
          onImportedRecordingAllowCloudVideoForThisReviewChange={setPracticeAllowCloudVideoForThisReview}
          onEnrollVoiceProfile={() => void handleEnrollVoiceProfile()}
          onPrepareVoiceProfileForMatching={() => void handlePrepareVoiceProfileForMatching()}
          onTestVoiceProfileMatch={() => void handleTestVoiceProfileMatch()}
          onMatchImportedRecordingVoice={() => void handleMatchImportedRecordingVoice()}
          onDiarizeImportedRecordingSpeakers={() => void handleDiarizeImportedRecordingSpeakers()}
          onMatchImportedRecordingSpeakerSegments={() => void handleMatchImportedRecordingSpeakerSegments()}
          onDeleteVoiceProfile={() => void handleDeleteVoiceProfile()}
          onImportRecordingSummary={() => void handleImportRecordingSummary()}
        />
      </section>
    </main>
  );
};
