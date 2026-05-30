// Project Home — a "new conversation" surface. Browsing past sessions lives in the
// sidebar SESSIONS list + its "View all" overlay, not here.

import { useEffect, useState } from 'react';
import { createFileRoute, useLocation, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getProject, listMembers } from '@/api/projects';
import { createSession } from '@/api/sessions';
import { AvatarStack } from '@/components/uiPrimitives';
import { ProjectHomeComposer, type ProjectHomeComposerSubmission } from '@/components/chat/ProjectHomeComposer';
import { ComposerModelPicker, DEFAULT_MODEL_ID, type ModelId } from '@/components/chat/ComposerModelPicker';
import { ComposerAgentPicker } from '@/components/chat/ComposerAgentPicker';
import { DEFAULT_AGENT_ID, getAgentSurface, type AgentId } from '@/domain/agentSurfaces';
import { useToastStore } from '@/components/Toast';
import { shortSessionId } from '@/lib/sessionId';
import { ApiError } from '@/api/client';

export const Route = createFileRoute('/_app/projects/$projectSlug/')({
  component: ProjectHome,
});

function ProjectHome() {
  const { projectSlug } = Route.useParams();
  const navigate = useNavigate();
  const location = useLocation();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
  const members = useQuery({ queryKey: ['members', projectSlug], queryFn: () => listMembers(projectSlug) });

  const [composerText, setComposerText] = useState('');
  const [selectedModelId, setSelectedModelId] = useState<ModelId>(DEFAULT_MODEL_ID);
  const [selectedAgentId, setSelectedAgentId] = useState<AgentId>(DEFAULT_AGENT_ID);
  const [focusNonce, setFocusNonce] = useState(0);

  const activeAgent = getAgentSurface(selectedAgentId);

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
      const session = await createSession(projectSlug);
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
      showToast(`세션 생성 실패: ${msg}`);
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
          {activeAgent.greeting}
        </p>

        <div
          className="cw-agent-composer-wrap"
          data-agent={selectedAgentId}
        >
          <div className="cw-agent-tabs-area">
            <ComposerAgentPicker value={selectedAgentId} onChange={setSelectedAgentId} />
            <span className="cw-preview-pill" aria-label="미리보기: 아직 서버에 전달되지 않습니다">Preview</span>
          </div>
          <ProjectHomeComposer
            value={composerText}
            onChange={setComposerText}
            onSubmit={handleSubmit}
            disabled={startSessionMutation.isPending}
            pending={startSessionMutation.isPending}
            placeholder={activeAgent.placeholder}
            focusSignal={focusNonce}
            onAttachClick={() => showToast('파일 추가 기능은 곧 추가됩니다.')}
            modelPicker={<ComposerModelPicker value={selectedModelId} onChange={setSelectedModelId} />}
          />
        </div>

        <div
          key={selectedAgentId}
          className="cw-suggested-prompts"
          aria-label="추천 프롬프트 (미리보기)"
        >
          {activeAgent.prompts.map((prompt) => (
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
