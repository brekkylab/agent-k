import { describe, it, expect } from 'vitest';
import { deriveStreamState } from './messages';
import type { MessageOutput } from './backend-types';

// Helper: build a minimal MessageOutput with a depth-0 assistant text message
function assistantOutput(text: string, depth = 0): MessageOutput {
  return {
    message: {
      role: 'assistant',
      contents: [{ type: 'text', text }],
    },
    depth,
  };
}

// Helper: assistant message carrying a tool_call (tool_calls is top-level on AiloyMessage)
function toolCallOutput(id: string, name: string): MessageOutput {
  return {
    message: {
      role: 'assistant',
      contents: [],
      tool_calls: [{ id, type: 'function', function: { name, arguments: '{}' } }],
    },
    depth: 0,
  };
}

// Helper: tool result message — msg.id is the tool-call id, contents carry the result text
function toolResultOutput(id: string, result: string): MessageOutput {
  return {
    message: {
      id,
      role: 'tool',
      contents: [{ type: 'text', text: result }],
    },
    depth: 0,
  };
}

describe('deriveStreamState', () => {
  it('returns empty state for empty outputs', () => {
    const state = deriveStreamState([]);
    expect(state.text).toBe('');
    expect(state.toolCalls).toEqual([]);
    expect(state.status).toBe('streaming');
    expect(state.subagentUpdates).toEqual([]);
  });

  it('returns text from a single assistant message', () => {
    const state = deriveStreamState([assistantOutput('Hello world')]);
    expect(state.text).toBe('Hello world');
  });

  it('REPLACES (not appends) accumulated text when two depth-0 assistant messages are present', () => {
    // Documents the intentional snapshot-replace semantics:
    // the backend sends complete text snapshots, not deltas.
    // Two distinct depth-0 assistant messages → only the last text survives.
    const state = deriveStreamState([
      assistantOutput('First message'),
      assistantOutput('Second message'),
    ]);
    expect(state.text).toBe('Second message');
  });

  it('preserves tool call state across outputs', () => {
    const state = deriveStreamState([
      toolCallOutput('tc1', 'search'),
      toolResultOutput('tc1', 'result text'),
    ]);
    expect(state.toolCalls).toHaveLength(1);
    expect(state.toolCalls[0].name).toBe('search');
    expect(state.toolCalls[0].result).toBe('result text');
  });

  it('accumulates subagent text (depth > 0) across outputs from the same source_agent', () => {
    const sub1: MessageOutput = {
      message: { role: 'assistant', contents: [{ type: 'text', text: 'sub ' }] },
      depth: 1,
      source_agent: 'sub-agent-1',
    };
    const sub2: MessageOutput = {
      message: { role: 'assistant', contents: [{ type: 'text', text: 'text' }] },
      depth: 1,
      source_agent: 'sub-agent-1',
    };
    const state = deriveStreamState([sub1, sub2]);
    expect(state.subagentUpdates).toHaveLength(1);
    expect(state.subagentUpdates[0].text).toBe('sub text');
  });

  it('accepts explicit status override', () => {
    const state = deriveStreamState([assistantOutput('done')], 'done');
    expect(state.status).toBe('done');
  });

  it('creates a stub tool call when a tool result arrives without a matching tool_call', () => {
    // The backend may send a tool result before the tool_call (or without one).
    // deriveStreamState should create a stub entry with name "(pending)".
    const state = deriveStreamState([toolResultOutput('orphan-id', 'orphan result')]);
    expect(state.toolCalls).toHaveLength(1);
    expect(state.toolCalls[0].id).toBe('orphan-id');
    expect(state.toolCalls[0].name).toBe('(pending)');
    expect(state.toolCalls[0].result).toBe('orphan result');
  });

  it('skips depth-1 assistant messages without a source_agent for subagentUpdates', () => {
    const depthOneNoAgent: MessageOutput = {
      message: { role: 'assistant', contents: [{ type: 'text', text: 'ignored' }] },
      depth: 1,
      source_agent: null,
    };
    const state = deriveStreamState([depthOneNoAgent]);
    expect(state.subagentUpdates).toEqual([]);
    // depth >= 1 items are skipped for the main text accumulator as well
    expect(state.text).toBe('');
  });

  it('keeps tool call id and arguments from the tool_call output', () => {
    const output: MessageOutput = {
      message: {
        role: 'assistant',
        contents: [],
        tool_calls: [{ id: 'tc2', type: 'function', function: { name: 'read_file', arguments: '{"path":"/tmp/x"}' } }],
      },
      depth: 0,
    };
    const state = deriveStreamState([output]);
    expect(state.toolCalls[0].id).toBe('tc2');
    expect(state.toolCalls[0].arguments).toBe('{"path":"/tmp/x"}');
  });

  it('handles multiple tool calls in a single message', () => {
    const output: MessageOutput = {
      message: {
        role: 'assistant',
        contents: [],
        tool_calls: [
          { id: 'a', type: 'function', function: { name: 'tool_a', arguments: '{}' } },
          { id: 'b', type: 'function', function: { name: 'tool_b', arguments: '{}' } },
        ],
      },
      depth: 0,
    };
    const state = deriveStreamState([output]);
    expect(state.toolCalls).toHaveLength(2);
    expect(state.toolCalls.map((tc) => tc.name)).toEqual(['tool_a', 'tool_b']);
  });

  it('separates subagent updates from two different source_agents', () => {
    const agentA: MessageOutput = {
      message: { role: 'assistant', contents: [{ type: 'text', text: 'from A' }] },
      depth: 1,
      source_agent: 'agent-a',
    };
    const agentB: MessageOutput = {
      message: { role: 'assistant', contents: [{ type: 'text', text: 'from B' }] },
      depth: 1,
      source_agent: 'agent-b',
    };
    const state = deriveStreamState([agentA, agentB]);
    expect(state.subagentUpdates).toHaveLength(2);
    const texts = new Map(state.subagentUpdates.map((u) => [u.sourceAgent, u.text]));
    expect(texts.get('agent-a')).toBe('from A');
    expect(texts.get('agent-b')).toBe('from B');
  });

  it('ignores outputs where message is missing', () => {
    // @ts-expect-error intentionally passing an invalid shape to verify guard
    const state = deriveStreamState([{ depth: 0 }]);
    expect(state.text).toBe('');
    expect(state.toolCalls).toEqual([]);
  });
});
