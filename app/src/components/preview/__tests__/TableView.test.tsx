/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { TableView } from '../TableView';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (k: string) => k }),
}));

afterEach(cleanup);

describe('TableView', () => {
  it('parses quoted commas and escaped quotes into single cells', () => {
    const csv = 'name,note\n"Lee, J","say ""hi"""\nKim,plain';
    render(<TableView content={csv} />);
    expect(screen.getByText('name')).toBeTruthy();
    expect(screen.getByText('note')).toBeTruthy();
    expect(screen.getByText('Lee, J')).toBeTruthy();   // quoted comma stays one cell
    expect(screen.getByText('say "hi"')).toBeTruthy(); // "" -> "
    expect(screen.getByText('Kim')).toBeTruthy();
  });

  it('honors a tab delimiter for tsv', () => {
    render(<TableView content={'a\tb\n1\t2'} delimiter={'\t'} />);
    expect(screen.getByText('a')).toBeTruthy();
    expect(screen.getByText('2')).toBeTruthy();
  });

  it('shows an empty state for blank content', () => {
    render(<TableView content="" />);
    expect(screen.getByText('preview.table_empty')).toBeTruthy();
  });
});
