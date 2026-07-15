import { useEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { deleteEntry, getFileBlob } from '@/api/workspace';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { FilePreviewModal } from '@/components/FilePreviewModal';
import { Icon } from '@/components/Icon';
import { useDialogEscape } from '@/lib/useDialogEscape';
import { SourceIcon } from '@/workspace-connections/icons';
import { evidenceForRecord, getKnowledgeRecordsForSource, sourceEntryFromEvidence } from '@/workspace-connections/knowledge';
import { getProviderMeta } from '@/workspace-connections/providers';
import { useProvider } from '@/workspace-connections/hooks/useProviders';
import type { SourceEntry } from '@/workspace-connections/types';

interface DetailPanelProps {
  entry: SourceEntry;
  onClose: () => void;
  /**
   * Attach hand-off to the home composer (wired in Task 5). When omitted the
   * button shows a transient mock notice instead — the v1 behavior for every
   * source until the real local flow lands.
   */
  onAttach?: (entry: SourceEntry) => void;
  onSelectEntry?: (entry: SourceEntry) => void;
}

const NOTICE_MS = 2500;

/** Format a byte count as a human-readable string (B / KB / MB). */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Format an ISO date string as a short locale date. */
function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  } catch {
    return iso;
  }
}

/** One parsed bodyPreview line: "speaker: text" (threads) or plain text. */
interface BubbleLine {
  speaker: string | null;
  text: string;
  isSelf: boolean;
}

function parseBubbleLine(line: string): BubbleLine {
  const m = line.match(/^([^:]{1,24}):\s*(.*)$/);
  if (!m) return { speaker: null, text: line, isSelf: false };
  const speaker = m[1]!.trim();
  return { speaker, text: m[2]!, isSelf: speaker === '나' };
}

function renderDocumentBody(body: string) {
  return body
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line !== '')
    .map((line, index) => {
      if (line.startsWith('## ')) {
        return <h3 key={index}>{line.slice(3)}</h3>;
      }
      if (line.startsWith('- [ ] ') || line.startsWith('- [x] ')) {
        const checked = line.startsWith('- [x] ');
        return (
          <div key={index} className="cw-ws-page-check">
            <span className={checked ? 'is-checked' : ''}>{checked ? '✓' : ''}</span>
            <p>{line.slice(6)}</p>
          </div>
        );
      }
      if (line.startsWith('- ')) {
        return (
          <div key={index} className="cw-ws-page-bullet">
            <span>•</span>
            <p>{line.slice(2)}</p>
          </div>
        );
      }
      return <p key={index}>{line}</p>;
    });
}

/**
 * DetailPanel — right-hand aside showing meta + preview + actions for the
 * selected entry. Only mounted for file/item/thread entries (views never
 * select folders). ESC and the ✕ button both clear the selection.
 */
export function DetailPanel({ entry, onClose, onAttach, onSelectEntry }: DetailPanelProps) {
  const { t } = useTranslation('files');
  const qc = useQueryClient();

  const [lightboxOpen, setLightboxOpen] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [noticeVisible, setNoticeVisible] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const noticeTimer = useRef<number | null>(null);

  // ESC clears the selection. Dialogs opened on top (lightbox, confirm)
  // register later on the shared dialog stack, so they win ESC first.
  useDialogEscape(onClose, { disabled: deleting });

  // Resolve the mount-aware provider once at the top; the detail query closes
  // over it (a hook can't be called inside queryFn). undefined only during the
  // brief mounts-loading window, so the query waits via `enabled`.
  const provider = useProvider(entry.sourceId);
  // Knowledge evidence produces synthetic entries whose sourceId is a TYPE
  // (e.g. 'notion'), not a live instance id — useProvider() can't resolve
  // those (mount-backed types have no static-catalog instance). Fall back to
  // the static catalog by id (== type for every non-instance entry) so the
  // header still shows a translated name/icon instead of the raw type string.
  const catalogFallback = provider ? undefined : getProviderMeta(entry.sourceId);
  const headerType = provider?.type ?? catalogFallback?.type ?? entry.sourceId;
  const headerName = provider
    ? provider.label ?? t(provider.nameKey)
    : catalogFallback
      ? t(catalogFallback.nameKey)
      : entry.sourceId;

  const detailQuery = useQuery({
    queryKey: ['ws-detail', entry.sourceId, entry.id],
    enabled: !!provider,
    queryFn: () => {
      if (!provider) throw new Error(`unknown source: ${entry.sourceId}`);
      return provider.detail(entry.id);
    },
  });

  // Reset transient UI state when the selection moves to another entry.
  useEffect(() => {
    setLightboxOpen(false);
    setConfirmingDelete(false);
    setNoticeVisible(false);
    setActionError(null);
  }, [entry.sourceId, entry.id]);

  // Clear any pending notice timer on unmount.
  useEffect(
    () => () => {
      if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    },
    [],
  );

  const isLocal = entry.sourceId === 'local';
  const isFile = entry.kind === 'file';
  const isKnowledgeEntry = entry.kind === 'record';
  const detail = detailQuery.data;
  const isDocumentEntry =
    entry.kind === 'page' || (entry.kind === 'item' && provider?.category === 'docs');
  const relatedKnowledge = isKnowledgeEntry ? [] : getKnowledgeRecordsForSource(entry);

  function handleAttach() {
    if (onAttach) {
      onAttach(entry);
      return;
    }
    // Mock notice: v1 behavior for all sources until Task 5 wires the real flow.
    setNoticeVisible(true);
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNoticeVisible(false), NOTICE_MS);
  }

  async function handleDownload() {
    if (!entry.path) return;
    setActionError(null);
    try {
      const blob = await getFileBlob(entry.path);
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = entry.title;
      a.click();
      // Revoke after the browser has had time to start the download.
      window.setTimeout(() => URL.revokeObjectURL(url), 10_000);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Download failed');
    }
  }

  async function handleDelete() {
    if (!entry.path) return;
    setDeleting(true);
    setActionError(null);
    try {
      await deleteEntry(entry.path);
      await qc.invalidateQueries({ queryKey: ['ws'] });
      setConfirmingDelete(false);
      onClose();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Delete failed');
    } finally {
      setDeleting(false);
    }
  }

  const bubbleLines =
    entry.kind === 'thread' && detail?.bodyPreview
      ? detail.bodyPreview.split('\n').filter((l) => l.trim() !== '')
      : null;

  return (
    <div className="cw-ws-detail-panel">
      <div className="cw-ws-detail-header">
        <span className="cw-ws-detail-badge">
          <SourceIcon sourceId={headerType} size={14} />
          {headerName}
        </span>
        <button
          type="button"
          className="cw-ws-detail-close"
          aria-label={t('workspace.detail.close')}
          onClick={onClose}
        >
          <Icon name="x" size={16} />
        </button>
      </div>

      <div className="cw-ws-detail-body">
        <div className="cw-ws-detail-title" title={entry.title}>{entry.title}</div>
        <div className="cw-ws-detail-meta">
          {entry.size != null && <span>{formatBytes(entry.size)}</span>}
          <span>{formatDate(entry.modifiedAt)}</span>
        </div>

        {detailQuery.isLoading && (
          <div className="cw-ws-detail-loading">{t('workspace.loading')}</div>
        )}

        {isKnowledgeEntry && (
          <div className="cw-ws-knowledge-detail" data-testid="knowledge-detail-preview">
            <div className="cw-ws-knowledge-detail-meta">
              {entry.collection && <span>{entry.collection}</span>}
              <span>{t(`workspace.knowledge.status.${entry.status ?? 'draft'}`)}</span>
              {entry.confidence != null && (
                <span>{t('workspace.knowledge.confidence')}: {Math.round(entry.confidence * 100)}%</span>
              )}
            </div>
            {detail?.bodyPreview && (
              <div className="cw-ws-knowledge-detail-body">
                {detail.bodyPreview.split('\n').filter((line) => line.trim() !== '').map((line, index) => (
                  <p key={index}>{line}</p>
                ))}
              </div>
            )}
            {evidenceForRecord(entry).length > 0 && (
              <div className="cw-ws-knowledge-detail-evidence">
                <strong>{t('workspace.knowledge.basedOn')}</strong>
                {evidenceForRecord(entry).map((evidence) => {
                  const sourceEntry = sourceEntryFromEvidence(evidence);
                  const canOpen = evidence.sourceId !== 'local' && Boolean(onSelectEntry);
                  const content = (
                    <>
                      <span className="cw-ws-knowledge-evidence-title">{evidence.label}</span>
                      <span className="cw-ws-knowledge-evidence-excerpt">{evidence.excerpt}</span>
                      <span className="cw-ws-knowledge-evidence-used">
                        {t('workspace.knowledge.usedFor')}: {evidence.usedFor}
                      </span>
                    </>
                  );
                  return canOpen ? (
                    <button
                      type="button"
                      key={evidence.id}
                      className="cw-ws-knowledge-evidence-card"
                      onClick={() => onSelectEntry?.(sourceEntry)}
                    >
                      {content}
                    </button>
                  ) : (
                    <span key={evidence.id} className="cw-ws-knowledge-evidence-card is-static">
                      {content}
                    </span>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {/* Files: mini preview. Local opens the full lightbox; mock sources
            show a static placeholder (no blob exists to preview). */}
        {isFile && isLocal && entry.path && (
          <button
            type="button"
            className="cw-ws-detail-preview-box"
            onClick={() => setLightboxOpen(true)}
          >
            <Icon name="file" size={22} />
            <span>{t('ui.preview')}</span>
          </button>
        )}
        {isFile && !isLocal && (
          <div className="cw-ws-detail-preview-placeholder">
            <Icon name="file" size={22} />
          </div>
        )}

        {/* Threads: one speaker-labeled bubble per line, reusing chat classes. */}
        {bubbleLines && (
          <div className="cw-ws-detail-bubbles">
            {bubbleLines.map((line, i) => {
              const { speaker, text, isSelf } = parseBubbleLine(line);
              return (
                <div key={i} className={`cw-message${isSelf ? ' is-self' : ''}`}>
                  <div className="cw-message-body">
                    {speaker && !isSelf && (
                      <span className="cw-ws-detail-speaker">{speaker}</span>
                    )}
                    <div className="cw-message-bubble">
                      <p className="cw-message-text">{text}</p>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        )}

        {/* Items outside docs keep the compact excerpt rendering. */}
        {entry.kind === 'item' && !isDocumentEntry && detail?.bodyPreview && (
          <p className="cw-ws-detail-excerpt">{detail.bodyPreview}</p>
        )}

        {/* Docs/tickets/pages: document-like preview that mirrors the source body. */}
        {isDocumentEntry && detail?.bodyPreview && (
          <div className="cw-ws-page-preview" data-testid="source-document-preview">
            <div className="cw-ws-page-icon" aria-hidden="true">
              {entry.emoji ?? (provider ? <SourceIcon sourceId={provider.type} size={22} /> : '📄')}
            </div>
            {renderDocumentBody(detail.bodyPreview)}
          </div>
        )}

        {!isKnowledgeEntry && relatedKnowledge.length > 0 && (
          <div className="cw-ws-source-knowledge-links" data-testid="source-knowledge-links">
            <strong>{t('workspace.knowledge.fromThisSource')}</strong>
            {relatedKnowledge.map((record) => (
              <button
                type="button"
                key={record.id}
                onClick={() => onSelectEntry?.(record)}
              >
                <span>{record.title}</span>
                <em>{t(`workspace.knowledge.status.${record.status ?? 'draft'}`)}</em>
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="cw-ws-detail-actions">
        {actionError && <span className="cw-form-error">{actionError}</span>}
        <button type="button" className="cw-btn-primary" onClick={handleAttach}>
          {t('workspace.detail.openChat')}
        </button>
        {noticeVisible && (
          <div className="cw-ws-detail-notice" role="status">
            {t('workspace.detail.mockAttachToast')}
          </div>
        )}
        {isLocal && isFile && (
          <>
            <button type="button" className="cw-btn-secondary" onClick={() => void handleDownload()}>
              {t('workspace.detail.download')}
            </button>
            <button
              type="button"
              className="cw-btn-secondary"
              onClick={() => setConfirmingDelete(true)}
            >
              {t('workspace.detail.delete')}
            </button>
          </>
        )}
        {!isLocal && detail?.externalUrl && (
          <a
            className="cw-btn-secondary"
            href={detail.externalUrl}
            target="_blank"
            rel="noopener noreferrer"
          >
            {t('workspace.detail.openOriginal')}
          </a>
        )}
      </div>

      {lightboxOpen && entry.path && (
        <FilePreviewModal
          path={entry.path}
          name={entry.title}
          onClose={() => setLightboxOpen(false)}
        />
      )}

      {confirmingDelete && (
        <ConfirmDialog
          title={t('delete.file_title')}
          body={t('delete.file_body', { name: entry.title })}
          confirmLabel={t('delete.confirm')}
          destructive
          pending={deleting}
          onConfirm={() => void handleDelete()}
          onClose={() => setConfirmingDelete(false)}
        />
      )}
    </div>
  );
}
