"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  PROVIDER_MODELS,
  PROVIDER_CONFIG,
  type ProviderName,
} from "@/lib/constants";
import { createProviderProfile, ApiError } from "@/lib/api";

const PROVIDER_NAMES = Object.keys(PROVIDER_MODELS) as ProviderName[];

export function ProviderForm({ onCreated }: { onCreated?: () => void }) {
  const [provider, setProvider] = useState<ProviderName>("OpenAI");
  const [apiKey, setApiKey] = useState("");
  const [name, setName] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setLoading(true);

    const config = PROVIDER_CONFIG[provider];
    const profileName = name.trim() || `${provider} Default`;

    try {
      await createProviderProfile({
        name: profileName,
        provider: {
          lm: {
            type: "api",
            schema: config.schema,
            url: config.url,
            api_key: apiKey,
          },
          tools: [],
        },
      });
      setApiKey("");
      setName("");
      onCreated?.();
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message);
      } else {
        setError("알 수 없는 오류가 발생했습니다.");
      }
    } finally {
      setLoading(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <label className="text-sm font-medium">LLM Provider</label>
          <select
            value={provider}
            onChange={(e) => setProvider(e.target.value as ProviderName)}
            className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            {PROVIDER_NAMES.map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))}
          </select>
        </div>

        <div className="space-y-2">
          <label className="text-sm font-medium">프로필 이름</label>
          <Input
            placeholder={`${provider} Default`}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
      </div>

      <div className="space-y-2">
        <label className="text-sm font-medium">API Key</label>
        <Input
          type="password"
          placeholder="sk-..."
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          required
        />
      </div>

      {error && (
        <p className="text-sm text-destructive">{error}</p>
      )}

      <Button type="submit" disabled={loading || !apiKey.trim()}>
        {loading ? "저장 중..." : "Provider 등록"}
      </Button>
    </form>
  );
}
