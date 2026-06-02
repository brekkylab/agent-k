import { useEffect, useState } from 'react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { deleteProject, getProject, listMembers, listProjects, updateProject } from '@/api/projects';
import { Avatar, SectionLabel } from '@/components/uiPrimitives';
import { Icon } from '@/components/Icon';
import type { User } from '@/domain/types';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { useToastStore } from '@/components/Toast';
import { useAuthStore } from '@/stores/auth';
import { canEditProject } from '@/lib/permissions';
import { ProjectModelChainsEditor } from '@/components/settings/ProjectModelChainsEditor';
import { ApiError } from '@/api/client';

export const Route = createFileRoute('/_app/projects/$projectSlug/settings')({
  component: SettingsPage,
});

function SettingsPage() {
  const { projectSlug } = Route.useParams();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);
  const currentUser = useAuthStore((s) => s.currentUser);

  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
  const members = useQuery({ queryKey: ['members', projectSlug], queryFn: () => listMembers(projectSlug) });

  const editable = canEditProject(project.data, currentUser);

  const ownerUser: User | null = (() => {
    if (!project.data) return null;
    const fromMembers = (members.data ?? []).find((m) => m.id === project.data!.ownerId);
    if (fromMembers) return fromMembers;
    if (currentUser?.id === project.data.ownerId) return currentUser;
    return null;
  })();

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [editMode, setEditMode] = useState(false);
  const [descEditMode, setDescEditMode] = useState(false);

  // 서버 데이터가 도착하면 input 초깃값을 채운다. 사용자가 편집하는 동안
  // 외부 refetch가 와도 입력을 덮어쓰지 않도록 project id 기반 dependency.
  useEffect(() => {
    if (project.data) {
      setName(project.data.name);
      setDescription(project.data.description ?? '');
    }
  }, [project.data?.id]);

  const updateMutation = useMutation({
    mutationFn: () => updateProject(projectSlug, { name: name.trim() }),
    onSuccess: async (updated) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['project', projectSlug] }),
        queryClient.invalidateQueries({ queryKey: ['projects'] }),
      ]);
      setName(updated.name);
      setSubmitError(null);
      setEditMode(false);
      showToast('프로젝트 이름을 변경했습니다');
    },
    onError: (err) => setSubmitError(messageOf(err)),
  });

  const descMutation = useMutation({
    mutationFn: () => updateProject(projectSlug, {
      description: description.trim() ? description.trim() : null,
    }),
    onSuccess: async (updated) => {
      await queryClient.invalidateQueries({ queryKey: ['project', projectSlug] });
      setDescription(updated.description ?? '');
      setSubmitError(null);
      setDescEditMode(false);
      showToast('설명을 변경했습니다');
    },
    onError: (err) => setSubmitError(messageOf(err)),
  });

  const deleteMutation = useMutation({
    mutationFn: () => deleteProject(projectSlug),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['projects'] });
      queryClient.removeQueries({ queryKey: ['project', projectSlug] });
      const remaining = (await queryClient.fetchQuery({ queryKey: ['projects'], queryFn: listProjects })) ?? [];
      showToast('프로젝트가 삭제되었습니다');
      setDeleteOpen(false);
      if (remaining[0]) {
        navigate({ to: '/projects/$projectSlug', params: { projectSlug: remaining[0].slug } });
      } else {
        navigate({ to: '/projects' });
      }
    },
    onError: (err) => {
      showToast(`삭제 실패: ${messageOf(err)}`);
    },
  });

  const trimmedName = name.trim();
  const dirty = project.data ? trimmedName !== project.data.name : false;
  const saveDisabled = !editable || !dirty || trimmedName.length === 0 || updateMutation.isPending;

  const descDirty = project.data
    ? description.trim() !== (project.data.description ?? '')
    : false;
  const descSaveDisabled = !editable || !descDirty || descMutation.isPending;

  return (
    <section className="cw-page cw-page-enter">
      <SectionLabel>Project settings</SectionLabel>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 18,
          flexWrap: 'wrap',
          marginTop: 4,
        }}
      >
        <div style={{ display: 'inline-flex', alignItems: 'center', gap: 10 }}>
          {editable && editMode ? (
            <>
              <input
                autoFocus
                value={name}
                onChange={(e) => { setName(e.target.value); if (submitError) setSubmitError(null); }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    if (!saveDisabled) updateMutation.mutate();
                  } else if (e.key === 'Escape') {
                    e.preventDefault();
                    if (project.data) setName(project.data.name);
                    setSubmitError(null);
                    setEditMode(false);
                  }
                }}
                disabled={updateMutation.isPending}
                maxLength={100}
                aria-label="프로젝트 이름"
                style={{
                  margin: 0,
                  padding: '4px 10px',
                  border: '1px solid var(--cw-line)',
                  borderRadius: 8,
                  background: 'var(--cw-paper)',
                  color: 'var(--cw-ink)',
                  fontSize: 'var(--cw-text-2xl)',
                  lineHeight: 1.12,
                  letterSpacing: '-0.025em',
                  fontWeight: 650,
                  fontFamily: 'inherit',
                  minWidth: 280,
                }}
              />
              <button
                type="button"
                className="cw-btn-primary"
                onClick={() => updateMutation.mutate()}
                disabled={saveDisabled}
              >
                {updateMutation.isPending ? '저장 중…' : '저장'}
              </button>
              <button
                type="button"
                className="cw-btn-secondary"
                onClick={() => {
                  if (project.data) setName(project.data.name);
                  setSubmitError(null);
                  setEditMode(false);
                }}
                disabled={updateMutation.isPending}
              >
                취소
              </button>
            </>
          ) : (
            <>
              <h1 style={{ margin: 0 }}>{project.data?.name ?? '…'}</h1>
              {editable && project.data && (
                <button
                  type="button"
                  onClick={() => {
                    setSubmitError(null);
                    setEditMode(true);
                  }}
                  aria-label="프로젝트 이름 편집"
                  title="편집"
                  style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    width: 30,
                    height: 30,
                    border: '1px solid var(--cw-line)',
                    borderRadius: 8,
                    background: 'var(--cw-paper-2)',
                    color: 'var(--cw-ink-3)',
                    cursor: 'pointer',
                    padding: 0,
                  }}
                >
                  <Icon name="writing" size={14} />
                </button>
              )}
            </>
          )}
        </div>
        {ownerUser && (
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 8,
              fontSize: 13,
              color: 'var(--cw-ink-3)',
            }}
          >
            <span>Owned by</span>
            <Avatar user={ownerUser} small />
            <b style={{ color: 'var(--cw-ink-2)', fontWeight: 600 }}>{ownerUser.name}</b>
          </span>
        )}
      </div>
      <p
        style={{
          margin: '6px 0 24px',
          fontSize: 12,
          color: 'var(--cw-ink-4)',
          fontFamily: 'var(--cw-font-mono)',
          fontStyle: 'italic',
        }}
      >
        {projectSlug}
      </p>

      {project.data && (
        <div style={{ marginBottom: 24 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
            <SectionLabel>Description</SectionLabel>
            {editable && !descEditMode && (
              <button
                type="button"
                onClick={() => {
                  setSubmitError(null);
                  setDescEditMode(true);
                }}
                aria-label="설명 편집"
                title="편집"
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  width: 22,
                  height: 22,
                  border: '1px solid var(--cw-line)',
                  borderRadius: 6,
                  background: 'var(--cw-paper-2)',
                  color: 'var(--cw-ink-3)',
                  cursor: 'pointer',
                  padding: 0,
                }}
              >
                <Icon name="writing" size={12} />
              </button>
            )}
          </div>

          {editable && descEditMode ? (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              <textarea
                autoFocus
                value={description}
                onChange={(e) => { setDescription(e.target.value); if (submitError) setSubmitError(null); }}
                onKeyDown={(e) => {
                  if (e.key === 'Escape') {
                    e.preventDefault();
                    setDescription(project.data?.description ?? '');
                    setSubmitError(null);
                    setDescEditMode(false);
                  }
                }}
                placeholder="이 프로젝트는 어떤 작업인가요?"
                disabled={descMutation.isPending}
                rows={3}
                style={{
                  resize: 'vertical',
                  minHeight: 80,
                  fontFamily: 'inherit',
                  fontSize: 14,
                  lineHeight: 1.6,
                  padding: '10px 12px',
                  border: '1px solid var(--cw-line)',
                  borderRadius: 8,
                  background: 'var(--cw-paper)',
                  color: 'var(--cw-ink)',
                }}
              />
              <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
                <button
                  type="button"
                  className="cw-btn-secondary"
                  onClick={() => {
                    setDescription(project.data?.description ?? '');
                    setSubmitError(null);
                    setDescEditMode(false);
                  }}
                  disabled={descMutation.isPending}
                >
                  취소
                </button>
                <button
                  type="button"
                  className="cw-btn-primary"
                  onClick={() => descMutation.mutate()}
                  disabled={descSaveDisabled}
                >
                  {descMutation.isPending ? '저장 중…' : '저장'}
                </button>
              </div>
            </div>
          ) : (
            <p
              style={{
                margin: 0,
                color: project.data.description ? 'var(--cw-ink-2)' : 'var(--cw-ink-4)',
                fontSize: 14,
                lineHeight: 1.6,
              }}
            >
              {project.data.description || '설명이 없습니다.'}
            </p>
          )}
        </div>
      )}

      {project.data && (
        <ProjectModelChainsEditor
          key={project.data.id}
          projectSlug={projectSlug}
          overrides={project.data.recommendedChains}
          editable={editable}
        />
      )}

      {submitError && (
        <div className="cw-dialog-warn" role="alert" style={{ marginBottom: 20 }}>{submitError}</div>
      )}

      {editable && (
        <div
          style={{
            marginTop: 24,
            border: '1px solid var(--cw-destructive)',
            borderRadius: 12,
            background: 'var(--cw-paper-2)',
            padding: 20,
          }}
        >
          <SectionLabel>Danger zone</SectionLabel>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, marginTop: 8 }}>
            <div style={{ minWidth: 0 }}>
              <h2 style={{ margin: 0, fontSize: 14, color: 'var(--cw-ink)' }}>프로젝트 삭제</h2>
              <p style={{ margin: '4px 0 0', color: 'var(--cw-ink-3)', fontSize: 12, lineHeight: 1.55 }}>
                세션·메시지·업로드 파일·sandbox 자원이 모두 함께 삭제됩니다. 이 작업은 되돌릴 수 없어요.
              </p>
            </div>
            <button
              type="button"
              className="cw-btn-primary"
              style={{ background: 'var(--cw-destructive)', borderColor: 'var(--cw-destructive)' }}
              onClick={() => setDeleteOpen(true)}
            >
              프로젝트 삭제
            </button>
          </div>
        </div>
      )}

      {deleteOpen && project.data && (
        <ConfirmDialog
          title="프로젝트를 삭제하시겠어요?"
          body={`"${project.data.name}"의 모든 세션·메시지·업로드가 함께 정리됩니다. 이 작업은 되돌릴 수 없습니다.`}
          confirmLabel="삭제"
          destructive
          pending={deleteMutation.isPending}
          onConfirm={() => deleteMutation.mutate()}
          onClose={() => setDeleteOpen(false)}
        />
      )}
    </section>
  );
}

function messageOf(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.status === 401) return '로그인이 만료되었습니다. 다시 로그인해 주세요.';
    if (err.status === 403) return '권한이 없습니다 (소유자만 가능)';
    if (err.status === 404) return '프로젝트를 찾을 수 없습니다.';
    if (err.status === 409) return '같은 이름의 프로젝트가 이미 있어요.';
    if (err.status === 422 || err.status === 400) return '입력값을 확인해 주세요.';
    return `${err.status} — ${err.message}`;
  }
  if (err instanceof Error) return err.message;
  return '요청에 실패했습니다.';
}
