import { Outlet, createFileRoute } from '@tanstack/react-router';

// Layout-only — actual /p view lives in _app.p.index.tsx so that
// the child route /p/$projectSlug can mount in this <Outlet />.
export const Route = createFileRoute('/_app/projects')({
  component: () => <Outlet />,
});
