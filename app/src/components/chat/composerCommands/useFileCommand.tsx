import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';

import { listDirentsRaw, stripScopePrefix } from '@/api/dirents';
import { FileTypeIcon } from '@/components/FileTypeIcon';

import type { CommandItem, ComposerCommand } from './types';

export interface PickedSharedFile {
  filename: string;
  globalPath: string;
}

// '#' command: list the project's shared files and hand the pick to the
// attachment tray. Shares the files page's query cache.
export function useFileCommand({
  projectId,
  emptyLabel,
  onPick,
}: {
  projectId: string;
  emptyLabel: string;
  onPick: (file: PickedSharedFile) => void;
}): ComposerCommand {
  const dirents = useQuery({
    queryKey: ['dirents', 'shared', projectId],
    queryFn: () => listDirentsRaw({ kind: 'shared', projectId }, true),
    enabled: projectId.length > 0,
  });

  const items: CommandItem[] = useMemo(() => {
    const scope = { kind: 'shared', projectId } as const;
    return (dirents.data ?? [])
      .filter((e) => e.kind === 'file')
      .map((e) => {
        const rel = stripScopePrefix(scope, e.path);
        const segments = rel.split('/');
        const filename = segments[segments.length - 1];
        return { globalPath: e.path, filename, parent: segments.slice(0, -1).join('/') };
      })
      .filter((f) => !f.filename.startsWith('.'))
      .map((f) => ({
        id: f.globalPath,
        label: f.filename,
        sublabel: f.parent || undefined,
        icon: <FileTypeIcon filename={f.filename} size={16} />,
      }));
  }, [dirents.data, projectId]);

  return useMemo(
    () => ({
      trigger: '#',
      triggerPosition: 'word-boundary' as const,
      isLoading: dirents.isLoading,
      emptyLabel,
      getItems: (query: string) => {
        const q = query.toLowerCase();
        if (!q) return items;
        return items.filter(
          (item) => item.label.toLowerCase().includes(q) || item.sublabel?.toLowerCase().includes(q),
        );
      },
      onSelect: (item: CommandItem) => {
        onPick({ filename: item.label, globalPath: item.id });
        // The '#query' token disappears; the file shows up as a chip instead.
        return { replaceWith: '' };
      },
    }),
    [items, dirents.isLoading, emptyLabel, onPick],
  );
}
