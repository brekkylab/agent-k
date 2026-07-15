import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { SourceIcon } from '@/workspace-connections/icons';
import { useProviders, useMounts, useUnconnectedCatalog } from '@/workspace-connections/hooks/useProviders';
import { PROVIDERS, MOUNT_BACKED_TYPES } from '@/workspace-connections/providers';
import { deleteMount, type MountResponse } from '@/api/mounts';
import { NotionMountForm } from './NotionMountForm';
import { S3MountForm } from './S3MountForm';
import type { SourceCategory, SourceProvider, SourceType } from '@/workspace-connections/types';

interface SourceRailProps {
  activeSourceId: string | null;
  onOpenSource: (id: string | null) => void;
}

// Isolated from the original `/workspace` variant's localStorage state so
// mock-connecting a source here never leaks into (or is leaked into by) the
// frozen original tab.
const CONNECTED_STORAGE_KEY = 'cw.workspace-connections.connectedSources';

type SourceCatalogCategory = Exclude<SourceCategory, 'knowledge'>;

// Source sections in display order. Knowledge is rendered as its own layer.
const SOURCE_CATEGORIES: { key: SourceCatalogCategory; labelKey: string }[] = [
  { key: 'files', labelKey: 'workspace.cat.files' },
  { key: 'docs', labelKey: 'workspace.cat.docs' },
  { key: 'messages', labelKey: 'workspace.cat.messages' },
];

// Keyed on the catalog TYPE, not a provider instance id — the add-dialog
// type-picker always operates on types (a type has 0..N connection instances).
const CONNECTION_FIELDS: Partial<Record<SourceType, string[]>> = {
  dropbox: ['accountEmail', 'accessToken'],
  figma: ['teamUrl', 'accessToken'],
  github: ['repositoryUrl', 'accessToken'],
  linear: ['workspaceUrl', 'apiKey'],
};

// A guide is either a single flow (`stepKeys`) or split into labelled
// `sections`, each with its own steps — the latter lets a multi-stage setup
// (create → share → connect) read as distinct stages instead of one long list.
type GuideSection = { titleKey: string; stepKeys: string[] };
type GuideDef = {
  titleKey: string;
  introKey: string;
  stepKeys?: string[];
  sections?: GuideSection[];
  permissionKeys: string[];
  noteKey: string;
};

const CONNECTION_GUIDES: Partial<Record<SourceType, GuideDef>> = {
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
  notion: {
    titleKey: 'workspace.connect.guides.notion.title',
    introKey: 'workspace.connect.guides.notion.intro',
    sections: [
      {
        titleKey: 'workspace.connect.guides.notion.create.title',
        stepKeys: [
          'workspace.connect.guides.notion.create.step1',
          'workspace.connect.guides.notion.create.step2',
          'workspace.connect.guides.notion.create.step3',
        ],
      },
      {
        titleKey: 'workspace.connect.guides.notion.share.title',
        stepKeys: [
          'workspace.connect.guides.notion.share.step1',
          'workspace.connect.guides.notion.share.step2',
          'workspace.connect.guides.notion.share.step3',
        ],
      },
      {
        titleKey: 'workspace.connect.guides.notion.connect.title',
        stepKeys: [
          'workspace.connect.guides.notion.connect.step1',
          'workspace.connect.guides.notion.connect.step2',
        ],
      },
    ],
    permissionKeys: [
      'workspace.connect.guides.notion.scopeRead',
      'workspace.connect.guides.notion.scopeShared',
    ],
    noteKey: 'workspace.connect.guides.notion.note',
  },
  s3: {
    titleKey: 'workspace.connect.guides.s3.title',
    introKey: 'workspace.connect.guides.s3.intro',
    sections: [
      {
        titleKey: 'workspace.connect.guides.s3.user.title',
        stepKeys: [
          'workspace.connect.guides.s3.user.step1',
          'workspace.connect.guides.s3.user.step2',
        ],
      },
      {
        titleKey: 'workspace.connect.guides.s3.policy.title',
        stepKeys: [
          'workspace.connect.guides.s3.policy.step1',
          'workspace.connect.guides.s3.policy.step2',
          'workspace.connect.guides.s3.policy.step3',
        ],
      },
      {
        titleKey: 'workspace.connect.guides.s3.key.title',
        stepKeys: [
          'workspace.connect.guides.s3.key.step1',
          'workspace.connect.guides.s3.key.step2',
        ],
      },
      {
        titleKey: 'workspace.connect.guides.s3.connect.title',
        stepKeys: [
          'workspace.connect.guides.s3.connect.step1',
          'workspace.connect.guides.s3.connect.step2',
        ],
      },
    ],
    permissionKeys: [
      'workspace.connect.guides.s3.scopeList',
      'workspace.connect.guides.s3.scopeGet',
    ],
    noteKey: 'workspace.connect.guides.s3.note',
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

// Real instances are always `connected: true`. Mock-connectable catalog types
// (github/linear/dropbox/figma) are gated by localStorage `connectedIds` —
// kept so the mock Connect action still surfaces its target afterward instead
// of dead-ending into an invisible rail.
function isProviderConnected(provider: SourceProvider, connectedIds: Set<string>): boolean {
  return provider.connected || connectedIds.has(provider.id);
}

// Render guide-step text, turning any embedded http(s) URL into a clickable
// link. The URL character class stops at non-ASCII (so a trailing Korean
// particle like "…my-integrations를" isn't swallowed into the href).
const URL_SPLIT_RE = /(https?:\/\/[A-Za-z0-9./_\-?=&#%~+]+)/g;
function renderTextWithLinks(text: string): ReactNode[] {
  // split() keeps the captured URL as its own segment; a URL segment always
  // starts with the scheme, so a plain startsWith check (not the global regex,
  // whose lastIndex is stateful) classifies each segment.
  return text.split(URL_SPLIT_RE).map((part, i) =>
    /^https?:\/\//.test(part) ? (
      <a key={i} href={part} target="_blank" rel="noreferrer">
        {part}
      </a>
    ) : (
      part
    ),
  );
}

// The setup guide body: intro, then either one flow or several labelled
// sections, followed by the required-access box and a closing note. Shared by
// the mount-backed and mock connect branches so both render guides identically.
function GuideBody({ guide }: { guide: GuideDef }) {
  const { t } = useTranslation('files');
  // A single-flow guide is one unlabelled section over its `stepKeys`.
  const sections: GuideSection[] = guide.sections ?? [{ titleKey: '', stepKeys: guide.stepKeys ?? [] }];
  return (
    <div className="cw-ws-add-guide">
      <div>
        <strong>{t(guide.titleKey)}</strong>
        <p>{t(guide.introKey)}</p>
      </div>
      {sections.map((section, i) => (
        <div key={section.titleKey || i} className="cw-ws-add-guide-section">
          {section.titleKey && <strong>{t(section.titleKey)}</strong>}
          <ol>
            {section.stepKeys.map((stepKey) => (
              <li key={stepKey}>{renderTextWithLinks(t(stepKey))}</li>
            ))}
          </ol>
        </div>
      ))}
      <div className="cw-ws-add-guide-permissions">
        <span>{t('workspace.connect.guides.permissions')}</span>
        <ul>
          {guide.permissionKeys.map((permissionKey) => (
            <li key={permissionKey}>{t(permissionKey)}</li>
          ))}
        </ul>
      </div>
      <p className="cw-ws-add-guide-note">{t(guide.noteKey)}</p>
    </div>
  );
}

function AddSourceDialog({
  connectedIds,
  initialType,
  onClose,
  onMockConnect,
  onOpenSource,
}: {
  connectedIds: Set<string>;
  initialType: SourceType | null;
  onClose: () => void;
  onMockConnect: (provider: SourceProvider) => void;
  onOpenSource: (id: string) => void;
}) {
  const { t } = useTranslation('files');
  const { data: mounts } = useMounts();
  const qc = useQueryClient();

  // Type-picker always iterates the static catalog (types), never instances —
  // a type can have 0..N real connections, listed separately below.
  const firstUnconnected = PROVIDERS.find((provider) => !isProviderConnected(provider, connectedIds));
  const initialProvider =
    PROVIDERS.find((provider) => provider.type === initialType) ?? firstUnconnected ?? PROVIDERS[0]!;
  const [selectedType, setSelectedType] = useState<SourceType>(initialProvider.type);
  const [detailMode, setDetailMode] = useState<'connect' | 'guide'>('connect');
  const selected = PROVIDERS.find((provider) => provider.type === selectedType) ?? initialProvider;
  const selectedConnected = isProviderConnected(selected, connectedIds);
  const requiredFields = selectedConnected ? [] : CONNECTION_FIELDS[selected.type] ?? ['workspaceUrl', 'apiKey'];
  const selectedGuide = selectedConnected ? undefined : CONNECTION_GUIDES[selected.type];
  const mode = selectedGuide ? detailMode : 'connect';
  const isMountBacked = MOUNT_BACKED_TYPES.has(selected.type);
  // Every existing real connection of the selected type, each independently
  // disconnectable — a type-picker selection is never an instance id.
  const existingConnections = isMountBacked
    ? (mounts ?? []).filter((m) => m.provider.type === selected.type)
    : [];

  const disconnect = useMutation({
    mutationFn: (id: string) => deleteMount(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['workspace', 'mounts'] }),
  });

  function handleMockConnect() {
    onMockConnect(selected);
  }

  function handleSelectProvider(type: SourceType) {
    setSelectedType(type);
    setDetailMode('connect');
  }

  function handleMountCreated(mount: MountResponse) {
    onClose();
    onOpenSource(mount.id);
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
                        key={provider.type}
                        className={`cw-ws-add-source${selected.type === provider.type ? ' is-active' : ''}${connected ? '' : ' is-unconnected'}`}
                        onClick={() => handleSelectProvider(provider.type)}
                      >
                        <SourceIcon sourceId={provider.type} size={18} />
                        <span>{provider.label ?? t(provider.nameKey)}</span>
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
              <SourceIcon sourceId={selected.type} size={22} />
              <div>
                <strong>{t(selected.nameKey)}</strong>
                <span>{selectedConnected ? t('workspace.connect.connectedHint') : t(`workspace.connect.instructions.${selected.type}`)}</span>
              </div>
            </div>
            {isMountBacked ? (
              // Notion/S3 connect via a real workspace mount (a MountForm), not
              // the mock localStorage catalog-connect. Existing connections of
              // this type are listed with their own Disconnect; the form below
              // is always available to add another connection.
              <>
                {existingConnections.length > 0 && (
                  <div className="cw-ws-add-fields">
                    <strong style={{ fontSize: 12, color: 'var(--cw-fg-3)' }}>
                      {t('workspace.connect.existingConnections', 'Existing connections')}
                    </strong>
                    {existingConnections.map((mount) => (
                      <div key={mount.id} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
                        <span style={{ fontSize: 13, color: 'var(--cw-fg-2)' }}>{mount.label ?? mount.prefix}</span>
                        <button
                          type="button"
                          className="cw-btn"
                          disabled={disconnect.isPending}
                          onClick={() => disconnect.mutate(mount.id)}
                        >
                          {t('workspace.connect.disconnect', 'Disconnect')}
                        </button>
                      </div>
                    ))}
                  </div>
                )}
                <strong style={{ fontSize: 13, display: 'block', margin: existingConnections.length > 0 ? '12px 0 0' : 0 }}>
                  {t('workspace.addConnection')}
                </strong>
                {selectedGuide && (
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
                {mode === 'guide' && selectedGuide ? (
                  <GuideBody guide={selectedGuide} />
                ) : selected.type === 'notion' ? (
                  <NotionMountForm onCreated={handleMountCreated} />
                ) : (
                  <S3MountForm onCreated={handleMountCreated} />
                )}
              </>
            ) : (
              <>
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
                  <GuideBody guide={selectedGuide} />
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
                    onClick={handleMockConnect}
                  >
                    {selectedConnected ? t('workspace.connect.open') : t('workspace.connect.action')}
                  </button>
                )}
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export function SourceRail({ activeSourceId, onOpenSource }: SourceRailProps) {
  const { t } = useTranslation('files');
  const [dialogOpen, setDialogOpen] = useState(false);
  const [dialogInitialType, setDialogInitialType] = useState<SourceType | null>(null);
  const [connectedIds, setConnectedIds] = useState<Set<string>>(() => readConnectedSourceIds());
  const allProviders = useProviders();
  const hintCandidates = useUnconnectedCatalog(connectedIds);

  // Rail = connections only. A provider (real instance or mock-connected
  // catalog entry) renders iff `isProviderConnected` — disconnected catalog
  // placeholders (including the s3/notion static entries, already excluded
  // from `useProviders()`) never reach the rail.
  const providersByCategory = (category: SourceCatalogCategory): SourceProvider[] =>
    allProviders
      .filter((p) => p.category === category)
      .filter((provider) => isProviderConnected(provider, connectedIds));
  const knowledgeProviders = allProviders.filter((provider) => provider.category === 'knowledge');

  // Discovery hint: stable within a ~60s window (never re-rolled every
  // render), hidden entirely once there is nothing left to suggest.
  const hintProvider =
    hintCandidates.length > 0
      ? hintCandidates[Math.floor(Date.now() / 60000) % hintCandidates.length]
      : null;

  function openAddDialog(type: SourceType | null = null) {
    setDialogInitialType(type);
    setDialogOpen(true);
  }

  function handleMockConnect(provider: SourceProvider) {
    const next = new Set(connectedIds);
    next.add(provider.id);
    setConnectedIds(next);
    writeConnectedSourceIds(next);
    setDialogOpen(false);
    onOpenSource(provider.id);
  }

  return (
    <>
      {/* "All sources" button — internal state, no navigation in this variant. */}
      <button
        type="button"
        className={`cw-ws-rail-all${activeSourceId == null ? ' is-active' : ''}`}
        onClick={() => onOpenSource(null)}
      >
        {t('workspace.all')}
      </button>

      <div className="cw-ws-rail-group-label">{t('workspace.groups.sources')}</div>
      {SOURCE_CATEGORIES.map(({ key, labelKey }) => {
        const providers = providersByCategory(key);
        if (providers.length === 0) return null;
        return (
          <div key={key}>
            <div className="cw-ws-rail-cat">{t(labelKey)}</div>
            {providers.map((provider) => (
              <button
                type="button"
                key={provider.id}
                className={`cw-ws-rail-row${activeSourceId === provider.id ? ' is-active' : ''}`}
                onClick={() => onOpenSource(provider.id)}
              >
                <SourceIcon sourceId={provider.type} size={18} />
                <span className="cw-ws-rail-row-name">{provider.label ?? t(provider.nameKey)}</span>
                <span className="cw-ws-rail-badge">
                  {provider.count != null ? provider.count : '—'}
                </span>
              </button>
            ))}
          </div>
        );
      })}

      {knowledgeProviders.length > 0 && (
        <div className="cw-ws-rail-knowledge">
          <div className="cw-ws-rail-group-label">{t('workspace.groups.knowledge')}</div>
          {knowledgeProviders.map((provider) => (
            <button
              type="button"
              key={provider.id}
              className={`cw-ws-rail-row cw-ws-rail-row-knowledge${activeSourceId === provider.id ? ' is-active' : ''}`}
              onClick={() => onOpenSource(provider.id)}
            >
              <SourceIcon sourceId={provider.type} size={18} />
              <span className="cw-ws-rail-row-name">{provider.label ?? t(provider.nameKey)}</span>
              <span className="cw-ws-rail-badge">
                {provider.count != null ? provider.count : '—'}
              </span>
            </button>
          ))}
        </div>
      )}

      {hintProvider && (
        <button
          type="button"
          className="cw-ws-rail-row"
          style={{ marginTop: 'auto', fontStyle: 'italic', color: 'var(--cw-fg-4)' }}
          onClick={() => openAddDialog(hintProvider.type)}
        >
          <SourceIcon sourceId={hintProvider.type} size={16} />
          <span className="cw-ws-rail-row-name">
            {t('workspace.connect.hint', { name: t(hintProvider.nameKey) })}
          </span>
        </button>
      )}

      {/* Add source button */}
      <button
        className="cw-ws-rail-add"
        style={hintProvider ? undefined : { marginTop: 'auto' }}
        onClick={() => openAddDialog()}
      >
        <span aria-hidden="true">＋</span> {t('workspace.addSource')}
      </button>

      {dialogOpen && (
        <AddSourceDialog
          connectedIds={connectedIds}
          initialType={dialogInitialType}
          onClose={() => setDialogOpen(false)}
          onMockConnect={handleMockConnect}
          onOpenSource={onOpenSource}
        />
      )}
    </>
  );
}
