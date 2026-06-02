// Model selector for the composer. Picks the underlying LLM for the
// conversation, grouped by capability tier. Tiers are display-only group
// headers — never selected directly. The first option is "recommended"
// (value = empty string → null), which resolves dynamically per agent type at
// agent-build time. Models in the active agent's recommendation chain are
// marked with ★; models whose provider has no API key on this server are
// disabled.

import { useId } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '@/components/Icon';
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
  const selectId = useId();
  const rec = recommendationFor(catalog, agentType);
  const recommendedSet = new Set(rec?.chain ?? []);
  // Name the resolved model in the "recommended" label only when it can run;
  // otherwise stay generic.
  const resolvedAvailable = rec
    ? !!catalog?.models.find((m) => m.id === rec.resolvedModel)?.available
    : false;
  const resolvedLabel = resolvedAvailable ? modelLabel(catalog, rec!.resolvedModel) : '';

  return (
    <span className="cw-model-picker" title={t('model_picker.select')}>
      <Icon name="sparkles" size={12} />
      <label htmlFor={selectId} className="sr-only">{t('model_picker.select')}</label>
      <select
        id={selectId}
        value={value ?? ''}
        onChange={(event) => onChange(event.target.value === '' ? null : event.target.value)}
        disabled={!catalog}
      >
        <option value="">{resolvedLabel ? t('model_picker.recommended_named', { label: resolvedLabel }) : t('model_picker.recommended')}</option>
        {TIER_ORDER.map((tier) => {
          const models = (catalog?.models ?? []).filter((m) => m.tier === tier);
          if (models.length === 0) return null;
          return (
            <optgroup key={tier} label={t(`tier.${tier}`)}>
              {models.map((model) => {
                const suffix = !model.available
                  ? t('model_picker.unavailable_suffix')
                  : recommendedSet.has(model.id)
                    ? ' ★'
                    : '';
                return (
                  <option key={model.id} value={model.id} disabled={!model.available}>
                    {model.label}
                    {suffix}
                  </option>
                );
              })}
            </optgroup>
          );
        })}
      </select>
    </span>
  );
}
