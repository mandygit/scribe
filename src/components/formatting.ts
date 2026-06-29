export const formatDuration = (durationMs: number): string => {
  const totalSeconds = Math.max(0, durationMs / 1_000);
  const roundedTenths = Math.round(totalSeconds * 10) / 10;

  if (roundedTenths < 60) {
    return `${roundedTenths.toFixed(1)}s`;
  }

  const roundedSeconds = Math.round(totalSeconds);
  const minutes = Math.floor(roundedSeconds / 60);
  const seconds = roundedSeconds % 60;

  return `${minutes}m ${seconds}s`;
};

export const formatNumber = (value: number, maximumFractionDigits = 0): string => {
  if (!Number.isFinite(value)) {
    return '0';
  }

  return new Intl.NumberFormat('en', {
    maximumFractionDigits,
  }).format(value);
};

export const formatPercent = (ratio: number): string => `${formatNumber(ratio * 100, 1)}%`;

export const formatScore = (score: number | null): string => {
  return score === null ? 'Unavailable' : `${score}/100`;
};

export const formatDateTime = (timestampMs: number): string =>
  new Intl.DateTimeFormat('en', {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(timestampMs));

export const scoreTone = (score: number | null): string => {
  if (score === null) {
    return 'Needs data';
  }

  if (score >= 85) {
    return 'Strong signal';
  }

  if (score >= 65) {
    return 'Developing';
  }

  return 'Needs attention';
};
