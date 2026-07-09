// Workspace candidate B — NotebookLM archetype (3-pane: sources / chat / studio).

import { createFileRoute } from '@tanstack/react-router';
import { NotebookLMWorkspace } from '@/workspace-candidates/notebooklm/NotebookLMWorkspace';

export const Route = createFileRoute('/workspace-b')({
  component: NotebookLMWorkspace,
});
