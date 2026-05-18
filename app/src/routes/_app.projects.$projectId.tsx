import { Outlet, createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { getProject } from '@/api/projects';

export const Route = createFileRoute('/_app/projects/$projectId')({
  loader: ({ params, context }) =>
    context.queryClient.ensureQueryData({
      queryKey: ['project', params.projectId],
      queryFn: () => getProject(params.projectId),
    }),
  component: ProjectLayout,
});

function ProjectLayout() {
  return <Outlet />;
}

export function useProject(projectId: string) {
  return useQuery({ queryKey: ['project', projectId], queryFn: () => getProject(projectId) });
}
