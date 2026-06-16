/**
 * @vitest-environment jsdom
 *
 * Behavior tests for the composer command framework: the useComposerCommands
 * hook + CommandSuggestionPopup, driven through a harness textarea the same
 * way the session route wires them. Pixel placement (the popup opens above
 * the composer box) needs real layout and is covered by Playwright instead.
 */
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useRef, useState } from 'react';

import { CommandSuggestionPopup } from './CommandSuggestionPopup';
import { useComposerCommands } from './useComposerCommands';
import type { CommandItem, ComposerCommand } from './types';

beforeAll(() => {
  // jsdom has no layout; the popup scrolls the highlighted row into view.
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(cleanup);

const FILES: CommandItem[] = [
  { id: 'projects/p/shared/report.pdf', label: 'report.pdf', sublabel: 'docs' },
  { id: 'projects/p/shared/data.csv', label: 'data.csv' },
];

function makeFileCommand(onPick: (id: string) => void): ComposerCommand {
  return {
    trigger: '#',
    triggerPosition: 'word-boundary',
    emptyLabel: 'no files',
    getItems: (q) => FILES.filter((f) => f.label.toLowerCase().includes(q.toLowerCase())),
    onSelect: (item) => {
      onPick(item.id);
      return { replaceWith: '' };
    },
  };
}

function makeMentionCommand(): ComposerCommand {
  return {
    trigger: '@',
    triggerPosition: 'word-boundary',
    emptyLabel: 'no members',
    getItems: () => [{ id: 'u1', label: 'Jane', sublabel: '@jane' }],
    onSelect: () => ({ replaceWith: '@jane ' }),
  };
}

function Harness({
  commands,
  disabled = false,
  onSend,
}: {
  commands: ComposerCommand[];
  disabled?: boolean;
  onSend?: () => void;
}) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [value, setValue] = useState('');
  const cmd = useComposerCommands({ textareaRef, value, onChangeValue: setValue, commands, disabled });
  return (
    <div>
      {cmd.open && (
        <CommandSuggestionPopup
          items={cmd.items}
          highlightIndex={cmd.highlightIndex}
          listboxId={cmd.listboxId}
          isLoading={cmd.activeCommand?.isLoading ?? false}
          emptyLabel={cmd.activeCommand?.emptyLabel ?? ''}
          loadingLabel="loading"
          onHighlight={cmd.setHighlightIndex}
          onSelect={cmd.selectItem}
        />
      )}
      <textarea
        aria-label="composer"
        ref={textareaRef}
        value={value}
        onChange={(e) => {
          setValue(e.target.value);
          cmd.handleSelectionChange();
        }}
        onKeyDown={(e) => {
          if (cmd.handleKeyDown(e)) return;
          if (e.key === 'Enter' && !e.shiftKey) onSend?.();
        }}
        onSelect={cmd.handleSelectionChange}
        onCompositionStart={cmd.handleCompositionStart}
        onCompositionEnd={cmd.handleCompositionEnd}
      />
    </div>
  );
}

function composer(): HTMLTextAreaElement {
  return screen.getByLabelText('composer');
}

// Type into the textarea with the caret at the end, mirroring real input.
function type(value: string, caret = value.length) {
  const el = composer();
  fireEvent.change(el, { target: { value } });
  el.setSelectionRange(caret, caret);
  fireEvent.select(el);
}

describe('useComposerCommands + CommandSuggestionPopup', () => {
  it('opens with the full list on a bare trigger and filters by query', () => {
    render(<Harness commands={[makeFileCommand(() => {})]} />);
    type('#');
    expect(screen.getByRole('listbox')).toBeTruthy();
    expect(screen.getAllByRole('option')).toHaveLength(2);

    type('#rep');
    expect(screen.getAllByRole('option')).toHaveLength(1);
    expect(screen.getByRole('option').textContent).toContain('report.pdf');
  });

  it('does not open mid-word or after a space dismisses the token', () => {
    render(<Harness commands={[makeFileCommand(() => {})]} />);
    type('foo#bar');
    expect(screen.queryByRole('listbox')).toBeNull();
    type('#rep done');
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('navigates with arrows (wrapping) and selects with Enter without sending', () => {
    const onPick = vi.fn();
    const onSend = vi.fn();
    render(<Harness commands={[makeFileCommand(onPick)]} onSend={onSend} />);
    type('#');

    fireEvent.keyDown(composer(), { key: 'ArrowDown' });
    fireEvent.keyDown(composer(), { key: 'ArrowDown' }); // wraps back to 0
    const options = screen.getAllByRole('option');
    expect(options[0].getAttribute('aria-selected')).toBe('true');

    fireEvent.keyDown(composer(), { key: 'Enter' });
    expect(onPick).toHaveBeenCalledWith('projects/p/shared/report.pdf');
    expect(onSend).not.toHaveBeenCalled();
    // '#' token removed (replaceWith: '').
    expect(composer().value).toBe('');
  });

  it('inserts the mention text on selection and restores the caret', () => {
    render(<Harness commands={[makeMentionCommand()]} />);
    type('hi @ja', 6);
    fireEvent.keyDown(composer(), { key: 'Enter' });
    expect(composer().value).toBe('hi @jane ');
  });

  it('Escape dismisses and the same token does not reopen until it changes', () => {
    render(<Harness commands={[makeFileCommand(() => {})]} />);
    type('#');
    fireEvent.keyDown(composer(), { key: 'Escape' });
    expect(screen.queryByRole('listbox')).toBeNull();

    // Still the same token after more typing at the same trigger → stays closed?
    // No: the query changed, so the dismissal is scoped to the exact token
    // start; typing keeps it dismissed only while start+trigger match.
    type('#r');
    expect(screen.queryByRole('listbox')).toBeNull();

    // Leaving the token (space ends it) and starting a new one reopens.
    type('#r x #', 6);
    expect(screen.getByRole('listbox')).toBeTruthy();
  });

  it('ignores keys while an IME composition is in flight', () => {
    const onPick = vi.fn();
    render(<Harness commands={[makeFileCommand(onPick)]} />);
    type('#');
    fireEvent.keyDown(composer(), { key: 'Enter', isComposing: true });
    expect(onPick).not.toHaveBeenCalled();
    expect(screen.getByRole('listbox')).toBeTruthy();
  });

  it('selects on pointer down without losing the token replacement', () => {
    const onPick = vi.fn();
    render(<Harness commands={[makeFileCommand(onPick)]} />);
    type('#data');
    fireEvent.pointerDown(screen.getByRole('option'));
    expect(onPick).toHaveBeenCalledWith('projects/p/shared/data.csv');
    expect(composer().value).toBe('');
  });

  it('shows the empty state when nothing matches', () => {
    render(<Harness commands={[makeFileCommand(() => {})]} />);
    type('#zzz');
    expect(screen.getByRole('listbox').textContent).toContain('no files');
    expect(screen.queryAllByRole('option')).toHaveLength(0);
  });

  it('stays closed while disabled', () => {
    render(<Harness commands={[makeFileCommand(() => {})]} disabled />);
    type('#');
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('routes each trigger to its own command', () => {
    render(<Harness commands={[makeFileCommand(() => {}), makeMentionCommand()]} />);
    type('@');
    expect(screen.getByRole('option').textContent).toContain('Jane');
    type('#');
    expect(screen.getAllByRole('option')).toHaveLength(2);
  });
});
