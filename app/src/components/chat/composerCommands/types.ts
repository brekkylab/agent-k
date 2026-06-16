import type { ReactNode } from 'react';

import type { TriggerPosition } from '@/lib/composerCommandToken';

export interface CommandItem {
  // Stable key (file path, user id). Factories resolve it back to their own
  // payload in onSelect, so the framework never carries command-private data.
  id: string;
  label: string;
  // Dimmed secondary line — parent directory path, @username, etc.
  sublabel?: string;
  icon?: ReactNode;
}

export interface ComposerCommand {
  trigger: string;
  triggerPosition: TriggerPosition;
  // Synchronous filter over already-cached data (react-query). Keeping this
  // sync means the hook needs no debounce or race handling.
  getItems(query: string): CommandItem[];
  isLoading?: boolean;
  emptyLabel: string;
  // Returns the text that replaces the active "[trigger][query]" token:
  // '' removes it (file attach), '@username ' inserts a mention.
  onSelect(item: CommandItem): { replaceWith: string };
}
