import { describe, expect, it } from 'bun:test';
import { formatDuration } from '../../src/components/formatting';

describe('formatDuration edge cases', () => {
  it('should not show "60.0s" or "Xm 60s"', () => {
    // 59.999 seconds should round to 60s, but then should be shown as "1m 0s"
    const result1 = formatDuration(59_999);
    console.log('formatDuration(59999):', result1);

    // 119.999 seconds might produce "1m 60s" which is invalid
    const result2 = formatDuration(119_999);
    console.log('formatDuration(119999):', result2);

    // These are the actual bugs
    expect(result1).not.toBe('60.0s'); // Will fail - this IS what it returns
    expect(result2).not.toContain('60s'); // Will fail if seconds are 60
  });
});
