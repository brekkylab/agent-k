import { describe, it, expect } from 'vitest';
import { buildTranscript } from '@/lib/transcript';
import type { MessageItem } from '@/api/types';

function makeItem(seq: number, role: string, overrides: Partial<MessageItem['message']> = {}): MessageItem {
  return { seq, message: { role, ...overrides } };
}

describe('buildTranscript', () => {
  it('extracts plain text from user message', () => {
    const items: MessageItem[] = [
      makeItem(1, 'user', { contents: [{ type: 'text', text: 'hello world' }] }),
    ];
    const result = buildTranscript(items);
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({ kind: 'user', text: 'hello world', toolCalls: [] });
  });

  it('pairs tool calls by id', () => {
    const items: MessageItem[] = [
      makeItem(1, 'assistant', {
        contents: [{ type: 'text', text: 'calling tool' }],
        tool_calls: [{ id: 'tc-1', function: { name: 'search', arguments: { q: 'test' } } }],
      }),
      makeItem(2, 'tool', { id: 'tc-1', contents: [{ type: 'text', text: 'result text' }] }),
    ];
    const result = buildTranscript(items);
    // tool message is skipped; assistant message has toolCalls with result
    expect(result).toHaveLength(1);
    expect(result[0]!.kind).toBe('assistant');
    expect(result[0]!.toolCalls).toHaveLength(1);
    expect(result[0]!.toolCalls[0]).toMatchObject({
      id: 'tc-1',
      name: 'search',
      result: 'result text',
    });
  });

  it('skips tool messages in output', () => {
    const items: MessageItem[] = [
      makeItem(1, 'tool', { id: 'x', contents: [{ type: 'text', text: 'result' }] }),
    ];
    expect(buildTranscript(items)).toHaveLength(0);
  });

  it('handles empty contents', () => {
    const items: MessageItem[] = [
      makeItem(1, 'assistant', {}),
    ];
    const result = buildTranscript(items);
    expect(result[0]!.text).toBe('');
  });
});
