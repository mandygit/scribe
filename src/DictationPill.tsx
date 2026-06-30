import { useEffect, useState } from 'react';
import './pill.css';
import {
  type DictationState,
  getAppStatus,
  isTauriRuntime,
  listenToDictationState,
  toggleDictation,
  updateDictationSettings,
} from './tauri-commands';

const LABELS: Record<DictationState, string> = {
  idle: 'Dictation',
  listening: 'Listening…',
  transcribing: 'Transcribing…',
};

/**
 * The floating dictation pill: a small Wispr-style bar in its own always-on-top,
 * non-focusable window. The mic button toggles dictation (same flow as the global
 * hotkey); the pill reflects idle → listening → transcribing via backend events.
 * The Polish toggle mirrors the app's dictation-polish setting.
 */
export default function DictationPill() {
  const [state, setState] = useState<DictationState>('idle');
  const [polishEnabled, setPolishEnabled] = useState(false);
  const [hotkey, setHotkey] = useState('cmd+shift+d');

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      try {
        const status = await getAppStatus();
        if (!cancelled) {
          setPolishEnabled(status.defaultSettings.dictationPolishEnabled);
          setHotkey(status.defaultSettings.dictationHotkey);
        }
      } catch {
        // Fall back to defaults; the pill still works without prior settings.
      }
      const handle = await listenToDictationState((next) => setState(next));
      if (cancelled) {
        handle();
      } else {
        unlisten = handle;
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const handleMic = () => {
    void toggleDictation();
  };

  const handlePolish = () => {
    const next = !polishEnabled;
    setPolishEnabled(next);
    // Optimistic; revert if the persist fails.
    void updateDictationSettings(hotkey, next).catch(() => setPolishEnabled(!next));
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
      <span className="dpill__label">{LABELS[state]}</span>
      <button
        type="button"
        className={`dpill__polish${polishEnabled ? ' is-on' : ''}`}
        onClick={handlePolish}
        aria-pressed={polishEnabled}
        title="Polish dictation with Apple Intelligence"
      >
        Polish
      </button>
    </div>
  );
}
