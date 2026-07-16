/**
 * Block model for the personal meeting-notes editor (the "Notes" tab).
 *
 * Notes are persisted as plain markdown-style text so they stay readable
 * outside the app (and in the DB), and are parsed back into typed blocks
 * for editing: `- ` bullets, `1. ` numbered items, `- [ ]` / `- [x]`
 * checkboxes, and anything else as plain text.
 */

export type NoteBlockType = 'text' | 'bullet' | 'number' | 'todo';

export interface NoteBlock {
  id: number;
  type: NoteBlockType;
  text: string;
  checked: boolean;
}

const TODO_LINE = /^- \[( |x|X)\] (.*)$/;
const BULLET_LINE = /^[-*] (.*)$/;
const NUMBER_LINE = /^\d+\. (.*)$/;

let nextBlockId = 1;

export const createNoteBlock = (type: NoteBlockType, text = '', checked = false): NoteBlock => ({
  id: nextBlockId++,
  type,
  text,
  checked,
});

/** Parses stored notes into editor blocks. Empty content yields a single
 * empty bullet, so the editor always has a line to type into. */
export const parseNotes = (content: string | null): NoteBlock[] => {
  const lines = (content ?? '').replace(/\r\n/g, '\n').split('\n');
  const blocks: NoteBlock[] = [];
  for (const line of lines) {
    const todo = line.match(TODO_LINE);
    if (todo) {
      blocks.push(createNoteBlock('todo', todo[2] ?? '', (todo[1] ?? '').toLowerCase() === 'x'));
      continue;
    }
    const bullet = line.match(BULLET_LINE);
    if (bullet) {
      blocks.push(createNoteBlock('bullet', bullet[1] ?? ''));
      continue;
    }
    const numbered = line.match(NUMBER_LINE);
    if (numbered) {
      blocks.push(createNoteBlock('number', numbered[1] ?? ''));
      continue;
    }
    blocks.push(createNoteBlock('text', line));
  }
  while (blocks.length > 1) {
    const last = blocks[blocks.length - 1];
    if (!last || !isBlockEmpty(last)) break;
    blocks.pop();
  }
  const only = blocks.length === 1 ? blocks[0] : undefined;
  if (blocks.length === 0 || (only && isBlockEmpty(only))) {
    return [createNoteBlock('bullet')];
  }
  return blocks;
};

const isBlockEmpty = (block: NoteBlock): boolean => block.type === 'text' && block.text.trim() === '';

/** Serializes editor blocks back to the stored markdown-style text. An
 * effectively empty document serializes to '' (which clears the notes). */
export const serializeNotes = (blocks: NoteBlock[]): string => {
  const hasContent = blocks.some((block) => block.text.trim() !== '');
  if (!hasContent) {
    return '';
  }
  return blocks
    .map((block, index) => {
      switch (block.type) {
        case 'bullet':
          return `- ${block.text}`;
        case 'number':
          return `${numberedIndex(blocks, index)}. ${block.text}`;
        case 'todo':
          return `- [${block.checked ? 'x' : ' '}] ${block.text}`;
        default:
          return block.text;
      }
    })
    .join('\n');
};

/** Display number for a numbered block: its 1-based position within the
 * consecutive run of numbered blocks it belongs to. */
export const numberedIndex = (blocks: NoteBlock[], index: number): number => {
  let count = 1;
  for (let i = index - 1; i >= 0 && blocks[i]?.type === 'number'; i--) {
    count++;
  }
  return count;
};
