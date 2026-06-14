import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { parseToolCallValue, classifyFieldValue } from '@/lib/toolCallFormat';
import type { ToolCallInvocation } from '@/domain/types';

/// Renders a parsed tool-call value as React rows — values are shown verbatim (no
/// markdown parsing), mirroring the structure `valueToMarkdown` uses for copy.
function ToolCallValueView({ value }: { value: unknown }) {
  // Memoized so the JSON.parse (in parseToolCallValue) and per-field JSON.stringify
  // (in classifyFieldValue) run once per value, not on every re-render. String values
  // compare by value, so an unchanged result string is a cache hit.
  const view = useMemo(() => {
    const parsed = parseToolCallValue(value);
    if (parsed.kind !== 'fields') return parsed;
    return {
      kind: 'fields' as const,
      fields: parsed.fields.map((f) => ({ key: f.key, display: classifyFieldValue(f.value) })),
    };
  }, [value]);

  if (view.kind === 'empty') return <p className="cw-toolcall-empty">(empty)</p>;
  if (view.kind === 'raw') return <pre className="cw-toolcall-raw">{view.text}</pre>;
  return (
    <dl className="cw-toolcall-fields">
      {view.fields.map(({ key, display }) => (
        <div key={key} className={`cw-toolcall-field${display.kind === 'block' ? ' is-block' : ''}`}>
          <dt>{key}</dt>
          <dd>{
            display.kind === 'block' ? <pre className="cw-toolcall-raw">{display.text}</pre>
            : display.kind === 'code' ? <pre className="cw-toolcall-inline">{display.text}</pre>
            : display.text
          }</dd>
        </div>
      ))}
    </dl>
  );
}

/// A collapsible tool-call. Inputs/results (and their parsing) are rendered lazily —
/// only while expanded — so collapsed tool calls cost nothing to parse.
export function ToolCallDetails({ tc, isStreaming }: { tc: ToolCallInvocation; isStreaming: boolean }) {
  const { t } = useTranslation('session');
  const [open, setOpen] = useState(false);
  return (
    <details
      className="cw-toolcall"
      onToggle={(e) => setOpen((e.currentTarget as HTMLDetailsElement).open)}
    >
      <summary>🔧 {tc.name}{tc.result === undefined && isStreaming ? ` · ${t('ui.tool_running')}` : ''}</summary>
      {open && (
        <>
          {tc.arguments !== undefined && (
            <div className="cw-toolcall-section">
              <span className="cw-toolcall-section-label">Inputs</span>
              <ToolCallValueView value={tc.arguments} />
            </div>
          )}
          {tc.result !== undefined && (
            <div className="cw-toolcall-section">
              <span className="cw-toolcall-section-label">Results</span>
              <ToolCallValueView value={tc.result} />
            </div>
          )}
        </>
      )}
    </details>
  );
}
