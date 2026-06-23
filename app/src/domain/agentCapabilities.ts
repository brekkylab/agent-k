// The fixed vocabulary of agent ("app-control") capabilities, mirroring the
// backend's stable capability names. Rendered in project settings: the ceiling
// editor and the current user's own-grant editor.
//
// Capabilities are grouped by the resource they act on for readability. The
// group order, and the capability order within each group, is display order.
// `member.*` lives under the `project` group — on the backend members are part
// of a project, not a standalone resource.
//
// i18n labels live under `agent_caps.<key>` (capability) and
// `agent_cap_groups.<group>` (group header) in the `project` namespace, where
// <key> is the capability name with `.` → `_`.
import type { IconName } from '@/components/Icon';

export const AGENT_CAPABILITY_GROUPS = [
  {
    group: 'user',
    capabilities: ['user.read_self', 'user.lookup', 'user.admin'],
  },
  {
    group: 'project',
    capabilities: ['project.read', 'member.read', 'member.manage'],
  },
  {
    group: 'session',
    capabilities: ['session.read'],
  },
  {
    group: 'automation',
    capabilities: [
      'automation.read',
      'automation.create',
      'automation.update',
      'automation.delete',
      'automation.run_read',
      'automation.run',
    ],
  },
] as const;

/** Flat list of all capabilities, in display order. Single source: the groups. */
export const AGENT_CAPABILITIES = AGENT_CAPABILITY_GROUPS.flatMap(
  (g) => g.capabilities,
) as readonly string[];

export type AgentCapability = (typeof AGENT_CAPABILITY_GROUPS)[number]['capabilities'][number];

/** Capability name → its i18n sub-key (`automation.read` → `automation_read`). */
export function capabilityLabelKey(capability: string): string {
  return capability.replace(/\./g, '_');
}

// One icon per action verb (the part after the `.`), so the same action reads
// the same across resources — e.g. every `*.read` shows an eye.
const ACTION_ICONS: Record<string, IconName> = {
  read: 'eye',
  read_self: 'eye',
  create: 'plus',
  update: 'writing',
  delete: 'trash',
  run: 'circle-play',
  run_read: 'list',
  manage: 'settings',
  lookup: 'search',
  admin: 'shield',
};

/** The icon for a capability, chosen by its action verb. */
export function capabilityIcon(capability: string): IconName {
  const action = capability.slice(capability.indexOf('.') + 1);
  return ACTION_ICONS[action] ?? 'check';
}
