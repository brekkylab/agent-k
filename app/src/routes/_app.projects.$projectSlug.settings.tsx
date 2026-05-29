import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { getProject } from '@/api/projects';
import { SectionLabel } from '@/components/uiPrimitives';
import { loadNs } from '@/i18n/loader';

export const Route = createFileRoute('/_app/projects/$projectSlug/settings')({
  loader: () => loadNs('project'),
  component: SettingsPage,
});

function SettingsPage() {
  const { projectSlug } = Route.useParams();
  const { t } = useTranslation('project');
  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });

  return (
    <section className="cw-page cw-simple-page cw-page-enter">
      <SectionLabel>{t('settings_page_meta.section_label')}</SectionLabel>
      <h1>{t('settings_page_meta.title')}</h1>
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
