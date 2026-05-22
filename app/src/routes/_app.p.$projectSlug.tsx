import { Outlet, createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { getProject } from '@/api/projects';

export const Route = createFileRoute('/_app/p/$projectSlug')({
  loader: ({ params, context }) =>
    context.queryClient.ensureQueryData({
      queryKey: ['project', params.projectSlug],
      queryFn: () => getProject(params.projectSlug),
    }),
  component: ProjectLayout,
});

function ProjectLayout() {
  return <Outlet />;
}

export function useProject(projectSlug: string) {
  return useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
}
