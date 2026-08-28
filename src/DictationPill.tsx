import { useCallback, useEffect, useRef, useState } from 'react';
import './pill.css';
import {
  copyLastDictation,
  type DictationPasteFailure,
  type DictationPasteFailureReason,
  type DictationState,
  getAppStatus,
  isTauriRuntime,
  listenToDictationLevel,
  listenToDictationPasteFailed,
  listenToDictationPillHover,
  listenToDictationState,
  listenToPolishSelectionNotice,
  type PillLayout,
  setPillLayout,
  toggleDictation,
} from './tauri-commands';

/** How long a polish-selection notice (e.g. "select text first") stays visible. */
const NOTICE_DURATION_MS = 2500;

/** Characters of transcript shown in the recovery widget before ellipsis. */
const RECOVERY_PREVIEW_CHARS = 140;

/** Bars in the listening waveform; must fit the listening capsule width. */
const BAR_COUNT = 9;

/** Stable render identities for the waveform's fixed positional bar slots. */
const BAR_SLOTS = Array.from({ length: BAR_COUNT }, (_, index) => `bar-${index}`);

/** Waveform bar height range (px), mapped from the normalised mic level. */
const BAR_MIN_HEIGHT = 3;
const BAR_MAX_HEIGHT = 18;

/**
 * How long the capsule's CSS collapse transition runs (see pill.css). Window
 * shrinks are delayed by this so the animation finishes before the window
 * clips it; grows apply immediately so the animation has room to play.
 */
const COLLAPSE_MS = 220;

/** Footprint order of the layouts, to tell window grows from shrinks. */
const LAYOUT_RANK: Record<PillLayout, number> = {
  idle: 0,
  hover: 1,
  listening: 2,
  transcribing: 2,
  notice: 3,
  'paste-failed': 4,
};

/**
 * Perceptual normalisation of the raw RMS level (0..1, speech typically lands
 * around 0.02-0.3): square-root compresses the range so quiet speech still
 * moves the bars, and the gain lifts normal speech towards full height.
 */
const normalizeLevel = (level: number): number => Math.min(1, Math.sqrt(Math.max(0, level)) * 2.2);

/** Renders "cmd+shift+d" as the macOS-style "⌘⇧D". */
const formatHotkey = (hotkey: string): string =>
  hotkey
    .split('+')
    .map((part) => {
      const token = part.trim().toLowerCase();
      switch (token) {
        case 'cmd':
        case 'command':
        case 'super':
        case 'meta':
          return '⌘';
        case 'shift':
          return '⇧';
        case 'option':
        case 'alt':
          return '⌥';
        case 'ctrl':
        case 'control':
          return '⌃';
        case 'space':
          return 'Space';
        default:
          return token.toUpperCase();
      }
    })
    .join('');

/** Browser-only handle for exercising the pill's states without the native shell. */
interface PillDevWindow {
  __scribePillDev?: {
    setState: (state: DictationState) => void;
    showNotice: (message: string) => void;
    pushLevel: (level: number) => void;
    showPasteFailure: (failure: DictationPasteFailure) => void;
  };
}

/**
 * The floating dictation pill, Wispr-style: an unobtrusive sliver at the
 * bottom of the screen that blooms under the cursor into a click-to-dictate
 * capsule, shows a live mic waveform while listening and a progress sweep
 * while transcribing, and never shows a standing text label. Each visual
 * layout also resizes its transparent window to hug the painted content (via
 * `set_pill_layout`) so the idle pill stops covering text boxes.
 */
export default function DictationPill() {
  const [state, setState] = useState<DictationState>('idle');
  const [notice, setNotice] = useState<string | null>(null);
  const [hovered, setHovered] = useState(false);
  const [hotkeyHint, setHotkeyHint] = useState<string | null>(null);
  const [pasteFailure, setPasteFailure] = useState<DictationPasteFailure | null>(null);
  const noticeTimeoutRef = useRef<number | null>(null);
  const barsRef = useRef<Array<HTMLSpanElement | null>>([]);
  const levelsRef = useRef<number[]>(Array.from({ length: BAR_COUNT }, () => 0));

  // The recovery widget outranks every other layout: it is the only one
  // holding text the user cannot get back any other way without opening the
  // app, so a stray hover or notice must not displace it.
  const layout: PillLayout =
    pasteFailure !== null ? 'paste-failed' : notice !== null ? 'notice' : state === 'idle' && hovered ? 'hover' : state;

  const showNotice = useCallback((message: string, durationMs: number = NOTICE_DURATION_MS) => {
    if (noticeTimeoutRef.current !== null) {
      window.clearTimeout(noticeTimeoutRef.current);
    }
    setNotice(message);
    noticeTimeoutRef.current = window.setTimeout(() => setNotice(null), durationMs);
  }, []);

  /** Pushes one mic level into the scrolling waveform, bypassing React state (30 Hz). */
  const pushLevel = useCallback((level: number) => {
    const levels = levelsRef.current;
    levels.push(normalizeLevel(level));
    levels.shift();
    for (let index = 0; index < BAR_COUNT; index += 1) {
      const bar = barsRef.current[index];
      if (bar) {
        bar.style.height = `${BAR_MIN_HEIGHT + Math.round((levels[index] ?? 0) * (BAR_MAX_HEIGHT - BAR_MIN_HEIGHT))}px`;
      }
    }
  }, []);

  const handleState = useCallback((next: DictationState) => {
    setState(next);
    // Starting a new dictation supersedes the previous one's recovery widget:
    // the text behind it is about to be replaced in `AppState::last_dictation`
    // anyway, so leaving the widget up would offer to copy the wrong thing.
    if (next === 'listening') {
      setPasteFailure(null);
    }
    // A resize under a stationary cursor does not fire mouseleave, which
    // would strand the pill in its hover layout; drop the flag and let a real
    // mousemove re-assert it.
    setHovered(false);
    if (next !== 'listening') {
      levelsRef.current = Array.from({ length: BAR_COUNT }, () => 0);
      for (const bar of barsRef.current) {
        if (bar) {
          bar.style.height = `${BAR_MIN_HEIGHT}px`;
        }
      }
    }
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) {
      // No Tauri events exist in a plain browser tab; expose a dev handle so
      // the visual states can still be exercised end-to-end there.
      const devWindow = window as PillDevWindow;
      devWindow.__scribePillDev = {
        setState: handleState,
        showNotice,
        pushLevel,
        showPasteFailure: setPasteFailure,
      };
      return () => {
        delete devWindow.__scribePillDev;
      };
    }
    let unlistenAll: Array<() => void> | undefined;
    let cancelled = false;
    void (async () => {
      try {
        const handles = [
          await listenToDictationState(handleState),
          await listenToPolishSelectionNotice(showNotice),
          await listenToDictationLevel(pushLevel),
          await listenToDictationPillHover(setHovered),
          await listenToDictationPasteFailed(setPasteFailure),
        ];
        if (cancelled) {
          for (const handle of handles) {
            handle();
          }
        } else {
          unlistenAll = handles;
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
      for (const handle of unlistenAll ?? []) {
        handle();
      }
      if (noticeTimeoutRef.current !== null) {
        window.clearTimeout(noticeTimeoutRef.current);
      }
    };
  }, [handleState, pushLevel, showNotice]);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }
    void getAppStatus()
      .then((status) => setHotkeyHint(formatHotkey(status.defaultSettings.dictationHotkey)))
      .catch(() => setHotkeyHint(null));
  }, []);

  // Keep the window hugging the painted content: grows apply immediately so
  // the capsule has room to expand into; shrinks wait for the collapse
  // animation so the window doesn't clip it mid-transition.
  const appliedLayoutRef = useRef<PillLayout>('idle');
  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }
    const previous = appliedLayoutRef.current;
    appliedLayoutRef.current = layout;
    const apply = () => {
      void setPillLayout(layout).catch((cause) => {
        console.error('DictationPill: window resize failed', cause);
      });
    };
    if (LAYOUT_RANK[layout] < LAYOUT_RANK[previous]) {
      const timeout = window.setTimeout(apply, COLLAPSE_MS);
      return () => window.clearTimeout(timeout);
    }
    apply();
  }, [layout]);

  const handleClick = () => {
    void toggleDictation();
  };

  if (pasteFailure !== null) {
    return <PasteRecoveryWidget failure={pasteFailure} onDismiss={() => setPasteFailure(null)} />;
  }

  // Hover comes from the Rust cursor watcher in the native shell (DOM mouse
  // events never fire in the non-activating panel); the DOM handlers are the
  // browser-tab fallback for the dev harness.
  const browserHoverHandlers = isTauriRuntime()
    ? {}
    : { onMouseEnter: () => setHovered(true), onMouseLeave: () => setHovered(false) };

  return (
    <button
      type="button"
      className="dpill"
      data-layout={layout}
      onClick={handleClick}
      {...browserHoverHandlers}
      aria-label={state === 'listening' ? 'Stop dictation' : 'Start dictation'}
    >
      {layout === 'hover' && (
        <span className="dpill__tooltip">
          {hotkeyHint !== null ? `Click or double-tap ${hotkeyHint}` : 'Click to dictate'}
        </span>
      )}
      <span className="dpill__body">
        <svg
          className="dpill__mic"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <rect x="9" y="2" width="6" height="12" rx="3" />
          <path d="M5 10a7 7 0 0 0 14 0" />
          <line x1="12" y1="19" x2="12" y2="22" />
        </svg>
        <span className="dpill__bars" aria-hidden="true">
          {BAR_SLOTS.map((slot, index) => (
            <span
              key={slot}
              className="dpill__bar"
              ref={(element) => {
                barsRef.current[index] = element;
              }}
            />
          ))}
        </span>
        <span className="dpill__sweep" aria-hidden="true">
          <span className="dpill__sweep-thumb" />
        </span>
        <span className="dpill__notice-text">{notice}</span>
      </span>
    </button>
  );
}

/**
 * What the recovery widget says for each way a paste can fail. A lookup rather
 * than nested ternaries because every one of these is a sentence the user
 * reads at the exact moment they are wondering where their words went: they
 * are content, and they belong somewhere they can be read end to end.
 *
 * Each hint says what happened, and where the user's own action is what fixes
 * it, says that instead.
 */
const RECOVERY_COPY: Record<DictationPasteFailureReason, { title: string; hint: string }> = {
  no_target: {
    title: 'Nothing to paste into',
    hint: 'Your cursor wasn\u2019t in a text field.',
  },
  target_not_frontmost: {
    title: 'Couldn\u2019t reach that app',
    hint: 'It never came back to the front, so nothing was pasted.',
  },
  secure_input_active: {
    title: 'Secure input is active',
    hint: 'macOS blocks pasting while a password field or secure terminal has focus.',
  },
  paste_did_not_land: {
    title: 'Nothing was inserted',
    hint: 'The app ignored the paste. The text is on your clipboard.',
  },
  accessibility_denied: {
    title: 'Accessibility permission needed',
    hint: 'Re-grant Scribe in System Settings > Privacy & Security > Accessibility.',
  },
  keystroke_failed: {
    title: 'Couldn\u2019t paste',
    hint: 'The paste keystroke was blocked.',
  },
};

/**
 * The recovery widget: what the user sees when a dictation had nowhere to go.
 *
 * Rendered as its own root rather than another layout inside the pill's
 * capsule because it is the only state with controls of its own - the pill is
 * a single `<button>`, and buttons cannot nest.
 *
 * It never closes itself -- not on a timer, and not after Copy. The user
 * dismisses it, or the next dictation replaces it. A dictation that didn't
 * paste is only recoverable from here or the app's Dictation tab, and anything
 * that disappears on its own can disappear while the user is still staring at
 * the app they expected the text to land in, which is exactly how the text got
 * lost in the first place.
 */
function PasteRecoveryWidget({ failure, onDismiss }: { failure: DictationPasteFailure; onDismiss: () => void }) {
  const [copied, setCopied] = useState(false);
  const message = RECOVERY_COPY[failure.reason];

  const handleCopy = useCallback(() => {
    // In the native shell the text lives in Rust (it was deliberately never
    // put on the clipboard on the no-target path), so the copy has to go
    // through the backend rather than navigator.clipboard -- which is also
    // unavailable to this panel, being a never-key window.
    const copy = isTauriRuntime()
      ? copyLastDictation().then(() => undefined)
      : navigator.clipboard.writeText(failure.text);
    void copy
      .then(() => setCopied(true))
      .catch((cause) => {
        console.error('DictationPill: copying the recovered dictation failed', cause);
      });
  }, [failure.text]);

  const preview =
    failure.text.length > RECOVERY_PREVIEW_CHARS
      ? `${failure.text.slice(0, RECOVERY_PREVIEW_CHARS).trimEnd()}…`
      : failure.text;

  return (
    <div className="dpill dpill--recovery" data-layout="paste-failed">
      <div className="drecover">
        <div className="drecover__head">
          <svg
            className="drecover__icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
          <span className="drecover__title">{message.title}</span>
          <button type="button" className="drecover__close" onClick={onDismiss} aria-label="Dismiss">
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.4"
              strokeLinecap="round"
              aria-hidden="true"
            >
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
        <p className="drecover__text">{preview}</p>
        <div className="drecover__actions">
          <span className="drecover__hint">{message.hint}</span>
          <button type="button" className="drecover__copy" onClick={handleCopy} disabled={copied}>
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
            >
              {copied ? (
                <polyline points="20 6 9 17 4 12" />
              ) : (
                <>
                  <rect x="9" y="9" width="12" height="12" rx="2" />
                  <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                </>
              )}
            </svg>
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
      </div>
    </div>
  );
}
