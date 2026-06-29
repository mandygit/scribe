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
