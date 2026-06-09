// Model selector for the composer. Picks the underlying LLM for the
// conversation, grouped by capability tier. Tiers are display-only group
// headers — never selected directly. The first option is "recommended"
// (value = null), which resolves dynamically per agent type at agent-build
// time. Models in the active agent's recommendation chain are marked with ★;
// models whose provider has no API key, and models the active agent does not
// permit (e.g. non-2.5 Gemini for Speedwagon), are disabled.

import { useTranslation } from 'react-i18next';
import { Icon } from '@/components/Icon';
import { Select, type SelectGroup } from '@/components/Select';
import {
  type ModelCatalog,
  type ModelTier,
  modelLabel,
  recommendationFor,
} from '@/api/models';

const TIER_ORDER: ModelTier[] = ['light', 'standard', 'max'];

export function ComposerModelPicker({
  catalog,
  agentType,
  value,
  onChange,
}: {
  catalog: ModelCatalog | undefined;
  /** Canonical agent_type, used to highlight recommended models. */
  agentType: string;
  /** Selected model id, or null for "recommended". */
  value: string | null;
  onChange: (id: string | null) => void;
}) {
  const { t } = useTranslation('automation');
  const rec = recommendationFor(catalog, agentType);
  const recommendedSet = new Set(rec?.chain ?? []);
  // Models this agent permits. Absent `allowed` (older backend) → unrestricted.
  const allowedSet = rec?.allowed ? new Set(rec.allowed) : null;
  const isAllowed = (id: string) => !allowedSet || allowedSet.has(id);
  // Name the resolved model in the "recommended" label only when it can run;
  // otherwise stay generic.
  const resolvedAvailable = rec
    ? !!catalog?.models.find((m) => m.id === rec.resolvedModel)?.available
    : false;
  const resolvedLabel = resolvedAvailable ? modelLabel(catalog, rec!.resolvedModel) : '';

  // "Recommended" (value = null) sits in its own unlabeled group at the top;
  // the tier groups follow. Chain members get ★; unavailable models are disabled.
  const options: SelectGroup<string | null>[] = [
    {
      options: [{
        value: null,
        label: resolvedLabel
          ? t('model_picker.recommended_named', { label: resolvedLabel })
          : t('model_picker.recommended'),
      }],
    },
    ...TIER_ORDER.flatMap((tier) => {
      const models = (catalog?.models ?? []).filter((m) => m.tier === tier);
      if (models.length === 0) return [];
      return [{
        label: t(`tier.${tier}`),
        options: models.map((model) => {
          const restricted = !isAllowed(model.id);
          return {
            value: model.id,
            label: model.label + (
              !model.available
                ? t('model_picker.unavailable_suffix')
                : restricted
                  ? t('model_picker.restricted_suffix')
                  : recommendedSet.has(model.id) ? ' ★' : ''
            ),
            disabled: !model.available || restricted,
          };
        }),
      }];
    }),
  ];

  return (
    <span className="cw-model-picker" title={t('model_picker.select')}>
      <Icon name="sparkles" size={12} />
      <Select
        value={value}
        onChange={onChange}
        options={options}
        triggerClassName="cw-model-picker-trigger"
        ariaLabel={t('model_picker.select')}
        disabled={!catalog}
      />
    </span>
  );
}
