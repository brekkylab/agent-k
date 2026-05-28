// Projects index — markup mirrors app-live ProjectsPage exactly.
// Server data: listProjects + per-project members/sessions for footer chips.

import { createFileRoute, useNavigate, useRouterState } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { listMembers, listProjects } from '@/api/projects';
import { listSessions } from '@/api/sessions';
import { Icon } from '@/components/Icon';
import { useNewProjectDialog } from '@/components/NewProjectDialog';
import { AvatarStack, SectionLabel } from '@/components/uiPrimitives';
import { useAuthStore } from '@/stores/auth';
import type { Project, Session, User } from '@/domain/types';

function useActiveProjectSlugFromRoute(): string | null {
  const state = useRouterState();
  for (const match of state.matches) {
    const params = match.params as { projectSlug?: string };
    if (params.projectSlug) return params.projectSlug;
  }
  return null;
}

export const Route = createFileRoute('/_app/projects/')({
  component: ProjectsPage,
});

function ProjectsPage() {
  const navigate = useNavigate();
  const projects = useQuery({ queryKey: ['projects'], queryFn: listProjects });
  const activeProjectSlug = useActiveProjectSlugFromRoute() ?? projects.data?.[0]?.slug ?? null;
  const creator = useNewProjectDialog();

  return (
    <section className="cw-page cw-projects-page cw-page-enter">
      <div className="cw-page-head">
        <div>
          <h1>Your projects</h1>
          <p>Each project is a workspace. Sessions, files, members, skills, schedule — all live inside.</p>
        </div>
        <button className="cw-btn-primary" onClick={creator.open}>
          <Icon name="plus" /> New project
        </button>
      </div>
      <SectionLabel>Projects · {projects.data?.length ?? 0} projects</SectionLabel>
      <div className="cw-project-grid">
        {(projects.data ?? []).map((project) => (
          <ProjectCard
            key={project.id}
            project={project}
            isActive={project.slug === activeProjectSlug}
            onOpen={() => navigate({ to: '/projects/$projectSlug', params: { projectSlug: project.slug } })}
          />
        ))}
      </div>
      {creator.dialog}
    </section>
  );
}

function ProjectCard({ project, isActive, onOpen }: { project: Project; isActive: boolean; onOpen: () => void }) {
  const currentUser = useAuthStore((s) => s.currentUser);
  const members = useQuery({ queryKey: ['members', project.slug], queryFn: () => listMembers(project.slug) });
  const sessions = useQuery({ queryKey: ['sessions', project.slug], queryFn: () => listSessions(project.slug) });
  const isOwner = currentUser?.id === project.ownerId;
  const userSessions = (sessions.data ?? []).filter((s) => s.origin === 'user');
  const latest = latestUpdated(userSessions);
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
        <span className="cw-card-stats">{userSessions.length}개 세션 · {latest}</span>
      </div>
    </button>
  );
}

function latestUpdated(sessions: Session[]): string {
  if (sessions.length === 0) return 'new';
  return sessions.map((s) => s.updatedAt).sort().reverse()[0] ?? 'new';
}
