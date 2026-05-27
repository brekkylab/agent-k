// Project Home — a "new conversation" surface. Browsing past sessions lives in the
// sidebar SESSIONS list + its "View all" overlay, not here.

import { useEffect, useState } from 'react';
import { createFileRoute, useLocation, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getProject, listMembers } from '@/api/projects';
import { createSession } from '@/api/sessions';
import { AvatarStack } from '@/components/uiPrimitives';
import { SessionComposer, type ComposerSubmission } from '@/components/chat/SessionComposer';
import { ComposerModelPicker, DEFAULT_MODEL_ID, type ModelId } from '@/components/chat/ComposerModelPicker';
import { ComposerAgentPicker, DEFAULT_AGENT_ID, getAgentOption, type AgentId } from '@/components/chat/ComposerAgentPicker';
import { useToastStore } from '@/components/Toast';
import { shortSessionId } from '@/lib/sessionId';
import { ApiError } from '@/api/client';

export const Route = createFileRoute('/_app/projects/$projectSlug/')({
  component: ProjectHome,
});

// Placeholder suggested prompts. `label` is the short chip text; `seedText` is what
// gets prefilled into the composer. In a follow-up PR each chip will also carry an
// `agentHint` (which agent suits the task) and submit via the ComposerSubmission
// envelope — defining the shape now keeps that seam real, not just a comment.
interface SuggestedPrompt {
  label: string;
  seedText: string;
  // future: agentHint?: string;
}

const SUGGESTED_PROMPTS: SuggestedPrompt[] = [
  { label: '프로젝트 파일 요약', seedText: '프로젝트 파일들을 요약해줘' },
  { label: '진행 상황 정리', seedText: '이번 주 진행 상황을 정리해줘' },
  { label: '리스크 찾기', seedText: '리스크와 블로커를 찾아줘' },
];

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

  const activeAgent = getAgentOption(selectedAgentId);

  // Sidebar '+' navigates here with focusComposer: bump the focus nonce (so the
  // composer focuses even on a repeat '+'), then consume the signal so a refresh
  // doesn't re-focus.
  useEffect(() => {
    if (!location.state.focusComposer) return;
    setFocusNonce((n) => n + 1);
    void navigate({ replace: true, state: (prev) => ({ ...prev, focusComposer: undefined }) });
  }, [location.state.focusComposer, navigate]);

  // Submit creates a session and hands the first message to the session page via
  // router state, where it auto-streams on entry.
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
        state: firstMessage ? { initialMessage: firstMessage } : undefined,
        // Morph the composer to its session position/shape (shared view-transition-name).
        viewTransition: true,
      });
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'create failed';
      showToast(`세션 생성 실패: ${msg}`);
    },
  });

  const handleSubmit = ({ text }: ComposerSubmission) => {
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
          Start a new conversation in {project.data?.name ?? 'this project'}
        </p>
        <ComposerAgentPicker value={selectedAgentId} onChange={setSelectedAgentId} />
        <SessionComposer
          value={composerText}
          onChange={setComposerText}
          onSubmit={handleSubmit}
          disabled={startSessionMutation.isPending}
          pending={startSessionMutation.isPending}
          size="large"
          placeholder={activeAgent.placeholder}
          focusSignal={focusNonce}
          onAttachClick={() => showToast('파일 추가 기능은 곧 추가됩니다.')}
          actionsSlot={<ComposerModelPicker value={selectedModelId} onChange={setSelectedModelId} />}
          belowSlot={
            <div className="cw-suggested-prompts" aria-label="추천 프롬프트 (미리보기)">
              {SUGGESTED_PROMPTS.map((prompt) => (
                <button
                  key={prompt.label}
                  type="button"
                  className="cw-suggested-chip"
                  onClick={() => setComposerText(prompt.seedText)}
                >
                  {prompt.label}
                </button>
              ))}
            </div>
          }
        />
      </div>
    </section>
  );
}
