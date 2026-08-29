export const formatDate = (ms: number): string =>
  new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });

export const formatDuration = (ms: number | null): string => {
  if (!ms || ms < 1000) return '—';
  const totalSeconds = Math.round(ms / 1000);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.round(totalSeconds / 60);
  return `${minutes} min`;
};

export const formatClock = (seconds: number): string => {
  const safeSeconds = Math.max(0, Math.floor(seconds));
  const m = Math.floor(safeSeconds / 60);
  const s = safeSeconds % 60;
  return `${m}:${s.toString().padStart(2, '0')}`;
};

export const meetingTitle = (item: { title: string | null; startedAtMs: number }): string =>
  item.title?.trim() ? item.title : `Meeting · ${formatDate(item.startedAtMs)}`;

/** Renders "cmd+shift+d" as the macOS-style "⌘⇧D", and "fn" as "Fn". */
export const formatHotkey = (hotkey: string): string =>
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
        // The only token naming a whole key rather than a modifier. Spelled
        // out rather than shown as 🌐, which renders as a full-colour emoji
        // and breaks the monochrome run of ⌘⇧⌥ glyphs beside it. Settings
        // still shows the globe, where picking the key is the point.
        case 'fn':
          return 'Fn';
        default:
          return token.toUpperCase();
      }
    })
    .join('');
