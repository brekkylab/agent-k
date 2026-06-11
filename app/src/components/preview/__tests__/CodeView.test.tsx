/** @vitest-environment jsdom */
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup } from '@testing-library/react';
import { fenceFor } from '../CodeView';

afterEach(cleanup);

describe('fenceFor', () => {
  it('uses 3 backticks for plain code', () => {
    expect(fenceFor('const a = 1;')).toBe('```');
  });
  it('grows the fence to exceed the longest backtick run in content', () => {
    expect(fenceFor('text ``` more').length).toBeGreaterThanOrEqual(4);
    expect(fenceFor('a ```` b').length).toBeGreaterThanOrEqual(5);
  });
});
