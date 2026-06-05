// Project Home — a "new conversation" surface. Browsing past sessions lives in the
// sidebar SESSIONS list + its "View all" overlay, not here.

import { useEffect, useState } from 'react';
import { createFileRoute, useLocation, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { getProject, listMembers } from '@/api/projects';
import { createSession } from '@/api/sessions';
import { getModelCatalog, recommendationFor } from '@/api/models';
import { AvatarStack } from '@/components/uiPrimitives';
import { ProjectHomeComposer, type ProjectHomeComposerSubmission } from '@/components/chat/ProjectHomeComposer';
import { ComposerModelPicker } from '@/components/chat/ComposerModelPicker';
import { ComposerAgentPicker } from '@/components/chat/ComposerAgentPicker';
import { DEFAULT_AGENT_ID, type AgentId, type SuggestedPrompt } from '@/domain/agentSurfaces';
import { useModelPrefsStore } from '@/stores/modelPrefs';
import { useToastStore } from '@/components/Toast';
import { shortSessionId } from '@/lib/sessionId';
import { ApiError } from '@/api/client';
import { loadNs } from '@/i18n/loader';

export const Route = createFileRoute('/_app/projects/$projectSlug/')({
  // Home composer + toasts live on `project`; `common` comes from parents.
  loader: () => loadNs('project', 'automation'),
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
    mutationFn: async (firstMessage: string) => {
      const session = await createSession(projectSlug, {
        agentType: selectedAgentId,
        model: selectedModel,
      });
      return { session, firstMessage };
    },
    onSuccess: async ({ session, firstMessage }) => {
      await queryClient.invalidateQueries({ queryKey: ['sessions', projectSlug] });
      setComposerText('');
      navigate({
        to: '/projects/$projectSlug/sessions/$sessionPrefix',
        params: { projectSlug, sessionPrefix: shortSessionId(session.id) },
        state: firstMessage
          ? { initialMessage: firstMessage, initialAgentId: selectedAgentId }
          : { initialAgentId: selectedAgentId },
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
    startSessionMutation.mutate(text);
  };

  const memberList = members.data ?? [];

  return (
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
            onAttachClick={() => showToast(t('home.attach_coming_soon'))}
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
  );
}
