import { describe, expect, it } from 'bun:test';

import { formatDuration, formatNumber, formatPercent, formatScore, scoreTone } from '../../src/components/formatting';

describe('UI formatting helpers', () => {
  it('formats short and minute-scale durations', () => {
    expect(formatDuration(1_250)).toBe('1.3s');
    expect(formatDuration(65_000)).toBe('1m 5s');
  });

  it('normalizes rounded seconds at minute boundaries', () => {
    expect(formatDuration(59_600)).toBe('59.6s');
    expect(formatDuration(59_999)).toBe('1m 0s');
    expect(formatDuration(119_999)).toBe('2m 0s');
  });

  it('formats invalid numeric values defensively', () => {
    expect(formatNumber(Number.NaN)).toBe('0');
    expect(formatNumber(Number.POSITIVE_INFINITY)).toBe('0');
  });

  it('formats percentages and optional scores for report surfaces', () => {
    expect(formatPercent(0.123)).toBe('12.3%');
    expect(formatScore(null)).toBe('Unavailable');
    expect(formatScore(88)).toBe('88/100');
  });

  it('maps scores to user-facing tone labels', () => {
    expect(scoreTone(null)).toBe('Needs data');
    expect(scoreTone(42)).toBe('Needs attention');
    expect(scoreTone(65)).toBe('Developing');
    expect(scoreTone(85)).toBe('Strong signal');
  });
});
