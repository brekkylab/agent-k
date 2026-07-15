// S3 mount registration form, surfaced in the source-connect dialog. Creates a
// real workspace VFS mount (POST /workspaces/{wid}/mounts) from S3 credentials;
// the secret access key is write-only (the backend never echoes it back, so
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
  return slug || 's3';
}

// Append -2, -3, … until `base` no longer collides with a bare prefix already
// in use, so a second connection's default never 409s.
function uniquifyPrefix(base: string, taken: Set<string>): string {
  if (!taken.has(base)) return base;
  let n = 2;
  while (taken.has(`${base}-${n}`)) n += 1;
  return `${base}-${n}`;
}

export function S3MountForm({ onCreated }: { onCreated?: (mount: MountResponse) => void }) {
  const { t } = useTranslation('files');
  const queryClient = useQueryClient();
  const { data: mounts } = useMounts();

  // Bare-prefix compare: the backend normalizes and stores a leading slash
  // ("/s3"), so comparing raw `m.prefix` against a bare candidate never
  // matches and the uniquify would keep re-proposing a colliding default.
  const takenPrefixes = useMemo(
    () => new Set((mounts ?? []).filter((m) => m.provider.type === 's3').map((m) => barePrefix(m.prefix))),
    [mounts],
  );

  const [label, setLabel] = useState('');
  const [prefix, setPrefix] = useState('');
  const [prefixTouched, setPrefixTouched] = useState(false);
  const [bucket, setBucket] = useState('');
  const [region, setRegion] = useState('');
  const [accessKeyId, setAccessKeyId] = useState('');
  const [secretAccessKey, setSecretAccessKey] = useState('');
  const [endpoint, setEndpoint] = useState('');
  const [keyPrefix, setKeyPrefix] = useState('');
  const [error, setError] = useState<string | null>(null);

  // Re-derived on every render from `label`/`takenPrefixes` so it stays fresh
  // once mounts load and as the user types a name; a manual prefix edit wins.
  const defaultPrefix = useMemo(
    () => uniquifyPrefix(label.trim() ? slugify(label) : 's3', takenPrefixes),
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
    onSuccess: async (mount) => {
      await invalidate();
      setSecretAccessKey('');
      onCreated?.(mount);
    },
    onError: (e) => setError(toMessage(e)),
  });

  const canSubmit =
    PREFIX_RE.test(effectivePrefix.trim()) &&
    bucket.trim().length > 0 &&
    accessKeyId.trim().length > 0 &&
    secretAccessKey.trim().length > 0 &&
    !create.isPending;

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
          placeholder="s3"
        />
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
