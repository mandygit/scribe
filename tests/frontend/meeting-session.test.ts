import { describe, expect, it } from 'bun:test';

import { completeMeetingSession, MeetingReviewError, type MeetingReviewPhase } from '../../src/meeting-session';
import type {
  AnalysisResult,
  LiveNudgeEvent,
  MetricsCalculationResult,
  RecordingMetadata,
  TranscriptionResult,
  TranscriptStreamEvent,
  TranscriptStreamSummary,
} from '../../src/tauri-commands';

const recording: RecordingMetadata = {
  meetingId: 'meeting-1',
  filePath: '/tmp/meeting-1.wav',
  systemAudioFilePath: null,
  durationMs: 60_000,
  sampleRateHz: 48_000,
  byteSize: 2_048,
  systemAudioByteSize: null,
  startedAtMs: 1_000,
  stoppedAtMs: 61_000,
  droppedSampleCount: 0,
  streamError: null,
  systemAudioStreamError: null,
};

const transcription: TranscriptionResult = {
  meetingId: 'meeting-1',
  segmentCount: 1,
  segments: [
    {
      sequenceNumber: 1,
      speakerLabel: 'User',
      text: 'We should ship this behind a flag.',
      startedAtMs: 0,
      endedAtMs: 2_000,
    },
  ],
};

const firstTranscriptSegment = transcription.segments[0];

if (firstTranscriptSegment === undefined) {
  throw new Error('Expected a transcript segment for the meeting-session test fixtures.');
}

const metrics: MetricsCalculationResult = {
  meetingId: 'meeting-1',
  summary: {
    fillerWordCount: 1,
    fillerWordRate: 0.03,
    hedgingPhraseCount: 0,
    wordCount: 30,
    durationMs: 60_000,
    wordsPerMinute: 120,
    userTalkTimeMs: 40_000,
    longestMonologueMs: 18_000,
  },
  metrics: [
    {
      id: 'metric-1',
      meetingId: 'meeting-1',
      name: 'words_per_minute',
      value: 120,
      unit: 'wpm',
      createdAtMs: 62_000,
    },
  ],
};

const analysis: AnalysisResult = {
  meetingId: 'meeting-1',
  reportId: 'report-1',
  generatedAtMs: 63_000,
  analysis: {
    overallScore: 84,
    observations: [
      {
        category: 'clarity',
        score: 84,
        quote: 'We should ship this behind a flag.',
        speakerLabel: 'User',
        contextQuote: null,
        contextSpeakerLabel: null,
        suggestion: 'Lead with the recommendation, then the rollout detail.',
      },
    ],
  },
  scorecard: {
    filler: { score: 88, unavailableReason: null },
    pace: { score: 81, unavailableReason: null },
    clarity: { score: 84, unavailableReason: null },
    talkTime: { score: 79, unavailableReason: null },
    analysis: { score: 84, unavailableReason: null },
    overall: { score: 83, unavailableReason: null },
  },
};

const activeSegmentEvent: TranscriptStreamEvent = {
  meetingId: 'meeting-1',
  segment: firstTranscriptSegment,
  isFinal: true,
};

const otherSegmentEvent: TranscriptStreamEvent = {
  meetingId: 'meeting-2',
  segment: {
    sequenceNumber: 99,
    speakerLabel: 'Other',
    text: 'Ignore me.',
    startedAtMs: 0,
    endedAtMs: 1_000,
  },
  isFinal: true,
};

const streamSummary: TranscriptStreamSummary = {
  meetingId: 'meeting-1',
  segmentCount: 1,
  droppedEventCount: 0,
};

const liveNudge: LiveNudgeEvent = {
  id: 'nudge-1',
  meetingId: 'meeting-1',
  category: 'fillerWords',
  severity: 'caution',
  message: 'Filler words are clustering.',
  suggestion: 'Pause before the next sentence.',
  evidence: 'um we should ship this',
  occurredAtMs: 2_000,
};

const otherMeetingNudge: LiveNudgeEvent = {
  ...liveNudge,
  id: 'nudge-2',
  meetingId: 'meeting-2',
};

describe('completeMeetingSession', () => {
  it('stops the session and automatically runs review stages with live updates', async () => {
    const order: string[] = [];
    const seenPhases: MeetingReviewPhase[] = [];
    const seenSegments: TranscriptStreamEvent[] = [];
    const seenSummaries: TranscriptStreamSummary[] = [];
    const seenNudges: LiveNudgeEvent[] = [];
    let transcriptListeners: {
      onSegment: (event: TranscriptStreamEvent) => void;
      onComplete: (summary: TranscriptStreamSummary) => void;
    } | null = null;
    let nudgeListeners: {
      onNudge: (event: LiveNudgeEvent) => void;
    } | null = null;

    const result = await completeMeetingSession(
      {
        stopRecording: async () => {
          order.push('stop');
          return recording;
        },
        listenToTranscriptStream: async (listeners) => {
          order.push('listen-transcript');
          transcriptListeners = listeners;
          return () => {
            order.push('unlisten-transcript');
          };
        },
        listenToLiveNudges: async (listeners) => {
          order.push('listen-nudges');
          nudgeListeners = listeners;
          return () => {
            order.push('unlisten-nudges');
          };
        },
        transcribeMeeting: async (meetingId) => {
          order.push(`transcribe:${meetingId}`);
          transcriptListeners?.onSegment(activeSegmentEvent);
          transcriptListeners?.onSegment(otherSegmentEvent);
          transcriptListeners?.onComplete(streamSummary);
          nudgeListeners?.onNudge(liveNudge);
          nudgeListeners?.onNudge(otherMeetingNudge);
          return transcription;
        },
        calculateMetrics: async (meetingId) => {
          order.push(`metrics:${meetingId}`);
          return metrics;
        },
        analyzeMeeting: async (meetingId) => {
          order.push(`analyze:${meetingId}`);
          return analysis;
        },
      },
      {
        onPhaseChange: (phase) => {
          seenPhases.push(phase);
          order.push(`phase:${phase}`);
        },
        onRecordingSaved: (savedRecording) => {
          order.push(`saved:${savedRecording.meetingId}`);
        },
        onTranscriptionReady: (savedTranscription) => {
          order.push(`transcribed:${savedTranscription.meetingId}`);
        },
        onMetricsReady: (savedMetrics) => {
          order.push(`metrics-ready:${savedMetrics.meetingId}`);
        },
        listeners: {
          onSegment: (event) => {
            seenSegments.push(event);
          },
          onComplete: (summary) => {
            seenSummaries.push(summary);
          },
          onNudge: (event) => {
            seenNudges.push(event);
          },
        },
      },
    );

    expect(seenPhases).toEqual(['transcribing', 'calculatingMetrics', 'analyzing']);
    expect(order).toEqual([
      'stop',
      'saved:meeting-1',
      'listen-transcript',
      'listen-nudges',
      'phase:transcribing',
      'transcribe:meeting-1',
      'transcribed:meeting-1',
      'phase:calculatingMetrics',
      'metrics:meeting-1',
      'metrics-ready:meeting-1',
      'phase:analyzing',
      'analyze:meeting-1',
      'unlisten-transcript',
      'unlisten-nudges',
    ]);
    expect(seenSegments).toEqual([activeSegmentEvent]);
    expect(seenSummaries).toEqual([streamSummary]);
    expect(seenNudges).toEqual([liveNudge]);
    expect(result).toEqual({
      recording,
      transcription,
      metrics,
      analysis,
    });
  });

  it('cleans up live listeners when transcription fails', async () => {
    const order: string[] = [];
    const transcribeError = new Error('whisper-cli failed');

    const result = await completeMeetingSession(
      {
        stopRecording: async () => recording,
        listenToTranscriptStream: async () => {
          order.push('listen-transcript');
          return () => {
            order.push('unlisten-transcript');
          };
        },
        listenToLiveNudges: async () => {
          order.push('listen-nudges');
          return () => {
            order.push('unlisten-nudges');
          };
        },
        transcribeMeeting: async () => {
          throw transcribeError;
        },
        calculateMetrics: async () => metrics,
        analyzeMeeting: async () => analysis,
      },
      {
        onPhaseChange: () => {
          order.push('phase');
        },
      },
    ).catch((error: unknown) => error);

    expect(result).toBeInstanceOf(MeetingReviewError);
    expect(result).toMatchObject({
      cause: transcribeError,
      message: 'whisper-cli failed',
      phase: 'transcribing',
      recording,
    });

    expect(order).toEqual(['listen-transcript', 'listen-nudges', 'phase', 'unlisten-transcript', 'unlisten-nudges']);
  });

  it('preserves saved recording metadata when report generation fails', async () => {
    const analyzeError = new Error('ollama schema mismatch');

    const result = await completeMeetingSession(
      {
        stopRecording: async () => recording,
        listenToTranscriptStream: async () => () => {},
        listenToLiveNudges: async () => () => {},
        transcribeMeeting: async () => transcription,
        calculateMetrics: async () => metrics,
        analyzeMeeting: async () => {
          throw analyzeError;
        },
      },
      {},
    ).catch((error: unknown) => error);

    expect(result).toBeInstanceOf(MeetingReviewError);
    expect(result).toMatchObject({
      cause: analyzeError,
      message: 'ollama schema mismatch',
      phase: 'analyzing',
      recording,
    });
  });
});
