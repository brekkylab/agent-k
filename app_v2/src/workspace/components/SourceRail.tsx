import { useState } from 'react';
import { Link, useNavigate } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { SourceIcon } from '@/workspace/icons';
import { PROVIDERS } from '@/workspace/providers';
import type { SourceCategory, SourceProvider } from '@/workspace/types';

interface SourceRailProps {
  activeSourceId: string | null | undefined;
}

const CONNECTED_STORAGE_KEY = 'cw.workspace.connectedSources';

type SourceCatalogCategory = Exclude<SourceCategory, 'knowledge'>;

// Source sections in display order. Knowledge is rendered as its own layer.
const SOURCE_CATEGORIES: { key: SourceCatalogCategory; labelKey: string }[] = [
  { key: 'files', labelKey: 'workspace.cat.files' },
  { key: 'docs', labelKey: 'workspace.cat.docs' },
  { key: 'messages', labelKey: 'workspace.cat.messages' },
];

const CONNECTION_FIELDS: Partial<Record<SourceProvider['id'], string[]>> = {
  dropbox: ['accountEmail', 'accessToken'],
  figma: ['teamUrl', 'accessToken'],
  github: ['repositoryUrl', 'accessToken'],
  linear: ['workspaceUrl', 'apiKey'],
};

const CONNECTION_GUIDES: Partial<Record<SourceProvider['id'], {
  titleKey: string;
  introKey: string;
  stepKeys: string[];
  permissionKeys: string[];
  noteKey: string;
}>> = {
  github: {
    titleKey: 'workspace.connect.guides.github.title',
    introKey: 'workspace.connect.guides.github.intro',
    stepKeys: [
      'workspace.connect.guides.github.step1',
      'workspace.connect.guides.github.step2',
      'workspace.connect.guides.github.step3',
      'workspace.connect.guides.github.step4',
    ],
    permissionKeys: [
      'workspace.connect.guides.github.scopeContents',
      'workspace.connect.guides.github.scopeIssues',
      'workspace.connect.guides.github.scopePullRequests',
    ],
    noteKey: 'workspace.connect.guides.github.note',
  },
  linear: {
    titleKey: 'workspace.connect.guides.linear.title',
    introKey: 'workspace.connect.guides.linear.intro',
    stepKeys: [
      'workspace.connect.guides.linear.step1',
      'workspace.connect.guides.linear.step2',
      'workspace.connect.guides.linear.step3',
      'workspace.connect.guides.linear.step4',
    ],
    permissionKeys: [
      'workspace.connect.guides.linear.scopeRead',
      'workspace.connect.guides.linear.scopeIssues',
    ],
    noteKey: 'workspace.connect.guides.linear.note',
  },
};

function readConnectedSourceIds(): Set<string> {
  if (typeof window === 'undefined') return new Set();
  try {
    const raw = window.localStorage.getItem(CONNECTED_STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(parsed) ? parsed.filter((id) => typeof id === 'string') : []);
  } catch {
    return new Set();
  }
}

function writeConnectedSourceIds(ids: Set<string>) {
  if (typeof window === 'undefined') return;
  window.localStorage.setItem(CONNECTED_STORAGE_KEY, JSON.stringify([...ids]));
}

function isProviderConnected(provider: SourceProvider, connectedIds: Set<string>): boolean {
  return provider.connected || connectedIds.has(provider.id);
}

function AddSourceDialog({
  connectedIds,
  initialSourceId,
  onClose,
  onConnect,
}: {
  connectedIds: Set<string>;
  initialSourceId: string | null;
  onClose: () => void;
  onConnect: (provider: SourceProvider) => void;
}) {
  const { t } = useTranslation('files');
  const firstUnconnected = PROVIDERS.find((provider) => !isProviderConnected(provider, connectedIds));
  const initialProvider =
    PROVIDERS.find((provider) => provider.id === initialSourceId) ?? firstUnconnected ?? PROVIDERS[0]!;
  const [selectedId, setSelectedId] = useState<SourceProvider['id']>(initialProvider.id);
  const [detailMode, setDetailMode] = useState<'connect' | 'guide'>('connect');
  const selected = PROVIDERS.find((provider) => provider.id === selectedId) ?? initialProvider;
  const selectedConnected = isProviderConnected(selected, connectedIds);
  const requiredFields = selectedConnected ? [] : CONNECTION_FIELDS[selected.id] ?? ['workspaceUrl', 'apiKey'];
  const selectedGuide = selectedConnected ? undefined : CONNECTION_GUIDES[selected.id];
  const mode = selectedGuide ? detailMode : 'connect';

  function handleConnect() {
    onConnect(selected);
  }

  function handleSelectProvider(provider: SourceProvider) {
    setSelectedId(provider.id);
    setDetailMode('connect');
  }

  const providersByCategory = (category: SourceCatalogCategory): SourceProvider[] =>
    PROVIDERS.filter((provider) => provider.category === category);

  return (
    <div
      className="cw-dialog-backdrop"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="cw-dialog cw-ws-add-dialog"
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
        <div className="cw-ws-add-catalog">
          <div className="cw-ws-add-list">
            {SOURCE_CATEGORIES.map(({ key, labelKey }) => {
              const providers = providersByCategory(key);
              if (providers.length === 0) return null;
              return (
                <div key={key}>
                  <div className="cw-ws-add-cat">{t(labelKey)}</div>
                  {providers.map((provider) => {
                    const connected = isProviderConnected(provider, connectedIds);
                    return (
                      <button
                        type="button"
                        key={provider.id}
                        className={`cw-ws-add-source${selected.id === provider.id ? ' is-active' : ''}${connected ? '' : ' is-unconnected'}`}
                        onClick={() => handleSelectProvider(provider)}
                      >
                        <SourceIcon sourceId={provider.id} size={18} />
                        <span>{t(provider.nameKey)}</span>
                        <em>{connected ? t('workspace.connect.connected') : t('workspace.connect.notConnected')}</em>
                      </button>
                    );
                  })}
                </div>
              );
            })}
          </div>
          <div className="cw-ws-add-detail">
            <div className="cw-ws-add-detail-head">
              <SourceIcon sourceId={selected.id} size={22} />
              <div>
                <strong>{t(selected.nameKey)}</strong>
                <span>{selectedConnected ? t('workspace.connect.connectedHint') : t(`workspace.connect.instructions.${selected.id}`)}</span>
              </div>
            </div>
            {!selectedConnected && selectedGuide && (
              <div className="cw-ws-add-tabs" aria-label={t('workspace.connect.tabs.label')}>
                <button
                  type="button"
                  className={mode === 'connect' ? 'is-active' : ''}
                  aria-pressed={mode === 'connect'}
                  onClick={() => setDetailMode('connect')}
                >
                  {t('workspace.connect.tabs.connect')}
                </button>
                <button
                  type="button"
                  className={mode === 'guide' ? 'is-active' : ''}
                  aria-pressed={mode === 'guide'}
                  onClick={() => setDetailMode('guide')}
                >
                  {t('workspace.connect.tabs.guide')}
                </button>
              </div>
            )}
            {!selectedConnected && mode === 'connect' && (
              <div className="cw-ws-add-fields">
                {requiredFields.map((field) => (
                  <label key={field} className="cw-field">
                    <span>{t(`workspace.connect.fields.${field}`)}</span>
                    <input
                      className="cw-input"
                      placeholder={t(`workspace.connect.placeholders.${field}`)}
                    />
                  </label>
                ))}
              </div>
            )}
            {!selectedConnected && mode === 'guide' && selectedGuide && (
              <div className="cw-ws-add-guide">
                <div>
                  <strong>{t(selectedGuide.titleKey)}</strong>
                  <p>{t(selectedGuide.introKey)}</p>
                </div>
                <ol>
                  {selectedGuide.stepKeys.map((stepKey) => (
                    <li key={stepKey}>{t(stepKey)}</li>
                  ))}
                </ol>
                <div className="cw-ws-add-guide-permissions">
                  <span>{t('workspace.connect.guides.permissions')}</span>
                  <ul>
                    {selectedGuide.permissionKeys.map((permissionKey) => (
                      <li key={permissionKey}>{t(permissionKey)}</li>
                    ))}
                  </ul>
                </div>
                <p className="cw-ws-add-guide-note">{t(selectedGuide.noteKey)}</p>
              </div>
            )}
            {mode === 'guide' ? (
              <button
                type="button"
                className="cw-btn-secondary"
                onClick={() => setDetailMode('connect')}
              >
                {t('workspace.connect.guides.back')}
              </button>
            ) : (
              <button
                type="button"
                className="cw-btn-primary"
                onClick={handleConnect}
              >
                {selectedConnected ? t('workspace.connect.open') : t('workspace.connect.action')}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export function SourceRail({ activeSourceId }: SourceRailProps) {
  const { t } = useTranslation('files');
  const navigate = useNavigate();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [dialogSourceId, setDialogSourceId] = useState<string | null>(null);
  const [hideUnconnected, setHideUnconnected] = useState(false);
  const [connectedIds, setConnectedIds] = useState<Set<string>>(() => readConnectedSourceIds());

  const providersByCategory = (category: SourceCatalogCategory): SourceProvider[] =>
    PROVIDERS.filter((p) => p.category === category)
      .filter((provider) => !hideUnconnected || isProviderConnected(provider, connectedIds));
  const knowledgeProviders = PROVIDERS.filter((provider) => provider.category === 'knowledge');

  function openAddDialog(sourceId: string | null = null) {
    setDialogSourceId(sourceId);
    setDialogOpen(true);
  }

  function handleConnect(provider: SourceProvider) {
    const next = new Set(connectedIds);
    next.add(provider.id);
    setConnectedIds(next);
    writeConnectedSourceIds(next);
    setDialogOpen(false);
    navigate({
      to: '/workspace/$sourceId',
      params: { sourceId: provider.id },
    });
  }

  return (
    <>
      {/* "All sources" button — navigates to /workspace (no sourceId) */}
      <Link
        to="/workspace"
        className={`cw-ws-rail-all${activeSourceId == null ? ' is-active' : ''}`}
      >
        {t('workspace.all')}
      </Link>

      <div className="cw-ws-rail-group-label">{t('workspace.groups.sources')}</div>
      {SOURCE_CATEGORIES.map(({ key, labelKey }) => {
        const providers = providersByCategory(key);
        if (providers.length === 0) return null;
        return (
          <div key={key}>
            <div className="cw-ws-rail-cat">{t(labelKey)}</div>
            {providers.map((provider) => (
              isProviderConnected(provider, connectedIds) ? (
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
              ) : (
                <button
                  type="button"
                  key={provider.id}
                  className="cw-ws-rail-row is-unconnected"
                  onClick={() => openAddDialog(provider.id)}
                >
                  <SourceIcon sourceId={provider.id} size={18} />
                  <span className="cw-ws-rail-row-name">{t(provider.nameKey)}</span>
                  <span className="cw-ws-rail-badge">{t('workspace.connect.cta')}</span>
                </button>
              ))
            )}
          </div>
        );
      })}

      {knowledgeProviders.length > 0 && (
        <div className="cw-ws-rail-knowledge">
          <div className="cw-ws-rail-group-label">{t('workspace.groups.knowledge')}</div>
          {knowledgeProviders.map((provider) => (
            <Link
              key={provider.id}
              to="/workspace/$sourceId"
              params={{ sourceId: provider.id }}
              className={`cw-ws-rail-row cw-ws-rail-row-knowledge${activeSourceId === provider.id ? ' is-active' : ''}`}
            >
              <SourceIcon sourceId={provider.id} size={18} />
              <span className="cw-ws-rail-row-name">{t(provider.nameKey)}</span>
              <span className="cw-ws-rail-badge">
                {provider.count != null ? provider.count : '—'}
              </span>
            </Link>
          ))}
        </div>
      )}

      <label className="cw-ws-rail-toggle" style={{ marginTop: 'auto' }}>
        <input
          type="checkbox"
          checked={hideUnconnected}
          onChange={(event) => setHideUnconnected(event.target.checked)}
        />
        <span>{t('workspace.hideUnconnected')}</span>
      </label>

      {/* Add source button */}
      <button
        className="cw-ws-rail-add"
        onClick={() => openAddDialog()}
      >
        <span aria-hidden="true">＋</span> {t('workspace.addSource')}
      </button>

      {dialogOpen && (
        <AddSourceDialog
          connectedIds={connectedIds}
          initialSourceId={dialogSourceId}
          onClose={() => setDialogOpen(false)}
          onConnect={handleConnect}
        />
      )}
    </>
  );
}
