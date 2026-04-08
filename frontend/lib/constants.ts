export const PROVIDER_MODELS = {
  OpenAI: ["gpt-5.4", "gpt-5.4-mini", "gpt-5.4-nano", "o4-mini", "o3", "gpt-4.1", "gpt-4.1-mini", "gpt-4.1-nano"],
  Anthropic: ["claude-sonnet-4-6", "claude-opus-4-6", "claude-haiku-4-5-20251001"],
  Gemini: ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.5-flash-lite", "gemini-3.1-pro-preview", "gemini-3-flash-preview"],
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
    url: "https://generativelanguage.googleapis.com/v1beta/models/",
  },
} as const;
