import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { IconButton } from '@/components/uiPrimitives';
import { parseToolCallValue, classifyFieldValue, toolCallToMarkdown } from '@/lib/toolCallFormat';
import type { ToolCallEntry } from '@/lib/transcript';

function ToolCallValueView({ value }: { value: unknown }) {
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

export function ToolCallDetails({ tc, isStreaming }: { tc: ToolCallEntry; isStreaming: boolean }) {
  const { t } = useTranslation('session');
  const [open, setOpen] = useState(false);
  const hasContent = tc.arguments !== undefined || tc.result !== undefined;
  return (
    <details
      className="cw-toolcall"
      onToggle={(e) => setOpen((e.currentTarget as HTMLDetailsElement).open)}
    >
      <summary>🔧 {tc.name}{tc.result === undefined && isStreaming ? ` · ${t('ui.tool_running')}` : ''}</summary>
      {open && (
        <>
          {hasContent && (
            <IconButton
              className="cw-toolcall-copy"
              icon="copy"
              label="Copy"
              onClick={() => {
                void navigator.clipboard?.writeText(toolCallToMarkdown(tc)).catch(() => {});
              }}
            />
          )}
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
