// Notion mount registration form, surfaced in the source-connect dialog.
// Creates a real workspace VFS mount (POST /workspaces/{wid}/mounts) from an
// integration token; the token is write-only (the backend never returns it, so
// there is nothing to prefill — replacing it means disconnect + reconnect).
// Disconnect lives in the dialog's existing-connections list, not in this form
// — this form is always a fresh "add another connection" flow.

import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { createMount, type MountResponse } from '@/api/mounts';
import { ApiError } from '@/api/client';
import { useMounts } from '@/workspace-connections/hooks/useProviders';
import { barePrefix } from '../providers/s3';

// A mount prefix must be a single top-level segment (the backend normalizes and
// rejects nested/empty ones).
const PREFIX_RE = /^[^/]+$/;

function slugify(label: string): string {
  const slug = label
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slug || 'notion';
}

// Append -2, -3, … until `base` no longer collides with a bare prefix already
// in use, so a second connection's default never 409s.
function uniquifyPrefix(base: string, taken: Set<string>): string {
  if (!taken.has(base)) return base;
  let n = 2;
  while (taken.has(`${base}-${n}`)) n += 1;
  return `${base}-${n}`;
}

export function NotionMountForm({ onCreated }: { onCreated?: (mount: MountResponse) => void }) {
  const { t } = useTranslation('files');
  const queryClient = useQueryClient();
  const { data: mounts } = useMounts();

  // Bare-prefix compare: the backend normalizes and stores a leading slash
  // ("/notion"), so comparing raw `m.prefix` against a bare candidate never
  // matches and the uniquify would keep re-proposing a colliding default.
  const takenPrefixes = useMemo(
    () => new Set((mounts ?? []).filter((m) => m.provider.type === 'notion').map((m) => barePrefix(m.prefix))),
    [mounts],
  );

  const [label, setLabel] = useState('');
  const [prefix, setPrefix] = useState('');
  const [prefixTouched, setPrefixTouched] = useState(false);
  const [apiKey, setApiKey] = useState('');
  const [error, setError] = useState<string | null>(null);

  // Re-derived on every render from `label`/`takenPrefixes` so it stays fresh
  // once mounts load and as the user types a name; a manual prefix edit wins.
  const defaultPrefix = useMemo(
    () => uniquifyPrefix(label.trim() ? slugify(label) : 'notion', takenPrefixes),
    [label, takenPrefixes],
  );
  const effectivePrefix = prefixTouched ? prefix : defaultPrefix;

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ['workspace', 'mounts'] });

  function toMessage(e: unknown): string {
    if (e instanceof ApiError) {
      // Map by HTTP status, not by string-matching the raw SQL error text.
      if (e.status === 409) return t('workspace.connect.prefixTaken', '그 이름의 연결이 이미 있어요');
      return e.message;
    }
    return String(e);
  }

  const create = useMutation({
    mutationFn: () =>
      createMount({
        prefix: effectivePrefix.trim(),
        label: label.trim() || undefined,
        provider: { type: 'notion', api_key: apiKey.trim() },
      }),
    onSuccess: async (mount) => {
      await invalidate();
      setApiKey('');
      onCreated?.(mount);
    },
    onError: (e) => setError(toMessage(e)),
  });

  const canSubmit = PREFIX_RE.test(effectivePrefix.trim()) && apiKey.trim().length > 0 && !create.isPending;

  return (
    <div className="cw-ws-add-fields">
      <label className="cw-field">
        <span>{t('workspace.connect.fields.label')}</span>
        <input
          className="cw-input"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder={t('workspace.connect.placeholders.label')}
        />
      </label>
      <label className="cw-field">
        <span>{t('workspace.connect.fields.mountPrefix', 'Mount name')}</span>
        <input
          className="cw-input"
          value={effectivePrefix}
          onChange={(e) => {
            setPrefixTouched(true);
            setPrefix(e.target.value);
          }}
          placeholder="notion"
        />
      </label>
      <label className="cw-field">
        <span>{t('workspace.connect.fields.notionToken', 'Notion integration token')}</span>
        <input
          className="cw-input"
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder="ntn_…"
          autoComplete="off"
        />
      </label>
      <p style={{ fontSize: 12, color: 'var(--cw-fg-3)', lineHeight: 1.5 }}>
        {t(
          'workspace.connect.notionShareHint',
          'Create an internal integration in Notion, then share each page or database you want to browse with that integration — otherwise it sees nothing.',
        )}
      </p>
      {error && <small className="is-blocked">{error}</small>}
      <button
        type="button"
        className="cw-btn-primary"
        disabled={!canSubmit}
        onClick={() => {
          setError(null);
          create.mutate();
        }}
      >
        {create.isPending
          ? t('workspace.connect.connecting', 'Connecting…')
          : t('workspace.connect.notionConnect', 'Connect Notion')}
      </button>
    </div>
  );
}
