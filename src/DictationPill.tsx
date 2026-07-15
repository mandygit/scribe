import { useEffect, useRef, useState } from 'react';
import './pill.css';
import {
  type DictationState,
  isTauriRuntime,
  listenToDictationState,
  listenToPolishSelectionNotice,
  toggleDictation,
} from './tauri-commands';

/** How long a polish-selection notice (e.g. "select text first") stays visible. */
const NOTICE_DURATION_MS = 2500;

const LABELS: Record<DictationState, string> = {
  idle: 'Dictation',
  listening: 'Listening…',
  transcribing: 'Transcribing…',
};

/**
 * The floating dictation pill: a small Wispr-style bar in its own always-on-top,
 * non-focusable window. The mic button toggles dictation (same flow as the global
 * hotkey); the pill reflects idle → listening → transcribing via backend events,
 * and briefly shows polish-selection feedback (e.g. "select text first").
 * Dictation polish itself is configured from Settings, not from the pill.
 */
export default function DictationPill() {
  const [state, setState] = useState<DictationState>('idle');
  const [notice, setNotice] = useState<string | null>(null);
  const noticeTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }
    let unlistenState: (() => void) | undefined;
    let unlistenNotice: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      try {
        const stateHandle = await listenToDictationState((next) => setState(next));
        const noticeHandle = await listenToPolishSelectionNotice((message) => {
          if (noticeTimeoutRef.current !== null) {
            window.clearTimeout(noticeTimeoutRef.current);
          }
          setNotice(message);
          noticeTimeoutRef.current = window.setTimeout(() => setNotice(null), NOTICE_DURATION_MS);
        });
        if (cancelled) {
          stateHandle();
          noticeHandle();
        } else {
          unlistenState = stateHandle;
          unlistenNotice = noticeHandle;
        }
      } catch (cause) {
        // A rejection here means the event plugin denied the registration --
        // almost certainly this window's label missing from
        // src-tauri/capabilities/default.json. Without this catch that
        // rejection is swallowed and looks like events silently never firing.
        console.error('DictationPill: event listener registration failed', cause);
      }
    })();
    return () => {
      cancelled = true;
      unlistenState?.();
      unlistenNotice?.();
      if (noticeTimeoutRef.current !== null) {
        window.clearTimeout(noticeTimeoutRef.current);
      }
    };
  }, []);

  const handleMic = () => {
    void toggleDictation();
  };

  return (
    <div className="dpill" data-state={state}>
      <button
        type="button"
        className="dpill__mic"
        onClick={handleMic}
        aria-label={state === 'idle' ? 'Start dictation' : 'Stop dictation'}
      >
        <span className="dpill__mic-dot" />
      </button>
      <span className={`dpill__label${notice ? ' dpill__label--notice' : ''}`}>{notice ?? LABELS[state]}</span>
    </div>
  );
}
