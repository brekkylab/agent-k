import { Outlet, createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/_app/projects/$projectId/automation')({
  component: AutomationLayout,
});

function AutomationLayout() {
  return <Outlet />;
}
