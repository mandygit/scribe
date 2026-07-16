import { useCallback, useEffect, useRef, useState } from 'react';
import './pill.css';
import {
  type DictationState,
  getAppStatus,
  isTauriRuntime,
  listenToDictationLevel,
  listenToDictationPillHover,
  listenToDictationState,
  listenToPolishSelectionNotice,
  type PillLayout,
  setPillLayout,
  toggleDictation,
} from './tauri-commands';

/** How long a polish-selection notice (e.g. "select text first") stays visible. */
const NOTICE_DURATION_MS = 2500;

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
  const noticeTimeoutRef = useRef<number | null>(null);
  const barsRef = useRef<Array<HTMLSpanElement | null>>([]);
  const levelsRef = useRef<number[]>(Array.from({ length: BAR_COUNT }, () => 0));

  const layout: PillLayout = notice !== null ? 'notice' : state === 'idle' && hovered ? 'hover' : state;

  const showNotice = useCallback((message: string) => {
    if (noticeTimeoutRef.current !== null) {
      window.clearTimeout(noticeTimeoutRef.current);
    }
    setNotice(message);
    noticeTimeoutRef.current = window.setTimeout(() => setNotice(null), NOTICE_DURATION_MS);
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
      devWindow.__scribePillDev = { setState: handleState, showNotice, pushLevel };
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
