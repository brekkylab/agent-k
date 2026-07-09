// Workspace candidate D — cultivation archetype (collections / triage / provenance).

import { createFileRoute } from '@tanstack/react-router';
import { CultivateWorkspace } from '@/workspace-candidates/cultivate/CultivateWorkspace';

export const Route = createFileRoute('/workspace-d')({
  component: CultivateWorkspace,
});
