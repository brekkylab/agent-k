// Provider model selector for the composer, in the ChatGPT/Claude style: pick the
// underlying LLM (Claude / OpenAI / Gemini model) for the conversation. This is a
// mock for now — the selection is not yet wired to the router/dispatcher — but the
// labels are the real provider model names, grouped by provider.

import { Icon } from '@/components/Icon';
import { Select, type SelectGroup } from '@/components/Select';

export type ModelId = string;

interface ProviderGroup {
  provider: string;
  models: { id: ModelId; label: string }[];
}

export const PROVIDER_MODELS: readonly ProviderGroup[] = [
  {
    provider: 'Claude',
    models: [
      { id: 'claude-opus-4-7', label: 'Claude Opus 4.7' },
      { id: 'claude-haiku-4-5', label: 'Claude Haiku 4.5' },
    ],
  },
  {
    provider: 'OpenAI',
    models: [
      { id: 'gpt-5.5', label: 'GPT-5.5' },
      { id: 'gpt-5.4', label: 'GPT-5.4' },
    ],
  },
  {
    provider: 'Gemini',
    models: [
      { id: 'gemini-3.5-flash', label: 'Gemini 3.5 Flash' },
      { id: 'gemini-2.5-flash', label: 'Gemini 2.5 Flash' },
    ],
  },
] as const;

export const DEFAULT_MODEL_ID: ModelId = 'claude-opus-4-7';

export function ComposerModelPicker({
  value,
  onChange,
}: {
  value: ModelId;
  onChange: (id: ModelId) => void;
}) {
  const options: SelectGroup<ModelId>[] = PROVIDER_MODELS.map((group) => ({
    label: group.provider,
    options: group.models.map((model) => ({ value: model.id, label: model.label })),
  }));
  return (
    <span className="cw-model-picker" title="모델 선택 (미리보기)">
      <Icon name="sparkles" size={12} />
      <Select
        value={value}
        onChange={onChange}
        options={options}
        triggerClassName="cw-model-picker-trigger"
        ariaLabel="모델 선택 (미리보기)"
      />
    </span>
  );
}
