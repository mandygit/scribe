import { useCallback, useEffect, useState } from 'react';
import './meeting-popup.css';
import {
  dismissMeetingPrompt,
  isTauriRuntime,
  listenToMeetingCallEnded,
  listenToMeetingDetected,
  setMeetingPlaceholderTitle,
  startRecording,
} from './tauri-commands';

/**
 * The floating "Record this meeting?" prompt: a small always-on-top bar in
 * its own window, shown by the Rust backend the moment it detects a live
 * Microsoft Teams call (see `meeting_detection` on the Rust side). Mirrors
 * DictationPill's shape -- a tiny component in its own transparent window,
 * driven entirely by backend events, guarded by `isTauriRuntime()`.
 */
export default function MeetingPopup() {
  const [meetingId, setMeetingId] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }
    let unlistenDetected: (() => void) | undefined;
    let unlistenEnded: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      try {
        const detectedHandle = await listenToMeetingDetected((id) => {
          setStarting(false);
          setMeetingId(id);
        });
        const endedHandle = await listenToMeetingCallEnded(() => setMeetingId(null));
        if (cancelled) {
          detectedHandle();
          endedHandle();
        } else {
          unlistenDetected = detectedHandle;
          unlistenEnded = endedHandle;
        }
      } catch (cause) {
        // A rejection here means the event plugin denied the registration --
        // almost certainly this window's label missing from
        // src-tauri/capabilities/default.json. Without this catch that
        // rejection is swallowed and looks like events silently never firing.
        console.error('MeetingPopup: event listener registration failed', cause);
      }
    })();
    return () => {
      cancelled = true;
      unlistenDetected?.();
      unlistenEnded?.();
    };
  }, []);

  const handleRecord = useCallback(() => {
    if (!meetingId || starting) {
      return;
    }
    setStarting(true);
    void (async () => {
      try {
        await startRecording(meetingId);
        // Best-effort labeling; a failure here shouldn't undo the recording
        // that already started. A placeholder title, so the summarizer's
        // generated title can still replace it.
        await setMeetingPlaceholderTitle(meetingId, 'Teams Meeting').catch(() => {});
      } catch {
        // The main Scribe window's history/recording state is the source of
        // truth if this fails -- the popup itself has nothing further to
        // report, so it just closes either way.
      } finally {
        setMeetingId(null);
        setStarting(false);
      }
    })();
  }, [meetingId, starting]);

  const handleDismiss = useCallback(() => {
    setMeetingId(null);
    void dismissMeetingPrompt();
  }, []);

  if (!meetingId) {
    return <div className="mpopup" data-visible="false" />;
  }

  return (
    <div className="mpopup" data-visible="true">
      <span className="mpopup__label">Record this Teams meeting?</span>
      <div className="mpopup__actions">
        <button type="button" className="mpopup__btn mpopup__btn--dismiss" onClick={handleDismiss}>
          Dismiss
        </button>
        <button type="button" className="mpopup__btn mpopup__btn--record" onClick={handleRecord} disabled={starting}>
          {starting ? 'Starting…' : 'Record'}
        </button>
      </div>
    </div>
  );
}
