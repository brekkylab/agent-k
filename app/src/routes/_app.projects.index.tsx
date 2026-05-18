// Projects index — markup mirrors app-live ProjectsPage exactly.
// Server data: listProjects + per-project members/sessions for footer chips.

import { createFileRoute, useNavigate, useRouterState } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { listMembers, listProjects } from '@/api/projects';
import { listSessions } from '@/api/sessions';
import { Icon } from '@/components/Icon';
import { AvatarStack, SectionLabel } from '@/components/uiPrimitives';
import { useAuthStore } from '@/stores/auth';
import { useToastStore } from '@/components/Toast';
import type { Project, Session, User } from '@/domain/types';

function useActiveProjectIdFromRoute(): string | null {
  const state = useRouterState();
  for (const match of state.matches) {
    const params = match.params as { projectId?: string };
    if (params.projectId) return params.projectId;
  }
  return null;
}

export const Route = createFileRoute('/_app/projects/')({
  component: ProjectsPage,
});

function ProjectsPage() {
  const navigate = useNavigate();
  const showToast = useToastStore((s) => s.show);
  const projects = useQuery({ queryKey: ['projects'], queryFn: listProjects });
  const activeProjectId = useActiveProjectIdFromRoute() ?? projects.data?.[0]?.id ?? null;

  return (
    <section className="cw-page cw-projects-page cw-page-enter">
      <div className="cw-page-head">
        <div>
          <h1>Your projects</h1>
          <p>Each project is a workspace. Sessions, files, members, skills, schedule — all live inside.</p>
        </div>
        <button className="cw-btn-primary" onClick={() => showToast('새 프로젝트 생성 다이얼로그는 곧 추가됩니다')}>
          <Icon name="plus" /> New project
        </button>
      </div>
      <SectionLabel>Projects · {projects.data?.length ?? 0} projects</SectionLabel>
      <div className="cw-project-grid">
        {(projects.data ?? []).map((project) => (
          <ProjectCard
            key={project.id}
            project={project}
            isActive={project.id === activeProjectId}
            onOpen={() => navigate({ to: '/projects/$projectId', params: { projectId: project.id } })}
          />
        ))}
      </div>
    </section>
  );
}

function ProjectCard({ project, isActive, onOpen }: { project: Project; isActive: boolean; onOpen: () => void }) {
  const currentUser = useAuthStore((s) => s.currentUser);
  const members = useQuery({ queryKey: ['members', project.id], queryFn: () => listMembers(project.id) });
  const sessions = useQuery({ queryKey: ['sessions', project.id], queryFn: () => listSessions(project.id) });
  const isOwner = currentUser?.id === project.ownerId;
  const latest = latestUpdated(sessions.data ?? []);
  const memberUsers: User[] = members.data ?? [];

  return (
    <button className={`cw-project-card ${isActive ? 'is-active' : ''}`} onClick={onOpen}>
      <div className="cw-project-card-head">
        <span className="cw-project-card-name">
          <Icon name="folder" size={15} />
          <span>{project.name}</span>
        </span>
        <span className={`cw-role-badge ${isOwner ? 'owner' : 'member'}`}>{isOwner ? 'Owner' : 'Member'}</span>
      </div>
      <p className="cw-project-card-desc">{project.description || '설명 없음'}</p>
      <div className="cw-project-card-footer">
        <AvatarStack users={memberUsers} />
        <span className="cw-card-stats">{sessions.data?.length ?? 0}개 세션 · {latest}</span>
      </div>
    </button>
  );
}

function latestUpdated(sessions: Session[]): string {
  if (sessions.length === 0) return 'new';
  return sessions.map((s) => s.updatedAt).sort().reverse()[0] ?? 'new';
}
