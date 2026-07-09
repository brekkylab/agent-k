import { useState } from 'react';
import { Link } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { SourceIcon } from '@/workspace/icons';
import { PROVIDERS } from '@/workspace/providers';
import type { SourceProvider } from '@/workspace/types';

interface SourceRailProps {
  activeSourceId: string | null | undefined;
}

// Planned sources shown in the "add source" dialog (static list)
const PLANNED_SOURCES = ['Notion', 'Linear', 'GitHub', 'Figma', 'Dropbox'];

// Category sections in display order
const CATEGORIES: { key: 'files' | 'docs' | 'messages'; labelKey: string }[] = [
  { key: 'files', labelKey: 'workspace.cat.files' },
  { key: 'docs', labelKey: 'workspace.cat.docs' },
  { key: 'messages', labelKey: 'workspace.cat.messages' },
];

function AddSourceDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation('files');
  return (
    <div
      className="cw-dialog-backdrop"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="cw-dialog"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <button className="cw-close" onClick={onClose} aria-label="Close">✕</button>
        <h2 style={{ margin: '0 0 8px', fontSize: 15, fontWeight: 600 }}>
          {t('workspace.addSource')}
        </h2>
        <p style={{ margin: '0 0 16px', fontSize: 13, color: 'var(--cw-fg-3)' }}>
          {t('workspace.addSourceBody')}
        </p>
        <ul className="cw-ws-planned-list">
          {PLANNED_SOURCES.map((name) => (
            <li
              key={name}
              className="cw-ws-planned-item"
            >
              {name}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

export function SourceRail({ activeSourceId }: SourceRailProps) {
  const { t } = useTranslation('files');
  const [dialogOpen, setDialogOpen] = useState(false);

  const providersByCategory = (category: 'files' | 'docs' | 'messages'): SourceProvider[] =>
    PROVIDERS.filter((p) => p.category === category);

  return (
    <>
      {/* "All sources" button — navigates to /workspace (no sourceId) */}
      <Link
        to="/workspace"
        className={`cw-ws-rail-all${activeSourceId == null ? ' is-active' : ''}`}
      >
        {t('workspace.all')}
      </Link>

      {/* Category sections */}
      {CATEGORIES.map(({ key, labelKey }) => {
        const providers = providersByCategory(key);
        if (providers.length === 0) return null;
        return (
          <div key={key}>
            <div className="cw-ws-rail-cat">{t(labelKey)}</div>
            {providers.map((provider) => (
              <Link
                key={provider.id}
                to="/workspace/$sourceId"
                params={{ sourceId: provider.id }}
                className={`cw-ws-rail-row${activeSourceId === provider.id ? ' is-active' : ''}`}
              >
                <SourceIcon sourceId={provider.id} size={18} />
                <span className="cw-ws-rail-row-name">{t(provider.nameKey)}</span>
                <span className="cw-ws-rail-badge">
                  {provider.count != null ? provider.count : '—'}
                </span>
              </Link>
            ))}
          </div>
        );
      })}

      {/* Add source button */}
      <button
        className="cw-ws-rail-add"
        onClick={() => setDialogOpen(true)}
        style={{ marginTop: 'auto' }}
      >
        <span aria-hidden="true">＋</span> {t('workspace.addSource')}
      </button>

      {dialogOpen && <AddSourceDialog onClose={() => setDialogOpen(false)} />}
    </>
  );
}
