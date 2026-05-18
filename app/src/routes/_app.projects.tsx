import { Outlet, createFileRoute } from '@tanstack/react-router';

// Layout-only — actual /projects view lives in _app.projects.index.tsx so that
// the child route /projects/$projectId can mount in this <Outlet />.
export const Route = createFileRoute('/_app/projects')({
  component: () => <Outlet />,
});
