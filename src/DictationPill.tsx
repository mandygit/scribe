import './pill.css';

/**
 * The floating dictation pill: a small bar pinned to the bottom-center of the
 * screen in its own always-on-top, non-focusable Tauri window. This is the
 * static shell — the state machine and controls are wired in a later step.
 */
export default function DictationPill() {
  return (
    <div className="dpill" data-state="idle">
      <button type="button" className="dpill__mic" aria-label="Start dictation">
        <span className="dpill__mic-dot" />
      </button>
      <span className="dpill__label">Dictation</span>
    </div>
  );
}
