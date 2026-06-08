/** @vitest-environment jsdom */
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/react';
import { HtmlView } from '../HtmlView';

afterEach(cleanup);

describe('HtmlView', () => {
  it('renders a sandboxed iframe that allows scripts but NOT same-origin', () => {
    const { container } = render(<HtmlView objectUrl="blob:test" title="page.html" />);
    const iframe = container.querySelector('iframe')!;
    const sandbox = iframe.getAttribute('sandbox') ?? '';
    expect(sandbox.split(/\s+/)).toContain('allow-scripts');
    expect(sandbox).not.toContain('allow-same-origin');
    expect(iframe.getAttribute('src')).toBe('blob:test');
  });
});
