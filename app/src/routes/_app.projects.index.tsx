// Projects index — markup mirrors app-live ProjectsPage exactly.
// Server data: listProjects + per-project members/sessions for footer chips.

import { createFileRoute, useNavigate, useRouterState } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { listMembers, listProjects } from '@/api/projects';
import { listSessions } from '@/api/sessions';
import { Icon } from '@/components/Icon';
import { useNewProjectDialog } from '@/components/NewProjectDialog';
import { AvatarStack, SectionLabel } from '@/components/uiPrimitives';
import { loadNs } from '@/i18n/loader';
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
  // NewProjectDialog is rendered here → `dialogs`. Cards show member badges → `members`.
  loader: () => loadNs('project', 'members', 'dialogs'),
  component: ProjectsPage,
});

function ProjectsPage() {
  const { t } = useTranslation('project');
  const navigate = useNavigate();
  const projects = useQuery({ queryKey: ['projects'], queryFn: listProjects });
  const activeProjectSlug = useActiveProjectSlugFromRoute() ?? projects.data?.[0]?.slug ?? null;
  const creator = useNewProjectDialog();

  return (
    <section className="cw-page cw-projects-page cw-page-enter">
      <div className="cw-page-head">
        <div>
          <h1>{t('list_page.title')}</h1>
          <p>{t('list_page.description')}</p>
        </div>
        <button className="cw-btn-primary" onClick={creator.open}>
          <Icon name="plus" /> {t('list_page.new_project')}
        </button>
      </div>
      <SectionLabel>{t('list_page.section_label', { count: projects.data?.length ?? 0 })}</SectionLabel>
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
  const { t } = useTranslation(['project', 'common', 'members']);
  const currentUser = useAuthStore((s) => s.currentUser);
  const members = useQuery({ queryKey: ['members', project.slug], queryFn: () => listMembers(project.slug) });
  const sessions = useQuery({ queryKey: ['sessions', project.slug, 'user'], queryFn: () => listSessions(project.slug, 'user') });
  const isOwner = currentUser?.id === project.ownerId;
  const userSessions = sessions.data ?? [];
  const latestRaw = latestUpdated(userSessions);
  const latest = latestRaw === 'new' ? t('card.latest_new') : latestRaw;
  const memberUsers: User[] = members.data ?? [];
  // Any unread mention in this project's sessions → surface a marker on the card
  // so it's visible from the projects list (before drilling into a session).
  const hasMention = (sessions.data ?? []).some((s) => s.unreadMention);

  return (
    <button className={`cw-project-card ${isActive ? 'is-active' : ''}`} onClick={onOpen}>
      <div className="cw-project-card-head">
        <span className="cw-project-card-name">
          <Icon name="folder" size={15} />
          <span>{project.name}</span>
          {hasMention && (
            <span className="cw-mention-dot" role="img" aria-label="you have unread mentions here" title="Unread mentions" />
          )}
        </span>
        <span className={`cw-role-badge ${isOwner ? 'owner' : 'member'}`}>
          {isOwner ? t('members:badges.owner') : t('members:badges.member')}
        </span>
      </div>
      <p className="cw-project-card-desc">{project.description || t('common:placeholders.no_description')}</p>
      <div className="cw-project-card-footer">
        <AvatarStack users={memberUsers} />
        <span className="cw-card-stats">{t('card.session_count', { count: userSessions.length, latest })}</span>
        {/* i18next picks `_one` or `_other` based on `count` automatically. */}
      </div>
    </button>
  );
}

function latestUpdated(sessions: Session[]): string {
  if (sessions.length === 0) return 'new';
  return sessions.map((s) => s.updatedAt).sort().reverse()[0] ?? 'new';
}
