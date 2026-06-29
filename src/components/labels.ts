import type { LiveNudgeEvent, MeetingHistoryItem } from '../tauri-commands';

export type ReportDimensionKey = 'filler' | 'pace' | 'clarity' | 'talkTime' | 'analysis';

export interface ReportDimensionConfig {
  key: ReportDimensionKey;
  label: string;
  cue: string;
}

export const REPORT_DIMENSIONS: ReportDimensionConfig[] = [
  { key: 'filler', label: 'Filler control', cue: 'Tracks filler word density against spoken words.' },
  { key: 'pace', label: 'Pace', cue: 'Rewards a steady 120-170 words per minute delivery.' },
  { key: 'clarity', label: 'Clarity', cue: 'Looks for hedging phrases that weaken commitments.' },
  { key: 'talkTime', label: 'Talk-time shape', cue: 'Checks how much of the meeting carried your voice.' },
  { key: 'analysis', label: 'Coaching analysis', cue: "Uses the local analyzer's holistic speaking score." },
];

export const NUDGE_CATEGORY_LABELS: Record<LiveNudgeEvent['category'], string> = {
  fillerWords: 'Fillers',
  hedging: 'Hedging',
  pace: 'Pace',
  talkTime: 'Talk time',
};

export const NUDGE_SEVERITY_LABELS: Record<LiveNudgeEvent['severity'], string> = {
  info: 'Notice',
  caution: 'Coach',
  urgent: 'Pause',
};

export const HISTORY_STATUS_LABELS: Record<MeetingHistoryItem['status'], string> = {
  recording: 'Recording',
  recorded: 'Recorded',
  transcribed: 'Transcribed',
  analyzed: 'Analyzed',
  failed_partial: 'Needs retry',
};
