import { describe, expect, it } from 'bun:test';
import { formatClock, formatDuration, meetingTitle } from '../../src/format';

describe('formatDuration', () => {
  it('returns a dash for null or sub-second input', () => {
    expect(formatDuration(null)).toBe('—');
    expect(formatDuration(500)).toBe('—');
  });

  it('shows seconds under a minute', () => {
    expect(formatDuration(4_000)).toBe('4s');
  });

  it('rounds to whole minutes above a minute', () => {
    expect(formatDuration(34 * 60_000)).toBe('34 min');
    expect(formatDuration(90_000)).toBe('2 min');
  });
});

describe('formatClock', () => {
  it('formats minutes and zero-padded seconds', () => {
    expect(formatClock(0)).toBe('0:00');
    expect(formatClock(9)).toBe('0:09');
    expect(formatClock(768)).toBe('12:48');
  });

  it('clamps negative input', () => {
    expect(formatClock(-5)).toBe('0:00');
  });
});

describe('meetingTitle', () => {
  it('uses the title when present', () => {
    expect(meetingTitle({ title: 'Q3 planning', startedAtMs: 0 })).toBe('Q3 planning');
  });

  it('falls back to a dated label when title is blank', () => {
    const label = meetingTitle({ title: null, startedAtMs: Date.UTC(2026, 5, 27) });
    expect(label.startsWith('Meeting · ')).toBe(true);
  });
});
