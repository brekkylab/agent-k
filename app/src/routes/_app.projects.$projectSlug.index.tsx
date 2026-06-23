// Project Home — a "new conversation" surface. Browsing past sessions lives in the
// sidebar SESSIONS list + its "View all" overlay, not here.

import { useEffect, useRef, useState } from 'react';
import { createFileRoute, useLocation, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { getProject, listMembers } from '@/api/projects';
import { createSession } from '@/api/sessions';
import { uploadFiles } from '@/api/dirents';
import { getModelCatalog, recommendationFor } from '@/api/models';
import { AvatarStack } from '@/components/uiPrimitives';
import { ProjectHomeComposer, type ProjectHomeComposerSubmission } from '@/components/chat/ProjectHomeComposer';
import { ComposerModelPicker } from '@/components/chat/ComposerModelPicker';
import { ComposerAgentPicker } from '@/components/chat/ComposerAgentPicker';
import { SharedFilePickerDialog } from '@/components/SharedFilePickerDialog';
import { type SessionImportItem } from '@/components/SharedFilesPanel';
import { DEFAULT_AGENT_ID, type AgentId, type SuggestedPrompt } from '@/domain/agentSurfaces';
import { useModelPrefsStore } from '@/stores/modelPrefs';
import { useToastStore } from '@/components/Toast';
import { shortSessionId } from '@/lib/sessionId';
import { useFileDropzone } from '@/lib/useFileDropzone';
import { MAX_ATTACHMENTS, MAX_UPLOAD_BYTES } from '@/domain/files';
import { ApiError } from '@/api/client';
import { loadNs } from '@/i18n/loader';

export const Route = createFileRoute('/_app/projects/$projectSlug/')({
  // Home composer + toasts live on `project`; `common` comes from parents.
  // `session` supplies the shared-file labels (import/added/…) the picker's column
  // view reuses, and AttachmentChip's remove label.
  loader: () => loadNs('project', 'automation', 'session'),
  component: ProjectHome,
});

function ProjectHome() {
  const { projectSlug } = Route.useParams();
  const { t } = useTranslation('project');
  const { t: tAgent } = useTranslation('automation');
  const navigate = useNavigate();
  const location = useLocation();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
  const members = useQuery({ queryKey: ['members', projectSlug], queryFn: () => listMembers(projectSlug) });
  // Catalog rarely changes (only when the server's configured providers do);
  // keep it fresh for a while so the picker doesn't refetch on every visit.
  const catalog = useQuery({
    queryKey: ['models', projectSlug],
    queryFn: () => getModelCatalog(projectSlug),
    staleTime: 5 * 60_000,
  });

  const [composerText, setComposerText] = useState('');
  // Local files staged on the home composer — uploaded to the new session's inputs/ on submit.
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  // Shared (server) files staged for attach — referenced by global path, no upload.
  const [pendingShared, setPendingShared] = useState<SessionImportItem[]>([]);
  // Synchronous mirror of pendingShared. addShared's cap/dedupe decision must see
  // additions made earlier in the same tick (a closure over the state value can't),
  // so it reads/advances this ref. Re-synced from state after every commit, so the
  // other mutation paths (remove/clear) keep it correct too.
  const pendingSharedRef = useRef<SessionImportItem[]>([]);
  useEffect(() => { pendingSharedRef.current = pendingShared; }, [pendingShared]);
  const [sharedPickerOpen, setSharedPickerOpen] = useState(false);
  const [selectedAgentId, setSelectedAgentId] = useState<AgentId>(DEFAULT_AGENT_ID);
  // Model selection is remembered per project + agent surface and persisted to
  // localStorage (see useModelPrefsStore): switching agents — or reloading —
  // restores that agent's last pick within this project. Missing = "recommended".
  const byProject = useModelPrefsStore((s) => s.byProject);
  const setModel = useModelPrefsStore((s) => s.setModel);
  const selectedModel = byProject[projectSlug]?.[selectedAgentId] ?? null;
  const setSelectedModel = (id: string | null) => setModel(projectSlug, selectedAgentId, id);
  const [focusNonce, setFocusNonce] = useState(0);

  // Agent copy (greeting/placeholder/suggested prompts) is i18n'd in the
  // `automation` namespace, keyed by the agent surface id.
  const agentGreeting = tAgent(`agent.${selectedAgentId}.greeting`);
  const agentPlaceholder = tAgent(`agent.${selectedAgentId}.placeholder`);
  const agentPrompts = tAgent(`agent.${selectedAgentId}.prompts`, {
    returnObjects: true,
  }) as SuggestedPrompt[];

  // The model that will actually run: an explicit pin, or what "recommended"
  // resolves to (a chain model, or the last-resort fallback). Send is blocked
  // only when that effective model has no configured provider — i.e. nothing
  // can run at all. The last-resort (Kimi) is a valid run target when its key
  // is set, so an available fallback does NOT block send. Judged once the
  // catalog has loaded.
  const rec = recommendationFor(catalog.data, selectedAgentId);
  const effectiveModel = selectedModel ?? rec?.resolvedModel;
  const effectiveAvailable =
    !!effectiveModel && !!catalog.data?.models.find((m) => m.id === effectiveModel)?.available;
  const sendBlocked = !!catalog.data && !effectiveAvailable;

  // The model pref seeds new sessions, so drop a pin that's no longer in the
  // catalog (saved before a catalog change): new sessions should fall back to
  // "recommended" rather than be created with an un-curated model. Runs once
  // the catalog has loaded; absent-from-catalog only — an in-catalog pin whose
  // provider key is missing is a deliberate choice and is left alone.
  useEffect(() => {
    if (!catalog.data || !selectedModel) return;
    if (!catalog.data.models.some((m) => m.id === selectedModel)) {
      setSelectedModel(null);
    }
  }, [catalog.data, selectedModel, selectedAgentId, projectSlug]);

  // Sidebar '+' navigates here with focusComposer: bump the focus nonce (so the
  // composer focuses even on a repeat '+'), then consume the signal so a refresh
  // doesn't re-focus.
  useEffect(() => {
    if (!location.state.focusComposer) return;
    setFocusNonce((n) => n + 1);
    void navigate({ replace: true, state: (prev) => ({ ...prev, focusComposer: undefined }) });
  }, [location.state.focusComposer, navigate]);

  // Submit creates a session and hands the first message + selected agent to the
  // session page via router state, where it auto-streams on entry.
  const startSessionMutation = useMutation({
    mutationFn: async (
      { firstMessage, files, shared }: { firstMessage: string; files: File[]; shared: SessionImportItem[] },
    ) => {
      const session = await createSession(projectSlug, {
        agentType: selectedAgentId,
        model: selectedModel,
      });
      // Shared files already live on the server, so just reference their global
      // paths. Local files are uploaded into the new session's inputs/ and
      // contribute their resulting paths. Both ride initialAttachments to the
      // auto-sent first message (shared first, then uploaded).
      let uploadedPaths: string[] = [];
      let failed: { name: string; reason: string }[] = [];
      // Use the created session's own projectId (always present) rather than the
      // project query, which may not have resolved yet — otherwise staged files
      // would be silently dropped when project.data is still loading.
      const projectId = session.projectId;
      if (files.length > 0) {
        const scope = { kind: 'inputs' as const, projectId, sessionId: session.id };
        try {
          const result = await uploadFiles(scope, files.map((file) => ({ file, targetPath: file.name })));
          uploadedPaths = result.succeeded.map((s) => s.path);
          // Per-file failures (e.g. over the size limit) carry their own reason.
          failed = result.failed.map((f) => ({ name: f.path, reason: f.error }));
        } catch (e) {
          // Upload threw entirely (network / multipart / access). The session was
          // already created, so rejecting the mutation would orphan it (and a retry
          // would create a duplicate). Instead keep the session: send the message
          // without the uploads and report every file with the shared failure reason.
          const reason = e instanceof ApiError ? e.message : e instanceof Error ? e.message : 'upload failed';
          failed = files.map((f) => ({ name: f.name, reason }));
        }
      }
      const attachmentPaths = [...shared.map((s) => s.globalPath), ...uploadedPaths];
      return { session, firstMessage, attachmentPaths, failed };
    },
    onSuccess: async ({ session, firstMessage, attachmentPaths, failed }) => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectSlug] });
      setComposerText('');
      setPendingFiles([]);
      setPendingShared([]);
      // Surface partial/total upload failures with the reason — the message still
      // sends with whatever uploaded, but don't let failed files vanish silently.
      // Each failure is a "name — reason" detail line; cap the list for a big batch.
      if (failed.length > 0) {
        const CAP = 5;
        const lines = failed.slice(0, CAP).map((f) => `${f.name} — ${f.reason}`);
        if (failed.length > CAP) lines.push(t('home.upload_failed_more', { count: failed.length - CAP }));
        showToast(t('home.upload_failed'), lines);
      }
      navigate({
        to: '/projects/$projectSlug/sessions/$sessionPrefix',
        params: { projectSlug, sessionPrefix: shortSessionId(session.id) },
        state: {
          // Attachments only ride along with a message — the session auto-sends on
          // `initialMessage`, so handing over attachments without one would just drop
          // them. Gate both together so that case can't arise.
          ...(firstMessage
            ? { initialMessage: firstMessage, ...(attachmentPaths.length > 0 ? { initialAttachments: attachmentPaths } : {}) }
            : {}),
          initialAgentId: selectedAgentId,
        },
        // Morph the composer to its session position/shape (shared view-transition-name).
        viewTransition: true,
      });
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'create failed';
      showToast(t('toast.session_create_failed', { message: msg }));
    },
  });

  const handleSubmit = ({ text }: ProjectHomeComposerSubmission) => {
    if (startSessionMutation.isPending) return;
    startSessionMutation.mutate({ firstMessage: text, files: pendingFiles, shared: pendingShared });
  };

  // Attach shared files picked from the dialog. Dedupe against what's already
  // staged; if the batch would push the combined total (uploads + shared) past
  // MAX_ATTACHMENTS, reject the whole batch (attach nothing) and toast — mirrors
  // the session's importSharedFiles so picking a 30+ folder behaves like not
  // picking it at all. Both the dedupe and the cap are decided against
  // pendingSharedRef (not the closure's state value) so two calls in the same tick
  // see each other's additions and can't blow past the cap; the toast stays out
  // here, not inside the updater, so it's a clean side effect rather than one fired
  // during render.
  const addShared = (items: SessionImportItem[]) => {
    if (items.length === 0) return;
    const cur = pendingSharedRef.current;
    const curPaths = new Set(cur.map((s) => s.globalPath));
    const fresh = items.filter((it) => !curPaths.has(it.globalPath));
    if (fresh.length === 0) return; // everything already attached
    if (pendingFiles.length + cur.length + fresh.length > MAX_ATTACHMENTS) {
      showToast(t('home.shared_picker.attach_limit', { max: MAX_ATTACHMENTS }));
      return; // over the cap — don't attach any of them
    }
    // Advance the ref synchronously so a same-tick repeat sees these additions; the
    // sync effect will re-affirm it from state after the commit.
    const next = [...cur, ...fresh];
    pendingSharedRef.current = next;
    setPendingShared(next);
  };

  // Cap staged files at MAX_ATTACHMENTS (clip + drag both land here). Reject the
  // whole batch if it would push over — mirrors the session's attach cap so the
  // first message can't exceed the backend's hard limit.
  const addFiles = (fs: File[]) => {
    if (fs.length === 0) return;
    // Drop files over the upload size limit at stage time (the backend would
    // reject them anyway; doing it here avoids a silent partial failure at send,
    // since the first message auto-sends on arrival). Skip the oversized ones and
    // stage the rest — matching the partial handling on the session/files surfaces.
    const tooBig = fs.filter((f) => f.size > MAX_UPLOAD_BYTES);
    const ok = fs.filter((f) => f.size <= MAX_UPLOAD_BYTES);
    if (tooBig.length > 0) {
      showToast(t('home.file_too_large', { max: Math.round(MAX_UPLOAD_BYTES / (1024 * 1024)) }), tooBig.map((f) => f.name));
    }
    if (ok.length === 0) return;
    if (pendingFiles.length + ok.length > MAX_ATTACHMENTS) {
      showToast(t('common:attach_limit', { max: MAX_ATTACHMENTS }));
      return;
    }
    setPendingFiles((prev) => [...prev, ...ok]);
  };
  // Drop computer files anywhere on the home page, mirroring the session's
  // full-surface drop zone — the whole page highlights uniformly.
  const composerDropzone = useFileDropzone({
    onFiles: addFiles,
    disabled: startSessionMutation.isPending,
  });

  const memberList = members.data ?? [];

  return (
    <div
      className={`cw-home-drop${composerDropzone.isOver ? ' is-drop-target' : ''}`}
      {...composerDropzone.dropProps}
    >
    <section className="cw-page cw-page-enter">
      <div className="cw-project-hero is-slim">
        <div>
          <h1>{project.data?.name ?? '...'}</h1>
        </div>
        <div className="cw-hero-actions">
          <AvatarStack users={memberList} />
        </div>
      </div>

      <div className="cw-home-blank">
        <p className="cw-home-greeting">
          {agentGreeting}
        </p>

        <div
          className="cw-agent-composer-wrap"
          data-agent={selectedAgentId}
        >
          <div className="cw-agent-tabs-area">
            <ComposerAgentPicker value={selectedAgentId} onChange={setSelectedAgentId} />
          </div>
          <ProjectHomeComposer
            value={composerText}
            onChange={setComposerText}
            onSubmit={handleSubmit}
            disabled={startSessionMutation.isPending}
            pending={startSessionMutation.isPending}
            sendBlocked={sendBlocked}
            sendBlockedHint={t('home.send_blocked_hint')}
            placeholder={agentPlaceholder}
            focusSignal={focusNonce}
            files={pendingFiles}
            onAddFiles={addFiles}
            onRemoveFile={(i) => setPendingFiles((prev) => prev.filter((_, idx) => idx !== i))}
            sharedFiles={pendingShared}
            onPickShared={project.data?.id ? () => setSharedPickerOpen(true) : undefined}
            onRemoveShared={(i) => setPendingShared((prev) => prev.filter((_, idx) => idx !== i))}
            modelPicker={
              <ComposerModelPicker
                catalog={catalog.data}
                agentType={selectedAgentId}
                value={selectedModel}
                onChange={setSelectedModel}
              />
            }
          />
        </div>

        <div
          key={selectedAgentId}
          className="cw-suggested-prompts"
          aria-label={t('home.suggested_prompts_aria')}
        >
          {agentPrompts.map((prompt) => (
            <button
              key={prompt.label}
              type="button"
              className="cw-suggested-chip"
              onClick={() => {
                setComposerText(prompt.seedText);
                setFocusNonce((n) => n + 1);
              }}
            >
              {prompt.label}
            </button>
          ))}
        </div>
      </div>
    </section>
    {sharedPickerOpen && project.data?.id && (
      <SharedFilePickerDialog
        projectId={project.data.id}
        projectName={project.data.name}
        onImport={addShared}
        staged={pendingShared}
        onRemove={(globalPath) => setPendingShared((prev) => prev.filter((s) => s.globalPath !== globalPath))}
        onClose={() => setSharedPickerOpen(false)}
      />
    )}
    </div>
  );
}
