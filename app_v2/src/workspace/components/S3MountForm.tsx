// S3 mount registration form, surfaced in the source-connect dialog. Creates a
// real workspace VFS mount (POST /workspaces/{wid}/mounts) from S3 credentials;
// the secret access key is write-only (the backend never echoes it back, so
// there is nothing to prefill — replacing it means disconnect + reconnect).

import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { createMount, deleteMount, type MountResponse } from '@/api/mounts';
import { ApiError } from '@/api/client';

// A mount prefix must be a single top-level segment (the backend normalizes and
// rejects nested/empty ones).
const PREFIX_RE = /^[^/]+$/;

export function S3MountForm({
  existingMount,
  onConnected,
}: {
  existingMount?: MountResponse;
  onConnected?: () => void;
}) {
  const { t } = useTranslation('files');
  const queryClient = useQueryClient();
  const [prefix, setPrefix] = useState('s3');
  const [bucket, setBucket] = useState('');
  const [region, setRegion] = useState('');
  const [accessKeyId, setAccessKeyId] = useState('');
  const [secretAccessKey, setSecretAccessKey] = useState('');
  const [endpoint, setEndpoint] = useState('');
  const [keyPrefix, setKeyPrefix] = useState('');
  const [error, setError] = useState<string | null>(null);

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ['workspace', 'mounts'] });
  const toMessage = (e: unknown) => (e instanceof ApiError ? e.message : String(e));

  const create = useMutation({
    mutationFn: () =>
      createMount({
        prefix: prefix.trim(),
        provider: {
          type: 's3',
          bucket: bucket.trim(),
          // Optional fields: omit when blank so the backend applies its defaults
          // (region → us-east-1, endpoint → real AWS, no key_prefix scoping).
          ...(region.trim() ? { region: region.trim() } : {}),
          access_key_id: accessKeyId.trim(),
          secret_access_key: secretAccessKey.trim(),
          ...(endpoint.trim() ? { endpoint: endpoint.trim() } : {}),
          ...(keyPrefix.trim() ? { key_prefix: keyPrefix.trim() } : {}),
        },
      }),
    onSuccess: async () => {
      await invalidate();
      setSecretAccessKey('');
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
    const info = existingMount.provider;
    return (
      <div className="cw-ws-add-fields">
        <p style={{ fontSize: 13, color: 'var(--cw-fg-2)' }}>
          {t('workspace.connect.s3ConnectedAt', 'Connected — mounted at')}{' '}
          <code>/{existingMount.prefix.replace(/^\/+/, '')}</code>
          {info.type === 's3' && (
            <>
              {' '}
              <span style={{ color: 'var(--cw-fg-3)' }}>({info.bucket})</span>
            </>
          )}
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

  const canSubmit =
    PREFIX_RE.test(prefix.trim()) &&
    bucket.trim().length > 0 &&
    accessKeyId.trim().length > 0 &&
    secretAccessKey.trim().length > 0 &&
    !create.isPending;

  return (
    <div className="cw-ws-add-fields">
      <label className="cw-field">
        <span>{t('workspace.connect.fields.mountPrefix', 'Mount name')}</span>
        <input className="cw-input" value={prefix} onChange={(e) => setPrefix(e.target.value)} placeholder="s3" />
      </label>
      <label className="cw-field">
        <span>{t('workspace.connect.fields.bucket', 'Bucket')}</span>
        <input className="cw-input" value={bucket} onChange={(e) => setBucket(e.target.value)} placeholder="my-bucket" />
      </label>
      <label className="cw-field">
        <span>{t('workspace.connect.fields.region', 'Region')}</span>
        <input className="cw-input" value={region} onChange={(e) => setRegion(e.target.value)} placeholder="us-east-1" />
      </label>
      <label className="cw-field">
        <span>{t('workspace.connect.fields.accessKeyId', 'Access key ID')}</span>
        <input
          className="cw-input"
          value={accessKeyId}
          onChange={(e) => setAccessKeyId(e.target.value)}
          placeholder="AKIA…"
          autoComplete="off"
        />
      </label>
      <label className="cw-field">
        <span>{t('workspace.connect.fields.secretAccessKey', 'Secret access key')}</span>
        <input
          className="cw-input"
          type="password"
          value={secretAccessKey}
          onChange={(e) => setSecretAccessKey(e.target.value)}
          placeholder="••••••••"
          autoComplete="off"
        />
      </label>
      <label className="cw-field">
        <span>{t('workspace.connect.fields.endpoint', 'Endpoint (S3-compatible, optional)')}</span>
        <input
          className="cw-input"
          value={endpoint}
          onChange={(e) => setEndpoint(e.target.value)}
          placeholder="https://s3.us-east-1.amazonaws.com"
        />
      </label>
      <label className="cw-field">
        <span>{t('workspace.connect.fields.keyPrefix', 'Key prefix (optional)')}</span>
        <input
          className="cw-input"
          value={keyPrefix}
          onChange={(e) => setKeyPrefix(e.target.value)}
          placeholder="reports/"
        />
      </label>
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
          : t('workspace.connect.s3Connect', 'Connect S3')}
      </button>
    </div>
  );
}
