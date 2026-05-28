import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { getProject } from '@/api/projects';
import { SectionLabel } from '@/components/uiPrimitives';

export const Route = createFileRoute('/_app/projects/$projectSlug/settings')({
  component: SettingsPage,
});

function SettingsPage() {
  const { projectSlug } = Route.useParams();
  const { t } = useTranslation('project');
  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });

  return (
    <section className="cw-page cw-simple-page cw-page-enter">
      <SectionLabel>Project metadata</SectionLabel>
      <h1>Settings</h1>
      <p>{t('settings_page.subtitle')}</p>
      <div className="cw-simple-stack">
        <code>name: {project.data?.name ?? '—'}</code>
        <code>description: {project.data?.description || '—'}</code>
        <code>slug: {projectSlug}</code>
        <code>id: {project.data?.id ?? '—'}</code>
        <code>owner_id: {project.data?.ownerId ?? '—'}</code>
      </div>
    </section>
  );
}
