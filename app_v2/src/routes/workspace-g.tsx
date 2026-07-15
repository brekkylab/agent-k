// Workspace candidate G — multi-connection sources: the same Workspace UI as
// `/workspace`, but with the instance-scoped connection model (N connections
// per mount-backed type). Isolated fork so `/workspace` stays frozen.

import { createFileRoute } from '@tanstack/react-router';
import { WorkspaceShell } from '@/workspace-connections/components/WorkspaceShell';

export const Route = createFileRoute('/workspace-g')({
  component: WorkspaceShell,
});
