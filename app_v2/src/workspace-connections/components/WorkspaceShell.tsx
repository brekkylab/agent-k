import { useState, useRef, useEffect, createContext, useContext } from 'react';
import { useTranslation } from 'react-i18next';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { putFile } from '@/api/workspace';
import { Icon } from '@/components/Icon';
import { setPendingAttachment } from '@/stores/pendingAttachment';
import { recentAcross } from '@/workspace-connections/providers';
import { useProvider, useProviders } from '@/workspace-connections/hooks/useProviders';
import { SourceRail } from './SourceRail';
import { DetailPanel } from './DetailPanel';
import { UnifiedList } from './UnifiedList';
import { FileBrowserView } from './FileBrowserView';
import { ItemListView } from './ItemListView';
import { NotionPageView } from './NotionPageView';
import { KnowledgeRecordView } from './KnowledgeRecordView';
import { ThreadListView } from './ThreadListView';
import type { SourceEntry } from '@/workspace-connections/types';

export interface WorkspaceOutletContext {
  onSelect: (entry: SourceEntry) => void;
}

// React context used to pass onSelect down to descendant view components.
// This variant has no sub-routes (source selection is internal useState), but
// the context is kept so views can stay decoupled from how selection is wired.
const WorkspaceContext = createContext<WorkspaceOutletContext | null>(null);

export function useWorkspaceSelection(): WorkspaceOutletContext {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) throw new Error('useWorkspaceSelection must be used inside WorkspaceShell');
  return ctx;
}

// Isolated multi-connection variant of WorkspaceShell: the active source is
// internal useState (not a router param), and there is no <Outlet/> — the
// source view renders inline via the provider.kind switch. Mirrors the B-F
// design-candidate shells, which are also self-contained (no sub-routes).
export function WorkspaceShell() {
  const { t } = useTranslation('files');
  const qc = useQueryClient();
  const navigate = useNavigate();
  const uploadRef = useRef<HTMLInputElement>(null);

  const [activeSourceId, setActiveSourceId] = useState<string | null>(null);
  const [selected, setSelected] = useState<SourceEntry | null>(null);
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);

  const providers = useProviders();
  const provider = useProvider(activeSourceId ?? undefined);

  // Recents query only runs for the "all sources" view; `enabled` skips it
  // entirely while browsing a specific source.
  const connectedIds = providers.filter((p) => p.connected).map((p) => p.id);
  const { data: recentEntries = [], isLoading: recentsLoading } = useQuery({
    queryKey: ['ws', 'all', connectedIds],
    queryFn: () => recentAcross(providers),
    enabled: activeSourceId == null,
  });

  // Clear selection when the user opens a different source.
  useEffect(() => {
    setSelected(null);
  }, [activeSourceId]);

  function handleSelect(entry: SourceEntry) {
    setSelected(entry);
  }

  function handleOpenSource(id: string | null) {
    setActiveSourceId(id);
  }

  // Local-only attach: store the shared-mount path then navigate home so the
  // chip appears. Mock sources keep the toast notice (onAttach stays undefined
  // for non-local entries — DetailPanel falls back to its notice behavior).
  function handleAttach(entry: SourceEntry) {
    setPendingAttachment({
      name: entry.title,
      sharedPath: '/root/shared' + (entry.path ?? '/' + entry.id),
    });
    void navigate({ to: '/' });
  }

  async function handleUpload(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    setUploading(true);
    setUploadError(null);
    try {
      await putFile('/' + file.name, file);
      await qc.invalidateQueries({ queryKey: ['ws'] });
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Unknown error';
      setUploadError(msg);
    } finally {
      setUploading(false);
      if (uploadRef.current) uploadRef.current.value = '';
    }
  }

  return (
    <WorkspaceContext.Provider value={{ onSelect: handleSelect }}>
      <div className="cw-ws-shell">
        {/* Left source rail */}
        <nav className="cw-ws-rail">
          <SourceRail activeSourceId={activeSourceId} onOpenSource={handleOpenSource} />
        </nav>

        {/* Main content area */}
        <main className="cw-ws-main">
          {/* Toolbar: upload button when showing all sources or local source view */}
          {(activeSourceId == null || activeSourceId === 'local') && (
            <div className="cw-ws-toolbar" style={{ justifyContent: 'flex-end' }}>
              {uploadError && (
                <span className="cw-form-error" style={{ flex: 1, margin: 0 }}>
                  {uploadError}
                </span>
              )}
              <label style={{ cursor: uploading ? 'wait' : 'pointer' }}>
                <input
                  ref={uploadRef}
                  type="file"
                  style={{ display: 'none' }}
                  onChange={handleUpload}
                  disabled={uploading}
                />
                <span
                  className="cw-btn-primary"
                  style={{ pointerEvents: uploading ? 'none' : 'auto', opacity: uploading ? 0.7 : 1 }}
                >
                  <Icon name="upload" size={14} />
                  {uploading ? t('ui.uploading') : t('ui.upload')}
                </span>
              </label>
            </div>
          )}

          {/* Source view: inline kind-switch, no <Outlet/> in this variant. */}
          {activeSourceId == null ? (
            <UnifiedList entries={recentEntries} onSelect={handleSelect} loading={recentsLoading} />
          ) : !provider ? null : provider.kind === 'files' ? (
            <FileBrowserView provider={provider} onSelect={handleSelect} />
          ) : provider.kind === 'items' ? (
            <ItemListView provider={provider} onSelect={handleSelect} />
          ) : provider.kind === 'pages' ? (
            <NotionPageView provider={provider} onSelect={handleSelect} />
          ) : provider.kind === 'records' ? (
            <KnowledgeRecordView provider={provider} onSelect={handleSelect} />
          ) : (
            <ThreadListView provider={provider} onSelect={handleSelect} />
          )}
        </main>

        {/* Detail aside — panel for the selected entry; ESC/✕ clears selection.
            Local entries get handleAttach; mock sources pass undefined so DetailPanel shows its notice. */}
        <aside className="cw-ws-detail">
          {selected && (
            <DetailPanel
              key={`${selected.sourceId}:${selected.id}`}
              entry={selected}
              onClose={() => setSelected(null)}
              onAttach={selected.sourceId === 'local' ? handleAttach : undefined}
              onSelectEntry={handleSelect}
            />
          )}
        </aside>
      </div>
    </WorkspaceContext.Provider>
  );
}
