import { Outlet, createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/_app/projects/$projectSlug/automation')({
  component: AutomationLayout,
});

function AutomationLayout() {
  return <Outlet />;
}
