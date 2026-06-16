import { useMemo } from 'react';

import { Icon } from '@/components/Icon';
import { Avatar } from '@/components/uiPrimitives';
import type { User } from '@/domain/types';
import { ALL_MENTION_SENTINEL } from '@/lib/mentionAll';

import type { CommandItem, ComposerCommand } from './types';

export interface PickedMention {
  userId: string;
  username: string;
}

// '@' command: mention a project member. Picking just inserts "@username " into
// the text; recognition is done at send time by scanning the text against the
// member roster, so a hand-typed "@username" works the same as a popup pick.
// onPick is optional (e.g. analytics); it does not drive recognition.
//
// allMention (optional) adds a synthetic "everyone" item at the top. Its id is
// the reserved sentinel; selecting it inserts the locale literal ('@모두 '/'@all '),
// which the send-time scan expands into the member list. Omit it (e.g. the
// project-home composer) to disable the everyone option.
export function useMentionCommand({
  members,
  emptyLabel,
  allMention,
  onPick,
}: {
  members: User[];
  emptyLabel: string;
  allMention?: { label: string; token: string } | null;
  onPick?: (mention: PickedMention) => void;
}): ComposerCommand {
  const items: CommandItem[] = useMemo(
    () =>
      members.map((user) => ({
        id: user.id,
        label: user.name,
        sublabel: user.username ? `@${user.username}` : undefined,
        icon: <Avatar user={user} small />,
      })),
    [members],
  );

  const allItem: CommandItem | null = useMemo(
    () =>
      allMention
        ? {
            id: ALL_MENTION_SENTINEL,
            label: allMention.label,
            sublabel: `@${allMention.token}`,
            icon: <Icon name="users" size={16} />,
          }
        : null,
    [allMention],
  );

  return useMemo(
    () => ({
      trigger: '@',
      triggerPosition: 'word-boundary' as const,
      emptyLabel,
      getItems: (query: string) => {
        const q = query.toLowerCase();
        const memberMatches = !q
          ? items
          : items.filter(
              (item) => item.label.toLowerCase().includes(q) || item.sublabel?.toLowerCase().includes(q),
            );
        // Surface the everyone item on an empty query or when it prefixes a
        // recognized token / matches the localized label.
        const showAll =
          !!allItem &&
          (q === '' ||
            'all'.startsWith(q) ||
            'everyone'.startsWith(q) ||
            '모두'.startsWith(q) ||
            allItem.label.toLowerCase().includes(q));
        return showAll && allItem ? [allItem, ...memberMatches] : memberMatches;
      },
      onSelect: (item: CommandItem) => {
        // Everyone: insert the locale literal; the send-time scan expands it.
        if (item.id === ALL_MENTION_SENTINEL && allMention) {
          return { replaceWith: `@${allMention.token} ` };
        }
        const user = members.find((m) => m.id === item.id);
        if (!user) return { replaceWith: '' };
        // The mention token must round-trip through the send-time scan, so the
        // inserted text and the candidate key are the same string.
        const username = user.username ?? user.name;
        onPick?.({ userId: user.id, username });
        return { replaceWith: `@${username} ` };
      },
    }),
    [items, allItem, allMention, members, emptyLabel, onPick],
  );
}
