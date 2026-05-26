import { Outlet, createFileRoute, redirect } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { getProjectBySlug } from '@/api/projects';

export const Route = createFileRoute('/_app/projects/$projectSlug')({
  loader: async ({ params, context, location }) => {
    const project = await context.queryClient.ensureQueryData({
      queryKey: ['project', params.projectSlug],
      queryFn: () => getProjectBySlug(params.projectSlug),
    });
    // Followed a retired slug: rewrite the URL to the project's current slug
    // while keeping whatever sub-path (files/members/s/{id}/...) the user was on.
    if (project.slug !== params.projectSlug) {
      const target = location.pathname.replace(
        `/projects/${params.projectSlug}`,
        `/projects/${project.slug}`,
      );
      throw redirect({ to: target, replace: true });
    }
    return project;
  },
  component: ProjectLayout,
});

function ProjectLayout() {
  return <Outlet />;
}

export function useProject(projectSlug: string) {
  return useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProjectBySlug(projectSlug) });
}
