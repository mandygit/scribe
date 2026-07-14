import { useCallback, useState } from 'react';
import './recording-indicator.css';
import { stopRecording } from './tauri-commands';

/**
 * The floating recording-in-progress indicator: a small always-on-top
 * vertical capsule with a stop button, shown by the Rust backend for any
 * active recording (started from the main window's button or the
 * meeting-detection popup's Record button). Unlike DictationPill/MeetingPopup,
 * this component needs no backend event listeners of its own — Rust drives
 * the window's own visibility directly (see `emit_recording_started`/
 * `emit_recording_stopped` in `lib.rs`), so there's nothing else to react to.
 */
export default function RecordingIndicator() {
  const [stopping, setStopping] = useState(false);

  const handleStop = useCallback(() => {
    if (stopping) {
      return;
    }
    setStopping(true);
    void (async () => {
      try {
        await stopRecording();
      } catch {
        // The main window's own error banner is the source of truth for a
        // failed stop; this pill just resets so the user can try again.
      } finally {
        setStopping(false);
      }
    })();
  }, [stopping]);

  return (
    <div className="rindicator">
      <div className="rindicator__handle" data-tauri-drag-region>
        <span className="rindicator__dot" />
        <span className="rindicator__grip">
          <span />
          <span />
          <span />
        </span>
      </div>
      <button
        type="button"
        className="rindicator__stop"
        onClick={handleStop}
        disabled={stopping}
        aria-label="Stop recording"
      >
        <span className="rindicator__stop-icon" />
      </button>
    </div>
  );
}
