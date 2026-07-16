import { describe, expect, it } from 'bun:test';
import { createNoteBlock, numberedIndex, parseNotes, serializeNotes } from '../../src/user-notes';

describe('parseNotes', () => {
  it('yields a single empty bullet for empty or null content', () => {
    for (const content of [null, '', '   \n  ']) {
      const blocks = parseNotes(content);
      expect(blocks.length).toBe(1);
      expect(blocks[0].type).toBe('bullet');
      expect(blocks[0].text).toBe('');
    }
  });

  it('recognizes bullets, numbers, todos, and plain text', () => {
    const blocks = parseNotes(
      'Prep questions\n- ask about budget\n* second bullet\n1. first\n2. second\n- [ ] send recap\n- [x] book room',
    );
    expect(blocks.map((block) => block.type)).toEqual(['text', 'bullet', 'bullet', 'number', 'number', 'todo', 'todo']);
    expect(blocks[1].text).toBe('ask about budget');
    expect(blocks[5].checked).toBe(false);
    expect(blocks[6].checked).toBe(true);
    expect(blocks[6].text).toBe('book room');
  });

  it('round-trips through serializeNotes', () => {
    const content = 'Prep\n- one\n1. two\n2. three\n- [x] done\n- [ ] todo';
    expect(serializeNotes(parseNotes(content))).toBe(content);
  });
});

describe('serializeNotes', () => {
  it('serializes an effectively empty document to an empty string', () => {
    expect(serializeNotes([createNoteBlock('bullet'), createNoteBlock('todo')])).toBe('');
  });

  it('numbers consecutive numbered blocks from their run position', () => {
    const blocks = [
      createNoteBlock('number', 'a'),
      createNoteBlock('number', 'b'),
      createNoteBlock('text', 'break'),
      createNoteBlock('number', 'c'),
    ];
    expect(serializeNotes(blocks)).toBe('1. a\n2. b\nbreak\n1. c');
  });
});

describe('numberedIndex', () => {
  it('restarts counting after a non-numbered block', () => {
    const blocks = [
      createNoteBlock('number', 'a'),
      createNoteBlock('number', 'b'),
      createNoteBlock('bullet', 'x'),
      createNoteBlock('number', 'c'),
    ];
    expect(numberedIndex(blocks, 0)).toBe(1);
    expect(numberedIndex(blocks, 1)).toBe(2);
    expect(numberedIndex(blocks, 3)).toBe(1);
  });
});
