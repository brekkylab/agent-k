export const PROVIDER_MODELS = {
  OpenAI: ["gpt-4o", "gpt-4o-mini", "gpt-4.1", "gpt-4.1-mini"],
  Anthropic: ["claude-sonnet-4-20250514", "claude-haiku-4-5-20251001"],
  Gemini: ["gemini-2.5-flash", "gemini-2.5-pro"],
} as const;

export type ProviderName = keyof typeof PROVIDER_MODELS;

export const PROVIDER_DEFAULT_PROFILE_NAMES: Record<ProviderName, string> = {
  OpenAI: "openai-default",
  Anthropic: "anthropic-default",
  Gemini: "gemini-default",
} as const;

// Backend serde: LangModelAPISchema #[serde(rename_all = "snake_case")]
// ChatCompletion → "chat_completion", Anthropic → "anthropic", Gemini → "gemini", OpenAI → "open_ai"
export const PROVIDER_CONFIG: Record<
  ProviderName,
  { schema: string; url: string }
> = {
  OpenAI: {
    schema: "chat_completion",
    url: "https://api.openai.com/v1/chat/completions",
  },
  Anthropic: {
    schema: "anthropic",
    url: "https://api.anthropic.com/v1/messages",
  },
  Gemini: {
    schema: "gemini",
    url: "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent",
  },
} as const;
