import { Outlet, createFileRoute, redirect } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { getProject } from '@/api/projects';
import { loadNs } from '@/i18n/loader';

export const Route = createFileRoute('/_app/projects/$projectSlug')({
  loader: async ({ params, context, location }) => {
    // Fetch project metadata and warm `common`/`project` ns in parallel.
    // These ns also live on `_app`'s loader, so this `loadNs` is a no-op
    // when the user enters via the SPA — but guarantees they're present
    // when the route is hit directly (refresh on a deep link).
    const [project] = await Promise.all([
      context.queryClient.ensureQueryData({
        queryKey: ['project', params.projectSlug],
        queryFn: () => getProject(params.projectSlug),
      }),
      loadNs('common', 'project'),
    ]);
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
  return useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
}
