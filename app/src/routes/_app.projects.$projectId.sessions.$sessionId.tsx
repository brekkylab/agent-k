// Session — markup mirrors app-live SessionPage. Chat surface (head + messages
// + composer) + right side (members, references, access, artifact).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createFileRoute } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getSession, updateSessionShareMode } from '@/api/sessions';
import { listMessages, streamMessage } from '@/api/messages';
import { getProject, listMembers } from '@/api/projects';
import { deleteDirent, downloadFile, uploadFiles, type DirentScope } from '@/api/dirents';
import { Icon } from '@/components/Icon';
import { Avatar, IntentBadge, SharePill, ShareSelect } from '@/components/uiPrimitives';
import { useAuthStore } from '@/stores/auth';
import { useToastStore } from '@/components/Toast';
import { shareMeta } from '@/domain/metadata';
import { MarkdownRenderer } from '@/components/chat/MarkdownRenderer';
import { AI_USER, SUBAGENT_PREFIX } from '@/api/transformers';
import { formatMessageTime, formatMessageTimeFull } from '@/lib/formatMessageTime';
import type { Message, ShareMode, User } from '@/domain/types';
import { ApiError } from '@/api/client';
import { SessionTitleText } from '@/components/SessionTitleText';
import { ArtifactsPanel } from '@/components/ArtifactsPanel';
import { CopyToSharedDialog } from '@/components/CopyToSharedDialog';
import { AttachmentChip } from '@/components/AttachmentChip';
import { AttachmentPreview } from '@/components/AttachmentPreview';
import { FileTypeIcon } from '@/components/FileTypeIcon';
import { ConfirmDialog } from '@/components/ConfirmDialog';

export const Route = createFileRoute('/_app/projects/$projectId/sessions/$sessionId')({
  component: SessionPage,
});

function stripSubagentPrefix(name: string): string {
  return name.startsWith(SUBAGENT_PREFIX) ? name.slice(SUBAGENT_PREFIX.length) : name;
}

function SessionPage() {
  const { projectId, sessionId } = Route.useParams();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const currentUser = useAuthStore((s) => s.currentUser);

  const project = useQuery({ queryKey: ['project', projectId], queryFn: () => getProject(projectId) });
  const session = useQuery({ queryKey: ['session', sessionId], queryFn: () => getSession(sessionId) });
  const members = useQuery({ queryKey: ['members', projectId], queryFn: () => listMembers(projectId) });
  const history = useQuery({
    queryKey: ['messages', sessionId],
    queryFn: () => listMessages(sessionId),
    enabled: Boolean(session.data && currentUser),
  });

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [composerText, setComposerText] = useState('');
  const [liveMessages, setLiveMessages] = useState<Message[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [copyToSharedPaths, setCopyToSharedPaths] = useState<string[] | null>(null);

  type PendingAttachment = {
    tempId: string;
    filename: string;
    status: 'uploading' | 'uploaded' | 'error';
    globalPath?: string;
    error?: string;
  };
  const [pendingAttachments, setPendingAttachments] = useState<PendingAttachment[]>([]);

  useEffect(() => {
    setLiveMessages([]);
    setComposerText('');
    setStreaming(false);
  }, [sessionId]);

  // After messages load, mark-read side effect has run on the backend — sync badge in session list.
  useEffect(() => {
    if (history.isSuccess) {
      void queryClient.invalidateQueries({ queryKey: ['sessions', projectId] });
    }
  }, [history.isSuccess, history.dataUpdatedAt, projectId, queryClient]);

  const allMessages = useMemo<Message[]>(() => [
    ...(history.data ?? []),
    ...liveMessages,
  ], [history.data, liveMessages]);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [allMessages.length, streaming]);

  const handleFileSelect = useCallback(async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? []);
    if (files.length === 0) return;
    // Reset input so same file can be selected again
    e.target.value = '';

    for (const file of files) {
      const tempId = `${Date.now()}-${file.name}`;
      setPendingAttachments((prev) => [...prev, { tempId, filename: file.name, status: 'uploading' }]);
      try {
        const scope = { kind: 'inputs' as const, projectId, sessionId };
        const result = await uploadFiles(scope, [{ file, targetPath: file.name }]);
        const succeeded = result.succeeded[0];
        if (succeeded) {
          setPendingAttachments((prev) => prev.map((a) =>
            a.tempId === tempId ? { ...a, status: 'uploaded', globalPath: succeeded.path } : a
          ));
        } else {
          const err = result.failed[0]?.error ?? 'upload failed';
          setPendingAttachments((prev) => prev.map((a) =>
            a.tempId === tempId ? { ...a, status: 'error', error: err } : a
          ));
        }
      } catch {
        setPendingAttachments((prev) => prev.map((a) =>
          a.tempId === tempId ? { ...a, status: 'error', error: 'upload failed' } : a
        ));
      }
    }
  }, [projectId, sessionId]);

  const send = useCallback(async () => {
    const text = composerText.trim();
    if (!text || streaming || hasUploadingAttachments) return;

    const attachmentPaths = pendingAttachments
      .filter((a) => a.status === 'uploaded' && a.globalPath)
      .map((a) => a.globalPath!);

    setComposerText('');
    setPendingAttachments([]);

    const nowIso = new Date().toISOString();
    const userMsg: Message = {
      id: `live-user-${Date.now()}`,
      sessionId,
      sender: { kind: 'user', userId: currentUser?.id ?? 'user' },
      createdAt: nowIso,
      body: text,
      status: 'done',
      attachments: attachmentPaths.length > 0 ? attachmentPaths : undefined,
    };
    const aiId = `live-ai-${Date.now()}`;
    setLiveMessages((prev) => [...prev, userMsg, {
      id: aiId,
      sessionId,
      sender: { kind: 'agent' as const, name: 'agent-k' },
      createdAt: nowIso,
      body: '',
      status: 'streaming' as const,
    }]);
    setStreaming(true);

    const ctrl = new AbortController();

    try {
      for await (const update of streamMessage(sessionId, text, ctrl.signal, attachmentPaths.length > 0 ? attachmentPaths : undefined)) {
        const isDone = update.status === 'done';
        const doneOrStreaming = isDone ? 'done' as const : 'streaming' as const;

        setLiveMessages((prev) => {
          // Update main agent-k bubble
          let next: Message[] = prev.map((m) => {
            if (m.id !== aiId) return m;
            const updatedToolCalls = update.toolCalls.length > 0
              ? update.toolCalls.map((tc) => ({ ...tc }))
              : m.toolCalls;
            return { ...m, body: update.text, status: doneOrStreaming, toolCalls: updatedToolCalls };
          });

          // Apply subagent bubble updates — stable id derived from sender name
          for (const sub of update.subagentUpdates) {
            const subId = `live-sub-${sub.sourceAgent}`;
            const exists = next.some((m) => m.id === subId);
            if (exists) {
              next = next.map((m) =>
                m.id === subId ? { ...m, body: sub.text, status: doneOrStreaming } : m,
              );
            } else {
              next = [...next, {
                id: subId,
                sessionId,
                sender: { kind: 'agent' as const, name: sub.sourceAgent },
                createdAt: nowIso,
                body: sub.text,
                status: 'streaming' as const,
              }];
            }
          }

          return next;
        });

        if (update.status === 'error') {
          showToast(`스트리밍 실패: ${update.errorText ?? 'unknown'}`);
          break;
        }
      }
    } catch (err) {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'stream failed';
      showToast(`전송 실패: ${msg}`);
    } finally {
      setStreaming(false);
      // Refetch and wait for new history data before clearing live messages to avoid flash.
      await queryClient.refetchQueries({ queryKey: ['messages', sessionId] });
      setLiveMessages([]);
      void queryClient.invalidateQueries({ queryKey: ['session', sessionId] });
      void queryClient.invalidateQueries({ queryKey: ['sessions', projectId] });
      void queryClient.invalidateQueries({ queryKey: ['dirents', 'artifacts', projectId, sessionId] });
    }
  }, [composerText, streaming, sessionId, projectId, currentUser, queryClient, showToast, pendingAttachments]);

  const shareMutation = useMutation({
    mutationFn: (mode: ShareMode) => updateSessionShareMode(sessionId, mode),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['session', sessionId] });
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectId] });
      showToast('공유 모드가 변경되었습니다');
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'update failed';
      showToast(`공유 변경 실패: ${msg}`);
    },
  });

  const sess = session.data;
  const userList = members.data ?? [];
  const creator = userList.find((u) => u.id === sess?.creatorId);
  const usersForRender: User[] = [...userList, AI_USER];
  const hasUploadingAttachments = pendingAttachments.some((a) => a.status === 'uploading');



  return (
    <div className="cw-session-layout cw-page-enter">
      <section className="cw-chat-surface">
        <div className="cw-chat-head">
          <div>
            <h1><SessionTitleText title={sess?.title ?? '...'} /></h1>
            <p>
              {creator && <>Started by <Avatar user={creator} small /> {creator.name} · </>}
              {sess?.references.length ?? 0} files ·{' '}
              <Avatar user={AI_USER} small /> Cowork Default
            </p>
          </div>
          <div className="cw-session-head-actions">
            {sess && <IntentBadge intent={sess.intent} />}
            {sess && (
              <ShareSelect mode={sess.shareMode} onChange={(mode) => shareMutation.mutate(mode)} />
            )}
          </div>
        </div>

        <div className="cw-messages">
          {allMessages.map((msg) => (
            <MessageBubble
              key={msg.id}
              message={msg}
              users={usersForRender}
              currentUserId={currentUser?.id ?? ''}
              artifactPaths={msg.artifacts}
              projectId={projectId}
              sessionId={sessionId}
              onCopyToShared={setCopyToSharedPaths}
              onArtifactDeleted={() => {
                void queryClient.invalidateQueries({ queryKey: ['messages', sessionId] });
              }}
            />
          ))}
          {streaming && (
            <div className="cw-live"><span />AI 답변 중…</div>
          )}
          <div ref={messagesEndRef} />
        </div>

        <form className="cw-composer" onSubmit={(e) => { e.preventDefault(); void send(); }}>
          {pendingAttachments.length > 0 && (
            <div className="cw-attach-tray" style={{ display: 'flex', flexWrap: 'wrap', gap: 6, padding: '6px 0', gridColumn: '1 / -1' }}>
              {pendingAttachments.map((a) => (
                <AttachmentChip
                  key={a.tempId}
                  filename={a.filename}
                  status={a.status}
                  error={a.error}
                  onRemove={() => setPendingAttachments((prev) => prev.filter((x) => x.tempId !== a.tempId))}
                />
              ))}
            </div>
          )}
          <div className="cw-composer-box">
            <input
              value={composerText}
              onChange={(e) => setComposerText(e.target.value)}
              placeholder="Message Cowork and the team…"
              disabled={streaming}
            />
            <button
              type="button"
              className="cw-attach-btn"
              aria-label="파일 첨부"
              onClick={() => fileInputRef.current?.click()}
              disabled={streaming}
              style={{ width: 30, height: 30, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', border: 0, borderRadius: '50%', background: 'transparent', color: 'var(--cw-ink-3)', cursor: 'pointer', flexShrink: 0 }}
            >
              <Icon name="paperclip" size={13} />
            </button>
            <button type="submit" className="cw-send-button" aria-label="Send" disabled={!composerText.trim() || streaming || hasUploadingAttachments}>
              <Icon name="send" size={12} />
            </button>
          </div>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            style={{ display: 'none' }}
            onChange={(e) => void handleFileSelect(e)}
          />
          <small>Enter to send · Reference files with @filename</small>
        </form>
      </section>

      <aside className="cw-session-side">
        <h3>Members</h3>
        {userList.map((user) => (
          <div className="cw-side-row" key={user.id}>
            <Avatar user={user} small />
            {user.name}
          </div>
        ))}
        <h3>Referenced files</h3>
        {sess?.references.length
          ? <p style={{ fontFamily: 'var(--cw-font-mono)', fontSize: 11 }}>{sess.references.join(', ')}</p>
          : <p>No pinned files yet.</p>}
        <h3>Access</h3>
        {sess && <SharePill mode={sess.shareMode} />}
        {sess && <p>{shareMeta[sess.shareMode].desc}</p>}
        <h3>Session</h3>
        <p style={{ fontFamily: 'var(--cw-font-mono)', fontSize: 10.5, color: 'var(--cw-ink-4)' }}>{sessionId}</p>
        <p style={{ fontFamily: 'var(--cw-font-mono)', fontSize: 10.5, color: 'var(--cw-ink-4)' }}>
          project · {project.data?.name ?? '...'}
        </p>
        <ArtifactsPanel
          projectId={projectId}
          sessionId={sessionId}
          onCopyToShared={(paths) => setCopyToSharedPaths(paths)}
        />
      </aside>

      {copyToSharedPaths !== null && (
        <CopyToSharedDialog
          open={copyToSharedPaths !== null}
          projectId={projectId}
          sessionId={sessionId}
          sourcePaths={copyToSharedPaths}
          onClose={() => setCopyToSharedPaths(null)}
          onDone={() => {
            setCopyToSharedPaths(null);
            void queryClient.invalidateQueries({ queryKey: ['dirents', 'shared', projectId] });
          }}
        />
      )}
    </div>
  );
}

function ArtifactChip({
  path,
  projectId,
  sessionId,
  onCopyToShared,
  onDeleted,
}: {
  path: string;
  projectId: string;
  sessionId: string;
  onCopyToShared: (paths: string[]) => void;
  onDeleted: (path: string) => void;
}) {
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const chipRef = useRef<HTMLDivElement>(null);
  const scope: DirentScope = { kind: 'artifacts', projectId, sessionId };
  const filename = path.split('/').pop() ?? path;

  useEffect(() => {
    if (!menuOpen) return;
    function onPtr(e: PointerEvent) {
      if (chipRef.current && !chipRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    }
    document.addEventListener('pointerdown', onPtr);
    return () => document.removeEventListener('pointerdown', onPtr);
  }, [menuOpen]);

  const deleteMutation = useMutation({
    mutationFn: () => deleteDirent(scope, path),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['dirents', 'artifacts', projectId, sessionId] });
      onDeleted(path);
      showToast('삭제되었습니다');
      setConfirmDelete(false);
    },
    onError: () => showToast('삭제 실패'),
  });

  return (
    <>
      <div
        ref={chipRef}
        className="cw-attach-chip cw-artifact-chip"
        style={{ cursor: 'pointer', position: 'relative' }}
        onClick={() => setMenuOpen(prev => !prev)}
      >
        <FileTypeIcon filename={filename} size={14} />
        <span className="cw-attach-name">{filename}</span>
        {menuOpen && (
          <ul
            className="cw-file-dropdown"
            style={{ top: '100%', left: 0, marginTop: 4 }}
            onClick={(e) => { e.stopPropagation(); setMenuOpen(false); }}
          >
            <li>
              <button type="button" onClick={() => downloadFile(scope, path)}>
                <Icon name="download" size={13} /> 다운로드
              </button>
            </li>
            <li>
              <button type="button" onClick={() => onCopyToShared([path])}>
                <Icon name="file" size={13} /> 공유 디렉토리로 복사
              </button>
            </li>
            <li>
              <button
                type="button"
                className="cw-file-dropdown-destructive"
                onClick={() => setConfirmDelete(true)}
              >
                <Icon name="trash" size={13} /> 삭제
              </button>
            </li>
          </ul>
        )}
      </div>
      {confirmDelete && (
        <ConfirmDialog
          title="삭제 확인"
          body={`"${filename}"을(를) 삭제하시겠습니까?`}
          confirmLabel="삭제"
          destructive
          pending={deleteMutation.isPending}
          onConfirm={() => deleteMutation.mutate()}
          onClose={() => setConfirmDelete(false)}
        />
      )}
    </>
  );
}

function extractSubagentQuery(args: unknown): string {
  if (typeof args === 'string') {
    try {
      const parsed = JSON.parse(args) as unknown;
      if (parsed && typeof parsed === 'object') {
        const vals = Object.values(parsed as Record<string, unknown>);
        const first = vals[0];
        if (typeof first === 'string') return first;
      }
    } catch { /* ignore */ }
    return args;
  }
  if (args && typeof args === 'object') {
    const vals = Object.values(args as Record<string, unknown>);
    const first = vals[0];
    if (typeof first === 'string') return first;
  }
  return '';
}

function MessageBubble({
  message,
  users,
  currentUserId,
  artifactPaths,
  projectId,
  sessionId,
  onCopyToShared,
  onArtifactDeleted,
}: {
  message: Message;
  users: User[];
  currentUserId: string;
  artifactPaths?: string[];
  projectId?: string;
  sessionId?: string;
  onCopyToShared?: (paths: string[]) => void;
  onArtifactDeleted?: (path: string) => void;
}) {
  const isAi = message.sender.kind === 'agent';
  const isSelf = message.sender.kind === 'user' && message.sender.userId === currentUserId;

  const displayUser: User = isAi
    ? (users.find((u) => u.id === 'ai') ?? AI_USER)
    : (users.find((u) => u.id === (message.sender as { userId: string }).userId)
      ?? { id: 'unknown', name: 'Member', roleLabel: 'Member', avatar: 'M', color: 'var(--cw-ink-3)' });

  const isStreaming = message.status === 'streaming';
  const timeLabel = formatMessageTime(message.createdAt);
  const agentLabel = isAi ? (message.sender as { name: string }).name : null;

  const isLive = message.id.startsWith('live-');

  return (
    <article className={`cw-message ${isAi ? 'is-ai' : isSelf ? 'is-self' : 'is-other'}${isLive ? ' is-entering' : ''}`}>
      {isAi ? <span className="cw-ai-chip">AI</span> : <Avatar user={displayUser} />}
      <div className="cw-message-body">
        <div className="cw-message-meta">
          <b>{isSelf ? `${displayUser.name.split(' ')[0]} · 나` : isAi ? (agentLabel ?? 'AI') : displayUser.name.split(' ')[0]}</b>
          <time dateTime={message.createdAt} data-tooltip={formatMessageTimeFull(message.createdAt)}>{timeLabel}</time>
        </div>
        <div className={isAi ? 'cw-ai-prose' : 'cw-message-bubble'}>
          {isAi
            ? <MarkdownRenderer text={message.body} />
            : message.body.split('\n').map((line, i) => <p key={`${message.id}-${i}`}>{line || ' '}</p>)}
        </div>
        {message.attachments && message.attachments.length > 0 && (
          <div className="cw-msg-attachments" style={{ display: 'flex', flexWrap: 'wrap', gap: 8, marginTop: 6 }}>
            {message.attachments.map((path) => (
              <AttachmentPreview key={path} globalPath={path} />
            ))}
          </div>
        )}
        {isAi && message.toolCalls?.map((tc) =>
          tc.name.startsWith(SUBAGENT_PREFIX) ? (
            <div key={tc.id} className="cw-subagent-call">
              <span className="cw-subagent-mention">@{stripSubagentPrefix(tc.name)}</span>
              {' '}{extractSubagentQuery(tc.arguments)}
            </div>
          ) : (
            <details key={tc.id} className="cw-toolcall">
              <summary>🔧 {tc.name}{tc.result === undefined && isStreaming ? ' · 실행 중…' : ''}</summary>
              {tc.arguments !== undefined && (
                <pre className="cw-toolcall-args">{typeof tc.arguments === 'string'
                  ? tc.arguments
                  : JSON.stringify(tc.arguments, null, 2)}</pre>
              )}
              {tc.result !== undefined && <pre className="cw-toolcall-result">{tc.result}</pre>}
            </details>
          )
        )}
        {isAi && artifactPaths && artifactPaths.length > 0 && projectId && sessionId && onCopyToShared && onArtifactDeleted && (
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginTop: 10 }}>
            {artifactPaths.map((path) => (
              <ArtifactChip
                key={path}
                path={path}
                projectId={projectId}
                sessionId={sessionId}
                onCopyToShared={onCopyToShared}
                onDeleted={onArtifactDeleted}
              />
            ))}
          </div>
        )}
        {isAi && message.status === 'done' && (
          <div className="cw-ai-actions">
            <button>Copy</button>
            <button>Regenerate</button>
            <button>Good</button>
          </div>
        )}
      </div>
    </article>
  );
}
