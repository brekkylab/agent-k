// Session — markup mirrors app-live SessionPage. Chat surface (head + messages
// + composer) + right side (members, references, access, artifact).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createFileRoute, useLocation, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { localizedNoun } from '@/i18n';
import { getSession, updateSessionShareMode } from '@/api/sessions';
import { listMessages, sendMessage, stopRun, getRunActive, deriveStreamState } from '@/api/messages';
import { appWs } from '@/api/ws';
import type { AppWsEvent } from '@/api/ws';
import type { MessageOutput } from '@/api/backend-types';
import { getProject, listMembers } from '@/api/projects';
import { deleteDirent, downloadFile, scopeRoot, uploadFiles, type DirentScope } from '@/api/dirents';
import { Icon } from '@/components/Icon';
import { Avatar, IconButton, SharePill, ShareSelect } from '@/components/uiPrimitives';
import { getAgentSurface, type AgentId } from '@/domain/agentSurfaces';
import { getModelCatalog, modelLabel } from '@/api/models';
import { useAuthStore } from '@/stores/auth';
import { useToastStore } from '@/components/Toast';
import { MarkdownRenderer } from '@/components/chat/MarkdownRenderer';
import { AI_USER, SUBAGENT_PREFIX } from '@/api/transformers';
import { formatMessageTime, formatMessageTimeFull } from '@/lib/formatMessageTime';
import type { Message, Session, ShareMode, User } from '@/domain/types';
import { ApiError } from '@/api/client';
import { SessionTitleText } from '@/components/SessionTitleText';
import { ArtifactsPanel } from '@/components/ArtifactsPanel';
import { CopyToSharedDialog } from '@/components/CopyToSharedDialog';
import { AttachmentChip } from '@/components/AttachmentChip';
import { AttachmentPreview } from '@/components/AttachmentPreview';
import { FilePreviewModal } from '@/components/FilePreviewModal';
import { FileTypeIcon } from '@/components/FileTypeIcon';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { loadNs } from '@/i18n/loader';
import { useDuplicateSession } from '@/lib/useDuplicateSession';
import { shortSessionId } from '@/lib/sessionId';
import { resolveComposerKeyAction } from '@/lib/composerKeys';

export const Route = createFileRoute('/_app/projects/$projectSlug/sessions/$sessionPrefix')({
  // CopyToSharedDialog + ConfirmDialog mounted inside → `dialogs`.
  loader: () => loadNs('session', 'dialogs'),
  component: SessionPage,
});

function stripSubagentPrefix(name: string): string {
  return name.startsWith(SUBAGENT_PREFIX) ? name.slice(SUBAGENT_PREFIX.length) : name;
}

function SessionPage() {
  const { projectSlug, sessionPrefix } = Route.useParams();
  const { t } = useTranslation(['session', 'common']);
  const navigate = useNavigate();
  const location = useLocation();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const currentUser = useAuthStore((s) => s.currentUser);

  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
  const session = useQuery({ queryKey: ['session', sessionPrefix], queryFn: () => getSession(sessionPrefix) });
  const members = useQuery({ queryKey: ['members', projectSlug], queryFn: () => listMembers(projectSlug) });
  // Catalog (with this project's chains) to label the session's effective model.
  const catalog = useQuery({
    queryKey: ['models', projectSlug],
    queryFn: () => getModelCatalog(projectSlug),
    staleTime: 5 * 60_000,
  });

  // The project/session queries key off the URL slug + prefix, but everything
  // session-scoped (messages, dirent scopes, attachments, artifacts) keys off the
  // resolved UUIDs so every component — this page, ArtifactsPanel, MessageBubble —
  // shares one canonical key. Empty until the queries resolve; UUID-keyed queries
  // are gated on session.data.
  const projectId = project.data?.id ?? '';
  const sessionId = session.data?.id ?? '';

  const history = useQuery({
    queryKey: ['messages', sessionId],
    queryFn: () => listMessages(sessionId),
    enabled: Boolean(session.data && currentUser),
    // Refetch on every entry so the backend's mark-read side effect runs, which
    // clears the sidebar unread badge via the dataUpdatedAt effect below. Without
    // this, the QueryClient's default 30s staleTime kept re-entries within that
    // window from triggering mark-read and the badge stayed visible.
    staleTime: 0,
  });

  // Agent chip — transient preview state from home. Persist it after router state
  // is cleared, but reset it when this route instance moves to another session.
  const [agentPreview, setAgentPreview] = useState<{ sessionPrefix: string; agentId?: AgentId }>(() => ({
    sessionPrefix,
    agentId: location.state.initialAgentId,
  }));
  if (agentPreview.sessionPrefix !== sessionPrefix) {
    setAgentPreview({ sessionPrefix, agentId: location.state.initialAgentId });
  } else if (location.state.initialAgentId && agentPreview.agentId !== location.state.initialAgentId) {
    setAgentPreview({ sessionPrefix, agentId: location.state.initialAgentId });
  }
  // Prefer the session's stored agent_type (authoritative); fall back to the
  // router-state preview only before the session has loaded (smooth from home).
  const activeAgent = session.data?.agentType
    ? getAgentSurface(session.data.agentType)
    : agentPreview.agentId
      ? getAgentSurface(agentPreview.agentId)
      : undefined;

  // Auto-send initial message refs — useRef instead of module-level Set so each
  // component instance tracks its own state (StrictMode-safe, no cross-session leak).
  const initialMessageRef = useRef<string | null>(location.state.initialMessage ?? null);
  const consumedInitialMessageRef = useRef(false);

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  // Set when the local user sends a message — the next scroll effect jumps to
  // the bottom instantly, regardless of how far up the user has scrolled.
  const forceScrollRef = useRef(false);
  // Previous message count, to distinguish "new message arrived" from the
  // initial history load / refetch (no pill on first render of a session).
  const prevMsgCountRef = useRef(0);

  // WS-driven: outputs map keyed by seq (for idempotent catch-up + live merging)
  const wsOutputsRef = useRef<Map<number, MessageOutput>>(new Map());
  // Optimistic user bubble ID (decides replacement when agent_run_started arrives)
  const optimisticUserIdRef = useRef<string | null>(null);
  // Fires after a stretch of WS silence to reconcile run state with the server.
  const wsInactivityTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Bumped on every (re)arm so a stale in-flight check can't act on an old run.
  const wsWatchdogGenRef = useRef(0);
  // Mirror of the `streaming` state, accessible from WS event handler closures
  // without adding `streaming` to the effect dependency array (which would
  // re-register the handler on every state transition).
  const streamingRef = useRef(false);
  // Tracks the highest seq seen; when a new seq exceeds this, outputs are
  // already in insertion order and the sort can be skipped.
  const maxSeqRef = useRef<number>(-1);
  // [B] run_id tracking for race-condition guards.
  // currentRunIdRef: run_id of the active run being tracked on this client.
  // doneRunIdsRef: set of run_ids we've already processed a Done for.
  const currentRunIdRef = useRef<string | null>(null);
  const doneRunIdsRef = useRef<Set<string>>(new Set());

  const [composerText, setComposerText] = useState('');
  // Composer layout — false = compact pill (textarea + buttons inline),
  // true = stacked layout (textarea on top, actions row below). We flip to
  // expanded the moment text would wrap to a second row in compact mode and
  // collapse back only when the field empties, so partial edits don't
  // oscillate between layouts.
  const [composerExpanded, setComposerExpanded] = useState(false);
  const [liveMessages, setLiveMessages] = useState<Message[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [ownedRunId, setOwnedRunId] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  // True when the server confirms the run is still alive despite WS silence.
  const [runDelayed, setRunDelayed] = useState(false);
  // "New message arrived while scrolled up" pill above the composer.
  const [showNewMsgPill, setShowNewMsgPill] = useState(false);
  // Ghost scroll-to-bottom button — visible whenever scrolled past the
  // auto-follow threshold, regardless of new messages.
  const [showScrollDown, setShowScrollDown] = useState(false);
  const [copyToShared, setCopyToShared] = useState<{ scope: DirentScope; paths: string[] } | null>(null);

  type PendingAttachment = {
    tempId: string;
    filename: string;
    status: 'uploading' | 'uploaded' | 'error';
    globalPath?: string;
    error?: string;
  };
  const [pendingAttachments, setPendingAttachments] = useState<PendingAttachment[]>([]);

  // Reset when switching sessions — done during render (not in an effect) so React
  // StrictMode's double-invoked effects can't wipe the optimistic user/ai bubbles
  // the initial-send effect adds on entry (React's "adjust state on prop change").
  const [trackedPrefix, setTrackedPrefix] = useState(sessionPrefix);
  if (trackedPrefix !== sessionPrefix) {
    setTrackedPrefix(sessionPrefix);
    setLiveMessages([]);
    setComposerText('');
    setStreaming(false);
    streamingRef.current = false;
    setOwnedRunId(null);
    setStopping(false);
    setRunDelayed(false);
    if (wsInactivityTimerRef.current) {
      clearTimeout(wsInactivityTimerRef.current);
      wsInactivityTimerRef.current = null;
    }
    setPendingAttachments([]);
    setShowNewMsgPill(false);
    setShowScrollDown(false);
    wsOutputsRef.current.clear();
    maxSeqRef.current = -1;
    optimisticUserIdRef.current = null;
    currentRunIdRef.current = null;
    doneRunIdsRef.current.clear();
    forceScrollRef.current = false;
    prevMsgCountRef.current = 0;
  }

  // After messages load, mark-read side effect has run on the backend — zero this
  // session's unread badge directly in every cached session list (any origin filter)
  // instead of refetching whole lists on each session entry.
  useEffect(() => {
    if (history.isSuccess && sessionId) {
      queryClient.setQueriesData<Session[]>({ queryKey: ['sessions', projectSlug] }, (old) => {
        if (!old?.some((s) => s.id === sessionId && s.unreadCount > 0)) return old;
        return old.map((s) => (s.id === sessionId ? { ...s, unreadCount: 0 } : s));
      });
    }
  }, [history.isSuccess, history.dataUpdatedAt, projectSlug, sessionId, queryClient]);

  const allMessages = useMemo<Message[]>(() => [
    ...(history.data ?? []),
    ...liveMessages,
  ], [history.data, liveMessages]);

  // Total rendered-content size. Streaming updates grow a live bubble in place
  // (body text, tool calls, tool results) without changing the message count,
  // so the auto-follow effect below also keys off this to keep re-checking
  // while content grows. `+ 1` per tool call counts its appearance; results
  // count by length so their arrival is visible even on short outputs.
  const contentVersion = useMemo(
    () =>
      allMessages.reduce(
        (n, m) =>
          n +
          m.body.length +
          (m.toolCalls?.reduce((a, tc) => a + 1 + (tc.result?.length ?? 0), 0) ?? 0),
        0,
      ),
    [allMessages],
  );

  useEffect(() => {
    const el = scrollContainerRef.current;
    if (!el) return;
    const prevCount = prevMsgCountRef.current;
    prevMsgCountRef.current = allMessages.length;

    // Own send — jump to the bottom immediately, even if scrolled far up.
    if (forceScrollRef.current) {
      forceScrollRef.current = false;
      messagesEndRef.current?.scrollIntoView({ behavior: 'instant' });
      setShowNewMsgPill(false);
      return;
    }

    // Initial history load (refresh / session entry) — start at the bottom.
    if (prevCount === 0) {
      messagesEndRef.current?.scrollIntoView({ behavior: 'instant' });
      return;
    }

    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    if (distanceFromBottom <= 700) {
      messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    } else if (allMessages.length > prevCount) {
      // Scrolled too far up to auto-follow — surface a "new message" pill instead.
      setShowNewMsgPill(true);
    }
  }, [allMessages.length, streaming, contentVersion]);

  // Track scroll position: dismiss the pill once the user is back near the
  // bottom, and toggle the ghost scroll-down button past the follow threshold.
  useEffect(() => {
    const el = scrollContainerRef.current;
    if (!el) return;
    const onScroll = () => {
      const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
      if (distanceFromBottom <= 40) {
        setShowNewMsgPill(false);
      }
      setShowScrollDown(distanceFromBottom > 700);
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => el.removeEventListener('scroll', onScroll);
  }, []);

  const scrollToBottom = useCallback(() => {
    setShowNewMsgPill(false);
    messagesEndRef.current?.scrollIntoView({ behavior: 'instant' });
  }, []);

  // Auto-grow the composer textarea + drive the compact↔expanded layout flip.
  // Reset to auto first so deleting lines shrinks the box; overflow stays hidden
  // until we hit the 200px cap to avoid a phantom scrollbar from sub-pixel
  // rounding.
  //
  // Layout trigger:
  //   - empty text  → compact (pill, inline actions)
  //   - while compact, if scrollHeight indicates the text just wrapped to a
  //     second row in the narrow inline-width context, flip to expanded.
  //   - once expanded, stay until the field empties (don't toggle back while
  //     the user is mid-edit; ChatGPT-style stability).
  useEffect(() => {
    const ta = composerRef.current;
    if (!ta) return;
    ta.style.height = 'auto';
    const next = Math.min(ta.scrollHeight, 200);
    ta.style.height = `${next}px`;
    ta.style.overflowY = ta.scrollHeight > 200 ? 'auto' : 'hidden';

    if (composerText.length === 0) {
      setComposerExpanded(false);
    } else if (!composerExpanded) {
      const cs = getComputedStyle(ta);
      const padTop = parseFloat(cs.paddingTop) || 0;
      const padBot = parseFloat(cs.paddingBottom) || 0;
      const lineHeight = parseFloat(cs.lineHeight) || 24;
      // +4px slack so sub-pixel rounding doesn't flip on a single-line value.
      if (ta.scrollHeight > lineHeight + padTop + padBot + 4) {
        setComposerExpanded(true);
      }
    }
  }, [composerText, composerExpanded]);

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

  // Clears all run/streaming state for a run that ended without a terminal WS event.
  const resetEndedRun = useCallback(() => {
    setRunDelayed(false);
    setStreaming(false);
    streamingRef.current = false;
    setOwnedRunId(null);
    setStopping(false);
    setLiveMessages([]);
    wsOutputsRef.current.clear();
    maxSeqRef.current = -1;
    currentRunIdRef.current = null;
    optimisticUserIdRef.current = null;
    if (sessionId) void queryClient.refetchQueries({ queryKey: ['messages', sessionId] });
  }, [sessionId, queryClient]);

  // After a stretch of WS silence, checks once whether the run is still active:
  // if so, surface a delay hint and refetch; if not, reconcile the ended run.
  // Continued re-checking (delayed / stopping) lives in the watchdog poll effect.
  const armWatchdog = useCallback((runId: string) => {
    if (wsInactivityTimerRef.current) clearTimeout(wsInactivityTimerRef.current);
    setRunDelayed(false);
    const gen = ++wsWatchdogGenRef.current;
    wsInactivityTimerRef.current = setTimeout(() => {
      wsInactivityTimerRef.current = null;
      void (async () => {
        let active: boolean;
        try {
          active = await getRunActive(sessionId, runId);
        } catch {
          return;
        }
        if (gen !== wsWatchdogGenRef.current) return;
        if (active) {
          setRunDelayed(true);
          void queryClient.refetchQueries({ queryKey: ['messages', sessionId] });
        } else {
          resetEndedRun();
        }
      })();
    }, 10_000);
  }, [sessionId, queryClient, resetEndedRun]);

  // While the run waits without WS events (delayed, or stopping in its grace),
  // poll the server so a lost terminal event still ends the run.
  useEffect(() => {
    if (!ownedRunId || !sessionId || (!runDelayed && !stopping)) return;
    const interval = stopping ? 3_000 : 10_000;
    const id = setInterval(() => {
      void (async () => {
        let active: boolean;
        try {
          active = await getRunActive(sessionId, ownedRunId);
        } catch {
          return;
        }
        if (!active) resetEndedRun();
      })();
    }, interval);
    return () => clearInterval(id);
  }, [runDelayed, stopping, ownedRunId, sessionId, resetEndedRun]);

  const send = useCallback(async (overrideText?: string) => {
    const text = (overrideText ?? composerText).trim();
    const uploading = pendingAttachments.some((a) => a.status === 'uploading');
    if (!text || !sessionId || streaming || uploading) return;

    const attachmentPaths = pendingAttachments
      .filter((a) => a.status === 'uploaded' && a.globalPath)
      .map((a) => a.globalPath!);

    setComposerText('');
    setPendingAttachments([]);

    // Optimistic user bubble (instant feedback until WS agent_run_started arrives)
    const nowIso = new Date().toISOString();
    const optimisticUserId = `live-user-${Date.now()}`;
    optimisticUserIdRef.current = optimisticUserId;
    const userMsg: Message = {
      id: optimisticUserId,
      sessionId: sessionPrefix,
      sender: { kind: 'user', userId: currentUser?.id ?? 'user' },
      createdAt: nowIso,
      body: text,
      status: 'done',
      attachments: attachmentPaths.length > 0 ? attachmentPaths : undefined,
    };
    forceScrollRef.current = true;
    setLiveMessages((prev) => [...prev, userMsg]);
    setStreaming(true);
    streamingRef.current = true;

    try {
      const ack = await sendMessage(sessionId, text, attachmentPaths.length > 0 ? attachmentPaths : undefined);
      if (!doneRunIdsRef.current.has(ack.run_id)) {
        setOwnedRunId(ack.run_id);
      }
      armWatchdog(ack.run_id);
    } catch (err) {
      // Send failed (network, 403, 423, etc.) — recover immediately
      if (wsInactivityTimerRef.current) {
        clearTimeout(wsInactivityTimerRef.current);
        wsInactivityTimerRef.current = null;
      }
      setRunDelayed(false);
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'send failed';
      showToast(t('toast.send_failed', { message: msg }));
      setStreaming(false);
      streamingRef.current = false;
      setOwnedRunId(null);
      setLiveMessages([]);
      wsOutputsRef.current.clear();
      optimisticUserIdRef.current = null;
    }
    // On success: the WS event (agent_run_done) handles completion. No finally block.
  }, [composerText, streaming, sessionPrefix, sessionId, currentUser, showToast, pendingAttachments, t, armWatchdog]);

  const stopGeneration = useCallback(async () => {
    if (!ownedRunId || !sessionId || stopping) return;
    if (wsInactivityTimerRef.current) {
      clearTimeout(wsInactivityTimerRef.current);
      wsInactivityTimerRef.current = null;
    }
    setRunDelayed(false);
    setStopping(true);
    const STOP_ATTEMPTS = 2;
    for (let attempt = 0; attempt < STOP_ATTEMPTS; attempt++) {
      try {
        await stopRun(sessionId, ownedRunId);
        return;
      } catch (err) {
        // 404 means the run already ended — reconcile instead of surfacing an error.
        if (err instanceof ApiError && err.status === 404) {
          resetEndedRun();
          return;
        }
        // Non-404: wait briefly and retry while attempts remain; once exhausted, treat the run as lost and reconcile.
        if (attempt < STOP_ATTEMPTS - 1) {
          await new Promise((resolve) => setTimeout(resolve, 1_000));
          continue;
        }
        const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'stop failed';
        showToast(t('toast.stop_failed', { message: msg }));
        resetEndedRun();
      }
    }
  }, [ownedRunId, sessionId, stopping, showToast, t, resetEndedRun]);

  const handleComposerKeyDown = useCallback((e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (resolveComposerKeyAction({ key: e.key, shiftKey: e.shiftKey, isComposing: e.nativeEvent.isComposing }) === 'send') {
      e.preventDefault();
      void send();
    }
  }, [send]);

  // Composer textarea is disabled while streaming, so listen on window for Esc.
  useEffect(() => {
    if (!streaming || !ownedRunId || stopping) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.isComposing) {
        e.preventDefault();
        void stopGeneration();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [streaming, ownedRunId, stopping, stopGeneration]);

  const duplicateMutation = useDuplicateSession(projectSlug);

  // Auto-send the first message handed over from the home composer via router state.
  // Deferred until session.data resolves so `sessionId` is non-empty when the
  // finally-block refetch runs. useRef guards ensure exactly one send per mount
  // (StrictMode-safe — refs survive the double-invoke cycle).
  useEffect(() => {
    if (consumedInitialMessageRef.current) return;
    if (!session.data) return;                        // wait for session UUID to resolve
    if (!initialMessageRef.current) return;
    consumedInitialMessageRef.current = true;
    const msg = initialMessageRef.current;
    initialMessageRef.current = null;
    // Clear router state so a hard refresh doesn't resend, but don't block send on it.
    void navigate({ replace: true, state: (prev) => ({ ...prev, initialMessage: undefined }) });
    void send(msg);
  }, [session.data, send, navigate]);

  // WS session subscription — subscribe/unsubscribe when sessionId changes.
  // subscribeSession is ref-counted, so a session the sidebar already subscribes to
  // won't send a fresh `subscribe` to the server (no catch-up). resyncSession forces
  // that re-request on every entry, so we receive either the in-progress run
  // (started + messages) or an idle sync.
  useEffect(() => {
    if (!sessionId) return;
    appWs.subscribeSession(sessionId);
    appWs.resyncSession(sessionId);
    return () => {
      appWs.unsubscribeSession(sessionId);
    };
  }, [sessionId]);

  // WS event handler — drives agent responses from WS events.
  useEffect(() => {
    if (!sessionId) return;

    const unsubscribe = appWs.subscribe((event: AppWsEvent) => {
      // Ignore events unrelated to this session
      if (!('session_id' in event) || event.session_id !== sessionId) return;

      if (event.type === 'agent_run_started') {
        const { run_id } = event;

        // [B] Sequence 1: if Done for this run already arrived, ignore the late Started.
        if (doneRunIdsRef.current.has(run_id)) return;

        armWatchdog(run_id);

        // [B] Sequence 2: only clear accumulated outputs when this is a genuinely new run.
        // If currentRunId already equals run_id, pre-start messages are already in
        // wsOutputsRef and should not be wiped (live arrived before replay started).
        const isNewRun = currentRunIdRef.current !== run_id;
        if (isNewRun) {
          wsOutputsRef.current.clear();
          maxSeqRef.current = -1;
        }
        currentRunIdRef.current = run_id;

        setStreaming(true);
        streamingRef.current = true;

        // Keep the optimistic user bubble if present; otherwise (another tab/user) add it from the event
        setLiveMessages((prev) => {
          // Skip duplicate `started` for the run we're already rendering — replay can
          // resend it (subscribe race, reconnect, resync) and would add a second bubble.
          if (!isNewRun && prev.some((m) => m.id === `live-ai-${sessionId}`)) return prev;
          const hasOptimistic = optimisticUserIdRef.current &&
            prev.some((m) => m.id === optimisticUserIdRef.current);
          if (hasOptimistic) {
            // Optimistic bubble already present — add only the AI bubble
            const nowIso = new Date().toISOString();
            return [...prev, {
              id: `live-ai-${sessionId}`,
              sessionId: sessionPrefix,
              sender: { kind: 'agent' as const, name: 'agent-k' },
              createdAt: nowIso,
              body: '',
              status: 'streaming' as const,
            }];
          } else {
            // Another tab/user: add the user bubble from event.user_message
            const { user_message } = event;
            const nowIso = new Date().toISOString();
            return [...prev,
              {
                id: `live-user-ws-${sessionId}`,
                sessionId: sessionPrefix,
                sender: { kind: 'user', userId: user_message.sender_user_id },
                createdAt: user_message.created_at ?? nowIso,
                body: user_message.content,
                status: 'done' as const,
                attachments: user_message.attachments.length > 0 ? user_message.attachments : undefined,
              },
              {
                id: `live-ai-${sessionId}`,
                sessionId: sessionPrefix,
                sender: { kind: 'agent' as const, name: 'agent-k' },
                createdAt: nowIso,
                body: '',
                status: 'streaming' as const,
              }
            ];
          }
        });
      }

      if (event.type === 'agent_message') {
        const { run_id, seq, output } = event;
        // [B] Claim the run on first message (Seq2: live message arrives before replay Started),
        // or ignore messages from a different run (stale broadcast vs. newer run).
        if (currentRunIdRef.current === null) {
          currentRunIdRef.current = run_id;
        } else if (run_id !== currentRunIdRef.current) {
          return;
        }
        armWatchdog(run_id);
        wsOutputsRef.current.set(seq, output);

        // Fast path: if seq is strictly greater than maxSeqRef (the common
        // in-order case), Map insertion order is already sorted — no sort needed.
        // Slow path: out-of-order delivery (catch-up replay race) triggers a
        // full sort. In both cases duplicates are idempotent via Map.set.
        const outOfOrder = seq <= maxSeqRef.current;
        if (seq > maxSeqRef.current) maxSeqRef.current = seq;

        const outputs = outOfOrder
          ? [...wsOutputsRef.current.entries()].sort(([a], [b]) => a - b).map(([, v]) => v)
          : [...wsOutputsRef.current.values()];
        const update = deriveStreamState(outputs, 'streaming');

        const aiId = `live-ai-${sessionId}`;
        setLiveMessages((prev) => {
          let next = prev.map((m) => {
            if (m.id !== aiId) return m;
            const updatedToolCalls = update.toolCalls.length > 0
              ? update.toolCalls.map((tc) => ({ ...tc }))
              : m.toolCalls;
            return { ...m, body: update.text, status: 'streaming' as const, toolCalls: updatedToolCalls };
          });
          // Upsert subagent bubbles
          for (const sub of update.subagentUpdates) {
            const subId = `live-sub-${sub.sourceAgent}`;
            const exists = next.some((m) => m.id === subId);
            const nowIso = new Date().toISOString();
            if (exists) {
              next = next.map((m) =>
                m.id === subId ? { ...m, body: sub.text, status: 'streaming' as const } : m,
              );
            } else {
              next = [...next, {
                id: subId,
                sessionId: sessionPrefix,
                sender: { kind: 'agent' as const, name: sub.sourceAgent },
                createdAt: nowIso,
                body: sub.text,
                status: 'streaming' as const,
              }];
            }
          }
          return next;
        });
      }

      if (event.type === 'agent_error') {
        const { run_id } = event;
        // [R2-3] Ignore stale errors from a previous run — same guard as agent_run_done.
        if (currentRunIdRef.current !== null && run_id !== currentRunIdRef.current) return;
        if (wsInactivityTimerRef.current) {
          clearTimeout(wsInactivityTimerRef.current);
          wsInactivityTimerRef.current = null;
        }
        setRunDelayed(false);
        showToast(t('toast.agent_error', { message: event.message }));
        setStreaming(false);
        streamingRef.current = false;
        setOwnedRunId(null);
        setStopping(false);
        setLiveMessages([]);
        wsOutputsRef.current.clear();
        maxSeqRef.current = -1;
        currentRunIdRef.current = null;
        optimisticUserIdRef.current = null;
      }

      if (event.type === 'agent_run_done') {
        const { run_id } = event;
        // [B] Track completed run_ids to guard against late agent_run_started (Sequence 1).
        doneRunIdsRef.current.add(run_id);
        // Ignore Done events for runs we're not currently tracking.
        if (currentRunIdRef.current !== null && run_id !== currentRunIdRef.current) return;

        if (wsInactivityTimerRef.current) {
          clearTimeout(wsInactivityTimerRef.current);
          wsInactivityTimerRef.current = null;
        }
        setRunDelayed(false);

        if (event.stopped) {
          showToast(t('toast.run_stopped'));
        }

        // Done: refetch history → clear liveMessages → stop streaming → invalidate
        void (async () => {
          if (sessionId) {
            await queryClient.refetchQueries({ queryKey: ['messages', sessionId] });
          }
          setLiveMessages([]);
          wsOutputsRef.current.clear();
          maxSeqRef.current = -1;
          currentRunIdRef.current = null;
          optimisticUserIdRef.current = null;
          setStreaming(false);
          streamingRef.current = false;
          setOwnedRunId(null);
          setStopping(false);
          void queryClient.invalidateQueries({ queryKey: ['session', sessionPrefix] });
          void queryClient.invalidateQueries({ queryKey: ['sessions', projectSlug] });
          void queryClient.invalidateQueries({ queryKey: ['dirents', 'artifacts', projectId, sessionId] });
        })();
      }

      if (event.type === 'agent_run_idle') {
        // The server has no active run for this session (completed before we
        // subscribed, or server restarted). resyncSession re-requests this on every
        // entry, so refetch history once to surface any persisted messages and reset
        // streaming state — covers navigating away mid-run and returning after done.
        if (wsInactivityTimerRef.current) {
          clearTimeout(wsInactivityTimerRef.current);
          wsInactivityTimerRef.current = null;
        }
        setRunDelayed(false);
        void queryClient.refetchQueries({ queryKey: ['messages', sessionId] });
        setLiveMessages([]);
        wsOutputsRef.current.clear();
        maxSeqRef.current = -1;
        optimisticUserIdRef.current = null;
        currentRunIdRef.current = null;
        setStreaming(false);
        streamingRef.current = false;
        setOwnedRunId(null);
        setStopping(false);
      }
    });

    return () => {
      unsubscribe();
      if (wsInactivityTimerRef.current) {
        clearTimeout(wsInactivityTimerRef.current);
        wsInactivityTimerRef.current = null;
      }
    };
  }, [sessionId, sessionPrefix, projectSlug, projectId, queryClient, showToast, t, armWatchdog]);

  const shareMutation = useMutation({
    mutationFn: (mode: ShareMode) => updateSessionShareMode(sessionPrefix, mode),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['session', sessionPrefix] });
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectSlug] });
      showToast(t('toast.share_mode_changed'));
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'update failed';
      showToast(t('toast.share_change_failed', { message: msg }));
    },
  });

  const sess = session.data;
  const userList = members.data ?? [];
  const creator = userList.find((u) => u.id === sess?.creatorId);
  const usersForRender: User[] = [...userList, AI_USER];
  const hasUploadingAttachments = pendingAttachments.some((a) => a.status === 'uploading');
  const sessionForkable = !streaming && allMessages.length > 0;

  // `sessions.model` is authoritative: an explicit pin, or — for a recommended
  // session — the model materialized at first build (see build_session_agent).
  // So just label it; catalog only maps the id → display name. (NULL only in the
  // brief pre-build window of an unsent session → falls back to the default text.)
  const sessionModelLabel = sess?.model ? modelLabel(catalog.data, sess.model) : undefined;
  // A pinned model whose provider key is gone still shows here, but the backend
  // silently runs a fallback at build time. The session carries `modelAvailable`
  // (judged server-side for any id, catalogued or not) so we can flag that
  // `model` isn't what actually runs. The catalog is still used only to label.
  const sessionModelUnavailable = !!sess?.model && sess.modelAvailable === false;

  return (
    <div className="cw-session-layout cw-page-enter">
      <section className="cw-chat-surface">
        <div className="cw-chat-head">
          <div>
            <div className="cw-session-title-row">
              <h1><SessionTitleText title={sess?.title ?? '...'} /></h1>
              {activeAgent && (
                <span
                  className="cw-session-agent-chip"
                  data-agent={activeAgent.id}
                  title={t('chat.agent_chip')}
                >
                  <Icon name={activeAgent.icon} size={13} />
                  <span>{activeAgent.label}</span>
                </span>
              )}
            </div>
            <p>
              {creator && <>{t('chat.started_by')} <Avatar user={creator} small /> {creator.name} · </>}
              {t('chat.files_count', { count: sess?.references.length ?? 0 })} ·{' '}
              <Avatar user={AI_USER} small />{' '}
              <span
                className={sessionModelUnavailable ? 'cw-session-model-unavailable' : undefined}
                title={sessionModelUnavailable ? t('chat.model_unavailable') : t('chat.model_in_use')}
              >
                {sessionModelLabel ?? t('chat.default_label')}
                {sessionModelUnavailable && t('chat.model_unavailable_suffix')}
              </span>
            </p>
          </div>
          <div className="cw-session-head-actions">
            {sess && (
              <IconButton
                icon="sticky-notes"
                label="세션 복제"
                title={
                  !sessionForkable
                    ? (streaming ? '응답 생성이 끝난 뒤 복제할 수 있습니다' : '메시지가 있는 세션만 복제할 수 있습니다')
                  : duplicateMutation.isPending ? '복제 중...'
                  : 'Duplicate session'
                }
                expandedText={duplicateMutation.isPending ? '복제 중...' : '세션 복제'}
                confirmText="한 번 더 눌러 복제"
                disabled={!sessionForkable || duplicateMutation.isPending}
                onClick={() => duplicateMutation.mutate(sessionId, {
                  onSuccess: (newSession) => {
                    navigate({
                      to: '/projects/$projectSlug/sessions/$sessionPrefix',
                      params: { projectSlug, sessionPrefix: shortSessionId(newSession.id) },
                    });
                  },
                })}
              />
            )}
            {sess && (
              <ShareSelect mode={sess.shareMode} onChange={(mode) => shareMutation.mutate(mode)} />
            )}
          </div>
        </div>

        <div className="cw-messages-scroll" ref={scrollContainerRef}>
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
              onCopyToShared={(scope, paths) => setCopyToShared({ scope, paths })}
              onArtifactDeleted={() => {
                void queryClient.invalidateQueries({ queryKey: ['messages', sessionId] });
              }}
            />
          ))}
          {streaming && (
            <div className={stopping ? 'cw-live is-stopping' : runDelayed ? 'cw-live is-delayed' : 'cw-live'}><span />{stopping ? t('ui.stopping') : runDelayed ? t('ui.response_delayed') : t('ui.ai_responding')}</div>
          )}
          <div ref={messagesEndRef} />
        </div>
        </div>

        {(showNewMsgPill || showScrollDown) && (
          <div className="cw-new-msg-anchor">
            {showNewMsgPill && (
              <button type="button" className="cw-new-msg-pill" onClick={scrollToBottom}>
                <Icon name="chevron" size={12} />
                {t('ui.new_message')}
              </button>
            )}
            {showScrollDown && (
              <button
                type="button"
                className="cw-scroll-down-btn"
                aria-label={t('ui.scroll_to_bottom')}
                title={t('ui.scroll_to_bottom')}
                onClick={scrollToBottom}
              >
                <Icon name="chevron" size={14} />
              </button>
            )}
          </div>
        )}

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
          <div className="cw-composer-box" data-expanded={composerExpanded ? 'true' : 'false'}>
            <textarea
              ref={composerRef}
              value={composerText}
              onChange={(e) => setComposerText(e.target.value)}
              onKeyDown={handleComposerKeyDown}
              placeholder={t('ui.composer_placeholder')}
              disabled={streaming}
              rows={1}
            />
            <div className="cw-composer-actions">
              <button
                type="button"
                className="cw-attach-btn"
                aria-label={t('ui.attach_file')}
                onClick={() => fileInputRef.current?.click()}
                disabled={streaming}
                style={{ width: 30, height: 30, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', border: 0, borderRadius: '50%', background: 'transparent', color: 'var(--cw-ink-3)', cursor: 'pointer', flexShrink: 0 }}
              >
                <Icon name="paperclip" size={13} />
              </button>
              {streaming && ownedRunId ? (
                <button
                  type="button"
                  className="cw-send-button"
                  aria-label={t('ui.stop_generation')}
                  title={stopping ? t('ui.stopping') : t('ui.stop_generation')}
                  onClick={() => void stopGeneration()}
                  disabled={stopping}
                >
                  <Icon name="stop" size={12} />
                </button>
              ) : (
                <button type="submit" className="cw-send-button" aria-label={t('ui.send_aria')} disabled={!composerText.trim() || !sessionId || streaming || hasUploadingAttachments}>
                  <Icon name="send" size={12} />
                </button>
              )}
            </div>
          </div>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            style={{ display: 'none' }}
            onChange={(e) => void handleFileSelect(e)}
          />
          <small>{t('ui.composer_hint')}</small>
        </form>
      </section>

      <aside className="cw-session-side">
        <h3>{t('side.members')}</h3>
        {userList.map((user) => (
          <div className="cw-side-row" key={user.id}>
            <Avatar user={user} small />
            {user.name}
          </div>
        ))}
        <h3>{t('side.referenced_files')}</h3>
        {sess?.references.length
          ? <p style={{ fontFamily: 'var(--cw-font-mono)', fontSize: 11 }}>{sess.references.join(', ')}</p>
          : <p>{t('side.no_pinned_files')}</p>}
        <h3>{t('side.access')}</h3>
        {sess && <SharePill mode={sess.shareMode} />}
        {sess && <p>{t(`common:share.${sess.shareMode}.desc`)}</p>}
        <h3>{t('side.session')}</h3>
        <p style={{ fontFamily: 'var(--cw-font-mono)', fontSize: 10.5, color: 'var(--cw-ink-4)' }}>{sessionPrefix}</p>
        <p style={{ fontFamily: 'var(--cw-font-mono)', fontSize: 10.5, color: 'var(--cw-ink-4)' }}>
          {t('side.project_label', { name: project.data?.name ?? '...' })}
        </p>
        <ArtifactsPanel
          projectId={projectId}
          sessionId={sessionId}
          onCopyToShared={(scope, paths) => setCopyToShared({ scope, paths })}
        />
      </aside>

      {copyToShared !== null && (
        <CopyToSharedDialog
          open={copyToShared !== null}
          projectId={projectId}
          sourceScope={copyToShared.scope}
          sourcePaths={copyToShared.paths}
          onClose={() => setCopyToShared(null)}
          onDone={() => {
            setCopyToShared(null);
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
  onCopyToShared: (scope: DirentScope, paths: string[]) => void;
  onDeleted: (path: string) => void;
}) {
  const { t, i18n } = useTranslation('session');
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [previewing, setPreviewing] = useState(false);
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
      showToast(t('toast.artifact_deleted'));
      setConfirmDelete(false);
    },
    onError: () => showToast(t('toast.artifact_delete_failed')),
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
              <button type="button" onClick={() => setPreviewing(true)}>
                <Icon name="eye" size={13} /> {t('artifact.preview')}
              </button>
            </li>
            <li>
              <button type="button" onClick={() => downloadFile(scope, path)}>
                <Icon name="download" size={13} /> {t('artifact.download')}
              </button>
            </li>
            <li>
              <button type="button" onClick={() => onCopyToShared(scope, [path])}>
                <Icon name="file" size={13} /> {t('artifact.copy_to_shared')}
              </button>
            </li>
            <li>
              <button
                type="button"
                className="cw-file-dropdown-destructive"
                onClick={() => setConfirmDelete(true)}
              >
                <Icon name="trash" size={13} /> {t('artifact.delete')}
              </button>
            </li>
          </ul>
        )}
      </div>
      {confirmDelete && (
        <ConfirmDialog
          title={t('delete_artifact.title')}
          body={t('delete_artifact.body', { name: `"${localizedNoun(filename, '을/를', i18n.language)}"` })}
          confirmLabel={t('delete_artifact.confirm')}
          destructive
          pending={deleteMutation.isPending}
          onConfirm={() => deleteMutation.mutate()}
          onClose={() => setConfirmDelete(false)}
        />
      )}
      {previewing && (
        <FilePreviewModal globalPath={`${scopeRoot(scope)}/${path}`} onClose={() => setPreviewing(false)} />
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
  onCopyToShared?: (scope: DirentScope, paths: string[]) => void;
  onArtifactDeleted?: (path: string) => void;
}) {
  const { t } = useTranslation('session');
  const isAi = message.sender.kind === 'agent';
  const isSelf = message.sender.kind === 'user' && message.sender.userId === currentUserId;

  const displayUser: User = isAi
    ? (users.find((u) => u.id === 'ai') ?? AI_USER)
    : (users.find((u) => u.id === (message.sender as { userId: string }).userId)
      ?? { id: 'unknown', name: 'Member', roleLabel: 'Member', avatar: 'M', color: 'var(--cw-ink-3)', preferredLanguage: 'en' });

  const isStreaming = message.status === 'streaming';
  const timeLabel = formatMessageTime(message.createdAt);
  const agentLabel = isAi ? (message.sender as { name: string }).name : null;

  const isLive = message.id.startsWith('live-');

  return (
    <article className={`cw-message ${isAi ? 'is-ai' : isSelf ? 'is-self' : 'is-other'}${isLive ? ' is-entering' : ''}`}>
      {isAi ? <span className="cw-ai-chip">AI</span> : <Avatar user={displayUser} />}
      <div className="cw-message-body">
        <div className="cw-message-meta">
          <b>{isSelf ? `${displayUser.name.split(' ')[0]} · ${t('ui.self_label')}` : isAi ? (agentLabel ?? 'AI') : displayUser.name.split(' ')[0]}</b>
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
              <AttachmentPreview key={path} globalPath={path} onCopyToShared={onCopyToShared} />
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
              <summary>🔧 {tc.name}{tc.result === undefined && isStreaming ? ` · ${t('ui.tool_running')}` : ''}</summary>
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
