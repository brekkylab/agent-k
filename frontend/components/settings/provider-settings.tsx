"use client";

import { useEffect, useState, useCallback } from "react";
import { Check, Trash2, KeyRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  PROVIDER_CONFIG,
  PROVIDER_DEFAULT_PROFILE_NAMES,
  type ProviderName,
} from "@/lib/constants";
import {
  getProviderProfiles,
  createProviderProfile,
  updateProviderProfile,
  deleteProviderProfile,
  ApiError,
} from "@/lib/api";
import type { ApiProviderProfile } from "@/lib/types";
import { toast } from "sonner";

const PROVIDERS: ProviderName[] = ["OpenAI", "Anthropic", "Gemini"];

interface ProviderCardState {
  profile: ApiProviderProfile | null;
  apiKey: string;
  loading: boolean;
  error: string | null;
  success: boolean;
  editing: boolean;
}

export function ProviderSettings() {
  const [states, setStates] = useState<Record<ProviderName, ProviderCardState>>(
    () => {
      const init = {} as Record<ProviderName, ProviderCardState>;
      for (const p of PROVIDERS) {
        init[p] = { profile: null, apiKey: "", loading: false, error: null, success: false, editing: false };
      }
      return init;
    }
  );
  const [fetching, setFetching] = useState(true);

  const fetchProfiles = useCallback(async () => {
    try {
      const profiles = await getProviderProfiles();
      setStates((prev) => {
        const next = { ...prev };
        for (const p of PROVIDERS) {
          const defaultName = PROVIDER_DEFAULT_PROFILE_NAMES[p];
          const found = profiles.find((pr) => pr.name === defaultName);
          next[p] = { ...next[p], profile: found ?? null };
        }
        return next;
      });
    } catch {
      toast.error("Provider 프로필을 불러오지 못했습니다");
    } finally {
      setFetching(false);
    }
  }, []);

  useEffect(() => {
    fetchProfiles();
  }, [fetchProfiles]);

  const updateCard = (provider: ProviderName, updates: Partial<ProviderCardState>) => {
    setStates((prev) => ({
      ...prev,
      [provider]: { ...prev[provider], ...updates },
    }));
  };

  const handleSave = async (provider: ProviderName) => {
    const card = states[provider];
    if (!card.apiKey.trim()) return;

    updateCard(provider, { loading: true, error: null, success: false });
    const config = PROVIDER_CONFIG[provider];
    const profileName = PROVIDER_DEFAULT_PROFILE_NAMES[provider];

    const providerData = {
      lm: {
        type: "api" as const,
        schema: config.schema,
        url: config.url,
        api_key: card.apiKey,
      },
      tools: [] as unknown[],
    };

    try {
      let profile: ApiProviderProfile;
      if (card.profile) {
        profile = await updateProviderProfile(card.profile.id, {
          name: profileName,
          provider: providerData,
          is_default: true,
        });
      } else {
        profile = await createProviderProfile({
          name: profileName,
          provider: providerData,
          is_default: true,
        });
      }
      updateCard(provider, { profile, apiKey: "", loading: false, success: true, editing: false });
      setTimeout(() => updateCard(provider, { success: false }), 2000);
    } catch (err) {
      updateCard(provider, {
        loading: false,
        error: err instanceof ApiError ? err.message : "저장 실패",
      });
    }
  };

  const handleDelete = async (provider: ProviderName) => {
    const card = states[provider];
    if (!card.profile) return;

    updateCard(provider, { loading: true, error: null });
    try {
      await deleteProviderProfile(card.profile.id);
      updateCard(provider, { profile: null, apiKey: "", loading: false, editing: false });
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        updateCard(provider, {
          loading: false,
          error: "이 Provider를 사용하는 세션이 있어 삭제할 수 없습니다.",
        });
      } else {
        updateCard(provider, {
          loading: false,
          error: err instanceof ApiError ? err.message : "삭제 실패",
        });
      }
    }
  };

  if (fetching) {
    return <p className="text-sm text-muted-foreground">불러오는 중...</p>;
  }

  return (
    <div className="space-y-2">
      {PROVIDERS.map((provider) => {
        const card = states[provider];
        const isRegistered = card.profile !== null;

        return (
          <div key={provider}>
            {/* 한 줄 요약 */}
            <div className="flex items-center justify-between rounded-md border px-4 py-2.5">
              <div className="flex items-center gap-3">
                <KeyRound className="h-4 w-4 text-muted-foreground" />
                <span className="text-sm font-medium">{provider}</span>
                {isRegistered ? (
                  <span className="flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-400">
                    <Check className="h-3 w-3" />
                    등록됨
                  </span>
                ) : (
                  <span className="text-xs text-muted-foreground">미등록</span>
                )}
              </div>
              <div className="flex items-center gap-1">
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 text-xs"
                  onClick={() => updateCard(provider, { editing: !card.editing, apiKey: "", error: null })}
                >
                  {card.editing ? "취소" : isRegistered ? "변경" : "등록"}
                </Button>
                {isRegistered && (
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 w-7 p-0"
                    disabled={card.loading}
                    onClick={() => handleDelete(provider)}
                  >
                    <Trash2 className="h-3.5 w-3.5 text-muted-foreground" />
                  </Button>
                )}
              </div>
            </div>

            {/* 펼쳐진 입력 폼 */}
            {card.editing && (
              <div className="ml-11 mt-2 mb-3 flex gap-2">
                <Input
                  type="password"
                  placeholder="API Key를 입력하세요"
                  value={card.apiKey}
                  onChange={(e) => updateCard(provider, { apiKey: e.target.value, error: null })}
                  className="flex-1 h-8 text-sm"
                  autoFocus
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleSave(provider);
                    if (e.key === "Escape") updateCard(provider, { editing: false, apiKey: "" });
                  }}
                />
                <Button
                  size="sm"
                  className="h-8"
                  disabled={card.loading || !card.apiKey.trim()}
                  onClick={() => handleSave(provider)}
                >
                  {card.loading ? "..." : "저장"}
                </Button>
              </div>
            )}

            {/* 에러/성공 메시지 */}
            {card.error && (
              <p className="ml-11 mt-1 text-xs text-destructive">{card.error}</p>
            )}
            {card.success && (
              <p className="ml-11 mt-1 text-xs text-emerald-600 dark:text-emerald-400">저장되었습니다.</p>
            )}
          </div>
        );
      })}
    </div>
  );
}
