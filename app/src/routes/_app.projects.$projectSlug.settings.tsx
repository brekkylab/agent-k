import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { getProjectBySlug } from '@/api/projects';
import { SectionLabel } from '@/components/uiPrimitives';

export const Route = createFileRoute('/_app/projects/$projectSlug/settings')({
  component: SettingsPage,
});

function SettingsPage() {
  const { projectSlug } = Route.useParams();
  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProjectBySlug(projectSlug) });

  return (
    <section className="cw-page cw-simple-page cw-page-enter">
      <SectionLabel>Project metadata</SectionLabel>
      <h1>Settings</h1>
      <p>프로젝트 메타데이터입니다. 편집 기능은 곧 추가됩니다.</p>
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
