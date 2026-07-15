// Notion mount registration form, surfaced in the source-connect dialog.
// Creates a real workspace VFS mount (POST /workspaces/{wid}/mounts) from an
// integration token; the token is write-only (the backend never returns it, so
// there is nothing to prefill — replacing it means disconnect + reconnect).

import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { createMount, deleteMount, type MountResponse } from '@/api/mounts';
import { ApiError } from '@/api/client';

// A mount prefix must be a single top-level segment (the backend normalizes and
// rejects nested/empty ones).
const PREFIX_RE = /^[^/]+$/;

export function NotionMountForm({
  existingMount,
  onConnected,
}: {
  existingMount?: MountResponse;
  onConnected?: () => void;
}) {
  const { t } = useTranslation('files');
  const queryClient = useQueryClient();
  const [prefix, setPrefix] = useState('notion');
  const [apiKey, setApiKey] = useState('');
  const [error, setError] = useState<string | null>(null);

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ['workspace', 'mounts'] });
  const toMessage = (e: unknown) => (e instanceof ApiError ? e.message : String(e));

  const create = useMutation({
    mutationFn: () =>
      createMount({
        prefix: prefix.trim(),
        provider: { type: 'notion', api_key: apiKey.trim() },
      }),
    onSuccess: async () => {
      await invalidate();
      setApiKey('');
      onConnected?.();
    },
    onError: (e) => setError(toMessage(e)),
  });

  const remove = useMutation({
    mutationFn: (id: string) => deleteMount(id),
    onSuccess: () => invalidate(),
    onError: (e) => setError(toMessage(e)),
  });

  if (existingMount) {
    return (
      <div className="cw-ws-add-fields">
        <p style={{ fontSize: 13, color: 'var(--cw-fg-2)' }}>
          {t('workspace.connect.notionConnectedAt', 'Connected — mounted at')} <code>/{existingMount.prefix.replace(/^\/+/, '')}</code>
        </p>
        {error && <small className="is-blocked">{error}</small>}
        <button
          type="button"
          className="cw-btn"
          disabled={remove.isPending}
          onClick={() => {
            setError(null);
            remove.mutate(existingMount.id);
          }}
        >
          {remove.isPending
            ? t('workspace.connect.disconnecting', 'Disconnecting…')
            : t('workspace.connect.disconnect', 'Disconnect')}
        </button>
      </div>
    );
  }

  const canSubmit = PREFIX_RE.test(prefix.trim()) && apiKey.trim().length > 0 && !create.isPending;

  return (
    <div className="cw-ws-add-fields">
      <label className="cw-field">
        <span>{t('workspace.connect.fields.mountPrefix', 'Mount name')}</span>
        <input
          className="cw-input"
          value={prefix}
          onChange={(e) => setPrefix(e.target.value)}
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
