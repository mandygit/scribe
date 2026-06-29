import { describe, expect, it } from 'bun:test';

import { messageFromUnknownError } from '../../src/error-utils';

describe('messageFromUnknownError', () => {
  it('returns the message from Error instances', () => {
    expect(messageFromUnknownError(new Error('native bridge failed'), 'fallback')).toBe('native bridge failed');
  });

  it('returns the message from Tauri-style error objects', () => {
    expect(
      messageFromUnknownError(
        {
          code: 'transcriber_model_not_configured',
          message: 'Configure a whisper.cpp model path before transcribing meetings.',
          details: null,
        },
        'fallback',
      ),
    ).toBe('Configure a whisper.cpp model path before transcribing meetings.');
  });

  it('falls back when the value has no usable message', () => {
    expect(messageFromUnknownError({ code: 'unknown' }, 'fallback')).toBe('fallback');
  });
});
