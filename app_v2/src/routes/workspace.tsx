import { createFileRoute } from '@tanstack/react-router';
import { WorkspaceShell } from '@/workspace/components/WorkspaceShell';

export const Route = createFileRoute('/workspace')({
  component: WorkspaceLayout,
});

function WorkspaceLayout() {
  return <WorkspaceShell />;
}
