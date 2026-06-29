import type { LiveNudgeEvent } from '../tauri-commands';
import { NUDGE_SEVERITY_LABELS } from './labels';

interface CoachDockProps {
  recordingMeetingId: string | null;
  isRecorderBusy: boolean;
  recordingIndicatorLabel: string;
  statusMessage: string;
  latestVisibleNudge: LiveNudgeEvent | null;
  onToggleRecording: () => void;
}

export const CoachDock = ({
  recordingMeetingId,
  isRecorderBusy,
  recordingIndicatorLabel,
  statusMessage,
  latestVisibleNudge,
  onToggleRecording,
}: CoachDockProps) => {
  const recordingState = recordingMeetingId ? 'recording' : isRecorderBusy ? 'reviewing' : 'idle';

  return (
    <section className="coach-dock" aria-labelledby="coach-dock-title">
      <div className="recording-pill" data-state={recordingState} role="status">
        <span aria-hidden="true" />
        <strong>{recordingIndicatorLabel}</strong>
      </div>

      <div className="coach-dock-copy">
        <span className="status-label">Live coach</span>
        <h2 id="coach-dock-title">Meeting cockpit</h2>
        <p>Start once, speak naturally, then let Resonance turn the recording into transcript-backed coaching below.</p>
      </div>

      <div className="coach-dock-nudge" aria-live="polite">
        {latestVisibleNudge ? (
          <>
            <span>{NUDGE_SEVERITY_LABELS[latestVisibleNudge.severity]}</span>
            <strong>{latestVisibleNudge.message}</strong>
            <p>{latestVisibleNudge.suggestion}</p>
          </>
        ) : isRecorderBusy ? (
          <>
            <span>Reviewing</span>
            <strong>Building your report</strong>
            <p>{statusMessage}</p>
          </>
        ) : (
          <>
            <span>Quiet</span>
            <strong>No active nudge</strong>
            <p>
              {recordingMeetingId ? 'Listening for communication signals.' : 'Start capture when your meeting begins.'}
            </p>
          </>
        )}
      </div>

      <button type="button" className="coach-primary-action" onClick={onToggleRecording} disabled={isRecorderBusy}>
        {recordingMeetingId ? 'Stop session' : 'Start session'}
      </button>
    </section>
  );
};
