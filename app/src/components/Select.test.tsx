/**
 * @vitest-environment jsdom
 *
 * Behavior tests for the generic <Select>. jsdom does not lay elements out
 * (getBoundingClientRect returns zeros), so these lock in *behavior* —
 * open/close, commit, keyboard nav, typeahead, grouping, disabled handling,
 * dismissal, value types, a11y wiring. Pixel placement (the body portal's
 * coordinates, viewport/container collision) is verified by Playwright e2e
 * instead, since it needs a real layout.
 */
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { Select, type SelectGroup, type SelectOption } from './Select';

// A controlled wrapper so commits flow back into `value` (mirrors real usage)
// while exposing the committed value via onCommit for assertions.
function ControlledSelect<T>({
  initial,
  onCommit,
  options,
  ariaLabel = 'Picker',
}: {
  initial: T;
  onCommit?: (value: T) => void;
  options: SelectOption<T>[] | SelectGroup<T>[];
  ariaLabel?: string;
}) {
  const [value, setValue] = useState<T>(initial);
  return (
    <Select<T>
      value={value}
      onChange={(next) => {
        setValue(next);
        onCommit?.(next);
      }}
      options={options}
      ariaLabel={ariaLabel}
    />
  );
}

const FRUITS: SelectOption<string>[] = [
  { value: 'apple', label: 'Apple' },
  { value: 'banana', label: 'Banana' },
  { value: 'cherry', label: 'Cherry' },
];

const FRUITS_BANANA_DISABLED: SelectOption<string>[] = [
  { value: 'apple', label: 'Apple' },
  { value: 'banana', label: 'Banana', disabled: true },
  { value: 'cherry', label: 'Cherry' },
];

const GROUPED: SelectGroup<string>[] = [
  { label: 'Claude', options: [{ value: 'opus', label: 'Opus' }, { value: 'haiku', label: 'Haiku' }] },
  { label: 'OpenAI', options: [{ value: 'gpt', label: 'GPT' }] },
];

const NUMBERS: SelectOption<number>[] = [
  { value: 1, label: 'One' },
  { value: 2, label: 'Two' },
];

beforeAll(() => {
  // The panel scrolls the focused option into view on open; jsdom has no layout
  // engine, so stub it to a no-op (otherwise it throws "Not implemented").
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  // Portal mounts into <body>; clean it up so queries don't bleed across tests.
  cleanup();
});

const trigger = (name = 'Picker') => screen.getByRole('button', { name });

describe('Select — rendering & toggle', () => {
  it('shows the selected option label and is closed initially', () => {
    render(<ControlledSelect options={FRUITS} initial="apple" ariaLabel="Fruit" />);
    expect(trigger('Fruit').textContent).toContain('Apple');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(trigger('Fruit').getAttribute('aria-expanded')).toBe('false');
  });

  it('opens on click, listing every option; clicking the trigger again closes', () => {
    render(<ControlledSelect options={FRUITS} initial="apple" ariaLabel="Fruit" />);
    fireEvent.click(trigger('Fruit'));
    expect(screen.getByRole('listbox')).toBeTruthy();
    expect(screen.getAllByRole('option')).toHaveLength(3);
    fireEvent.click(trigger('Fruit'));
    expect(screen.queryByRole('listbox')).toBeNull();
  });
});

describe('Select — selection (mouse)', () => {
  it('commits the clicked option, fires onChange, closes, and updates the trigger', () => {
    const onCommit = vi.fn();
    render(<ControlledSelect options={FRUITS} initial="apple" onCommit={onCommit} ariaLabel="Fruit" />);
    fireEvent.click(trigger('Fruit'));
    fireEvent.click(screen.getByRole('option', { name: 'Cherry' }));
    expect(onCommit).toHaveBeenCalledWith('cherry');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(trigger('Fruit').textContent).toContain('Cherry');
  });

  it('preserves non-string (numeric) values through onChange', () => {
    const onCommit = vi.fn();
    render(<ControlledSelect options={NUMBERS} initial={1} onCommit={onCommit} ariaLabel="Count" />);
    fireEvent.click(trigger('Count'));
    fireEvent.click(screen.getByRole('option', { name: 'Two' }));
    expect(onCommit).toHaveBeenCalledWith(2);
  });
});

describe('Select — keyboard', () => {
  it('opens on ArrowDown, then ArrowDown moves focus and Enter commits', () => {
    const onCommit = vi.fn();
    render(<ControlledSelect options={FRUITS} initial="apple" onCommit={onCommit} ariaLabel="Fruit" />);
    fireEvent.keyDown(trigger('Fruit'), { key: 'ArrowDown' }); // open (focus = selected)
    expect(screen.getByRole('listbox')).toBeTruthy();
    fireEvent.keyDown(trigger('Fruit'), { key: 'ArrowDown' }); // move to Banana
    fireEvent.keyDown(trigger('Fruit'), { key: 'Enter' });
    expect(onCommit).toHaveBeenCalledWith('banana');
  });

  it('typeahead jumps focus to the first match', () => {
    const onCommit = vi.fn();
    render(<ControlledSelect options={FRUITS} initial="apple" onCommit={onCommit} ariaLabel="Fruit" />);
    fireEvent.click(trigger('Fruit'));
    fireEvent.keyDown(trigger('Fruit'), { key: 'c' }); // -> Cherry
    fireEvent.keyDown(trigger('Fruit'), { key: 'Enter' });
    expect(onCommit).toHaveBeenCalledWith('cherry');
  });

  it('Escape closes without committing', () => {
    const onCommit = vi.fn();
    render(<ControlledSelect options={FRUITS} initial="apple" onCommit={onCommit} ariaLabel="Fruit" />);
    fireEvent.click(trigger('Fruit'));
    fireEvent.keyDown(trigger('Fruit'), { key: 'Escape' });
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(onCommit).not.toHaveBeenCalled();
  });
});

describe('Select — disabled options', () => {
  it('does not commit a disabled option on click and stays open', () => {
    const onCommit = vi.fn();
    render(<ControlledSelect options={FRUITS_BANANA_DISABLED} initial="apple" onCommit={onCommit} ariaLabel="Fruit" />);
    fireEvent.click(trigger('Fruit'));
    const banana = screen.getByRole('option', { name: 'Banana' });
    expect(banana.getAttribute('aria-disabled')).toBe('true');
    fireEvent.click(banana);
    expect(onCommit).not.toHaveBeenCalled();
    expect(screen.getByRole('listbox')).toBeTruthy();
  });

  it('arrow navigation skips disabled options', () => {
    const onCommit = vi.fn();
    render(<ControlledSelect options={FRUITS_BANANA_DISABLED} initial="apple" onCommit={onCommit} ariaLabel="Fruit" />);
    fireEvent.keyDown(trigger('Fruit'), { key: 'ArrowDown' }); // open (focus = Apple)
    fireEvent.keyDown(trigger('Fruit'), { key: 'ArrowDown' }); // skip disabled Banana -> Cherry
    fireEvent.keyDown(trigger('Fruit'), { key: 'Enter' });
    expect(onCommit).toHaveBeenCalledWith('cherry');
  });
});

describe('Select — grouping', () => {
  it('renders group headers and marks the panel as grouped', () => {
    render(<ControlledSelect options={GROUPED} initial="opus" ariaLabel="Model" />);
    fireEvent.click(trigger('Model'));
    expect(screen.getByText('Claude')).toBeTruthy();
    expect(screen.getByText('OpenAI')).toBeTruthy();
    expect(screen.getByRole('listbox').className).toContain('is-grouped');
    expect(screen.getAllByRole('option')).toHaveLength(3);
  });
});

describe('Select — dismissal & a11y', () => {
  it('closes on an outside pointerdown', () => {
    render(<ControlledSelect options={FRUITS} initial="apple" ariaLabel="Fruit" />);
    fireEvent.click(trigger('Fruit'));
    expect(screen.getByRole('listbox')).toBeTruthy();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('wires the listbox combobox aria attributes', () => {
    render(<ControlledSelect options={FRUITS} initial="apple" ariaLabel="Fruit" />);
    const btn = trigger('Fruit');
    expect(btn.getAttribute('aria-haspopup')).toBe('listbox');
    expect(btn.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(btn);
    expect(btn.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('option', { name: 'Apple' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('option', { name: 'Banana' }).getAttribute('aria-selected')).toBe('false');
  });
});
