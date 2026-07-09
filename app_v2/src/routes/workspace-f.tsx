// Workspace candidate F — ops console archetype (Glean/Dust knowledge ops console).

import { createFileRoute } from '@tanstack/react-router';
import { OpsConsoleWorkspace } from '@/workspace-candidates/opsconsole/OpsConsoleWorkspace';

export const Route = createFileRoute('/workspace-f')({
  component: OpsConsoleWorkspace,
});
