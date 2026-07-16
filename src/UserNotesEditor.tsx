import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
  createNoteBlock,
  type NoteBlock,
  type NoteBlockType,
  numberedIndex,
  parseNotes,
  serializeNotes,
} from './user-notes';

const AUTOSAVE_DELAY_MS = 800;
const SAVED_FLASH_MS = 2000;

type SaveState = 'idle' | 'saving' | 'saved' | 'error';

/**
 * The personal notes editor on a meeting's "Notes" tab: a small block-based
 * editor (plain lines, bullets, numbered items, checkboxes) that autosaves
 * as markdown-style text. Mount it with a `key` of the meeting id so
 * switching meetings resets the local draft.
 */
export default function UserNotesEditor({
  initialContent,
  onSave,
}: {
  initialContent: string | null;
  onSave: (content: string) => Promise<void>;
}) {
  const [blocks, setBlocks] = useState<NoteBlock[]>(() => parseNotes(initialContent));
  const [saveState, setSaveState] = useState<SaveState>('idle');
  const [activeId, setActiveId] = useState<number | null>(null);
  const inputRefs = useRef(new Map<number, HTMLTextAreaElement>());
  const pendingFocusRef = useRef<{ id: number; caret: number } | null>(null);
  const lastSavedRef = useRef(serializeNotes(parseNotes(initialContent)));

  const serialized = useMemo(() => serializeNotes(blocks), [blocks]);
  const serializedRef = useRef(serialized);
  serializedRef.current = serialized;
  const onSaveRef = useRef(onSave);
  onSaveRef.current = onSave;

  const persist = useCallback(async (content: string) => {
    setSaveState('saving');
    try {
      await onSaveRef.current(content);
      lastSavedRef.current = content;
      setSaveState('saved');
      window.setTimeout(() => setSaveState((current) => (current === 'saved' ? 'idle' : current)), SAVED_FLASH_MS);
    } catch {
      setSaveState('error');
    }
  }, []);

  useEffect(() => {
    if (serialized === lastSavedRef.current) return;
    const timeoutId = window.setTimeout(() => void persist(serialized), AUTOSAVE_DELAY_MS);
    return () => window.clearTimeout(timeoutId);
  }, [serialized, persist]);

  // Flush an unsaved draft when the editor unmounts (switching tabs or
  // meetings) so nothing typed is ever silently dropped.
  useEffect(
    () => () => {
      if (serializedRef.current !== lastSavedRef.current) {
        void onSaveRef.current(serializedRef.current).catch(() => {});
      }
    },
    [],
  );

  useLayoutEffect(() => {
    const pending = pendingFocusRef.current;
    if (!pending) return;
    pendingFocusRef.current = null;
    const input = inputRefs.current.get(pending.id);
    if (input) {
      input.focus();
      input.setSelectionRange(pending.caret, pending.caret);
    }
  });

  const updateBlock = useCallback((id: number, patch: Partial<NoteBlock>) => {
    setBlocks((current) => current.map((block) => (block.id === id ? { ...block, ...patch } : block)));
  }, []);

  const handleTextChange = useCallback((block: NoteBlock, value: string) => {
    // Markdown-style shortcuts: typing a list marker at the start of a plain
    // line converts it, matching what people try out of habit.
    if (block.type === 'text') {
      const todo = value.match(/^(?:- )?\[( |x|X)?\] $/);
      if (todo) {
        setBlocks((current) =>
          current.map((entry) =>
            entry.id === block.id
              ? { ...entry, type: 'todo' as const, text: '', checked: (todo[1] ?? '').toLowerCase() === 'x' }
              : entry,
          ),
        );
        pendingFocusRef.current = { id: block.id, caret: 0 };
        return;
      }
      const marker = value.match(/^([-*]|\d+\.) $/);
      if (marker) {
        const type: NoteBlockType = marker[1] === '-' || marker[1] === '*' ? 'bullet' : 'number';
        setBlocks((current) => current.map((entry) => (entry.id === block.id ? { ...entry, type, text: '' } : entry)));
        pendingFocusRef.current = { id: block.id, caret: 0 };
        return;
      }
    }
    setBlocks((current) => current.map((entry) => (entry.id === block.id ? { ...entry, text: value } : entry)));
  }, []);

  // Stable ref-backed lookups so handleKeyDown doesn't rebind on every keystroke.
  const blocksRef = useRef(blocks);
  blocksRef.current = blocks;
  const blocksRefLookup = useCallback((index: number): NoteBlock | null => blocksRef.current[index] ?? null, []);
  const blocksLengthLookup = useCallback((): number => blocksRef.current.length, []);

  const focusBlock = useCallback((block: NoteBlock | null) => {
    if (!block) return;
    const input = inputRefs.current.get(block.id);
    if (input) {
      input.focus();
      input.setSelectionRange(block.text.length, block.text.length);
    }
  }, []);

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>, block: NoteBlock, index: number) => {
      const input = event.currentTarget;
      if (event.key === 'Enter') {
        event.preventDefault();
        if (block.type !== 'text' && block.text === '') {
          // Enter on an empty list item exits the list.
          updateBlock(block.id, { type: 'text' });
          pendingFocusRef.current = { id: block.id, caret: 0 };
          return;
        }
        const caret = input.selectionStart ?? block.text.length;
        const before = block.text.slice(0, caret);
        const after = block.text.slice(caret);
        const next = createNoteBlock(block.type, after);
        setBlocks((current) => {
          const copy = current.map((entry) => (entry.id === block.id ? { ...entry, text: before } : entry));
          copy.splice(index + 1, 0, next);
          return copy;
        });
        pendingFocusRef.current = { id: next.id, caret: 0 };
        return;
      }
      if (event.key === 'Backspace' && input.selectionStart === 0 && input.selectionEnd === 0) {
        if (block.type !== 'text') {
          // First Backspace at the start of a list item removes the marker.
          event.preventDefault();
          updateBlock(block.id, { type: 'text' });
          pendingFocusRef.current = { id: block.id, caret: 0 };
          return;
        }
        const previous = index > 0 ? blocksRefLookup(index - 1) : null;
        if (previous) {
          event.preventDefault();
          const caret = previous.text.length;
          setBlocks((current) => {
            const merged = current.map((entry) =>
              entry.id === previous.id ? { ...entry, text: entry.text + block.text } : entry,
            );
            return merged.filter((entry) => entry.id !== block.id);
          });
          pendingFocusRef.current = { id: previous.id, caret };
        }
        return;
      }
      if (event.key === 'ArrowUp' && index > 0) {
        event.preventDefault();
        focusBlock(blocksRefLookup(index - 1));
        return;
      }
      if (event.key === 'ArrowDown' && index < blocksLengthLookup() - 1) {
        event.preventDefault();
        focusBlock(blocksRefLookup(index + 1));
      }
    },
    [updateBlock, blocksRefLookup, blocksLengthLookup, focusBlock],
  );

  const setActiveType = useCallback(
    (type: NoteBlockType) => {
      const targetId = activeId ?? blocksRef.current[blocksRef.current.length - 1]?.id;
      if (targetId === undefined) return;
      const target = blocksRef.current.find((entry) => entry.id === targetId);
      if (!target) return;
      const nextType = target.type === type ? 'text' : type;
      updateBlock(targetId, { type: nextType, checked: false });
      const input = inputRefs.current.get(targetId);
      pendingFocusRef.current = { id: targetId, caret: input?.selectionStart ?? target.text.length };
    },
    [activeId, updateBlock],
  );

  const activeType = blocks.find((entry) => entry.id === activeId)?.type ?? null;

  return (
    <div className="notes-editor">
      <div className="notes-toolbar">
        <ToolbarButton
          label="Bulleted list"
          active={activeType === 'bullet'}
          onClick={() => setActiveType('bullet')}
          icon="M5 6.5h.01M5 12h.01M5 17.5h.01M9.5 6.5H19M9.5 12H19M9.5 17.5H19"
        />
        <ToolbarButton
          label="Numbered list"
          active={activeType === 'number'}
          onClick={() => setActiveType('number')}
          icon="M4 5.5h2v4M4 9.5h4M10 6.5h9M10 12h9M10 17.5h9M4 14h3l-3 3.5h3"
        />
        <ToolbarButton
          label="Checklist"
          active={activeType === 'todo'}
          onClick={() => setActiveType('todo')}
          icon="M3.5 6l1.5 1.5L8 4.5M3.5 12.5l1.5 1.5L8 11M11 6.5h8M11 13h8M3.5 18.5h15.5"
        />
        <span className={`notes-save-state${saveState === 'error' ? ' is-error' : ''}`} role="status">
          {saveState === 'saving'
            ? 'Saving…'
            : saveState === 'saved'
              ? 'Saved'
              : saveState === 'error'
                ? "Couldn't save"
                : ''}
        </span>
      </div>

      <div className="notes-blocks">
        {blocks.map((block, index) => (
          <NoteBlockRow
            key={block.id}
            block={block}
            index={index}
            blocks={blocks}
            placeholder={blocks.length === 1 && block.text === '' ? 'Write a note for this meeting…' : undefined}
            registerInput={(id, element) => {
              if (element) inputRefs.current.set(id, element);
              else inputRefs.current.delete(id);
            }}
            onFocusBlock={setActiveId}
            onTextChange={handleTextChange}
            onKeyDown={handleKeyDown}
            onToggleChecked={() => updateBlock(block.id, { checked: !block.checked })}
          />
        ))}
      </div>
    </div>
  );
}

function ToolbarButton({
  label,
  active,
  onClick,
  icon,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  icon: string;
}) {
  return (
    <button
      type="button"
      className={`notes-tool${active ? ' is-active' : ''}`}
      aria-label={label}
      aria-pressed={active}
      title={label}
      onMouseDown={(event) => event.preventDefault()}
      onClick={onClick}
    >
      <svg
        width={15}
        height={15}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.8}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <path d={icon} />
      </svg>
    </button>
  );
}

function NoteBlockRow({
  block,
  index,
  blocks,
  placeholder,
  registerInput,
  onFocusBlock,
  onTextChange,
  onKeyDown,
  onToggleChecked,
}: {
  block: NoteBlock;
  index: number;
  blocks: NoteBlock[];
  placeholder?: string;
  registerInput: (id: number, element: HTMLTextAreaElement | null) => void;
  onFocusBlock: (id: number) => void;
  onTextChange: (block: NoteBlock, value: string) => void;
  onKeyDown: (event: React.KeyboardEvent<HTMLTextAreaElement>, block: NoteBlock, index: number) => void;
  onToggleChecked: () => void;
}) {
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  const resize = useCallback(() => {
    const input = inputRef.current;
    if (!input) return;
    input.style.height = 'auto';
    input.style.height = `${input.scrollHeight}px`;
  }, []);

  // Re-measure whenever the text changes programmatically too (splits,
  // merges, marker conversions), not just on direct typing.
  // biome-ignore lint/correctness/useExhaustiveDependencies: block.text drives the height
  useLayoutEffect(() => {
    resize();
  }, [resize, block.text]);

  return (
    <div className={`note-block is-${block.type}${block.type === 'todo' && block.checked ? ' is-checked' : ''}`}>
      {block.type === 'bullet' && (
        <span className="note-marker" aria-hidden="true">
          •
        </span>
      )}
      {block.type === 'number' && (
        <span className="note-marker" aria-hidden="true">
          {numberedIndex(blocks, index)}.
        </span>
      )}
      {block.type === 'todo' && (
        <input
          type="checkbox"
          className="note-check"
          checked={block.checked}
          aria-label={block.checked ? 'Mark as not done' : 'Mark as done'}
          onChange={onToggleChecked}
        />
      )}
      <textarea
        ref={(element) => {
          inputRef.current = element;
          registerInput(block.id, element);
        }}
        className="note-input"
        rows={1}
        value={block.text}
        placeholder={placeholder}
        aria-label="Note line"
        onFocus={() => onFocusBlock(block.id)}
        onChange={(event) => onTextChange(block, event.target.value)}
        onInput={resize}
        onKeyDown={(event) => onKeyDown(event, block, index)}
      />
    </div>
  );
}
