"use client";

import { useState, useEffect, useCallback, useRef } from "react";
import { ChevronDown } from "lucide-react";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import {
  PROVIDER_MODELS,
  PROVIDER_DEFAULT_PROFILE_NAMES,
  type ProviderName,
} from "@/lib/constants";
import { getProviderProfiles } from "@/lib/api";
import type { ApiProviderProfile } from "@/lib/types";

const PROVIDERS: ProviderName[] = ["OpenAI", "Anthropic", "Gemini"];

interface ModelSelectorProps {
  selectedProvider: ProviderName | null;
  selectedModel: string | null;
  onSelect: (provider: ProviderName, model: string, profileId: string) => void;
}

export function ModelSelector({
  selectedProvider,
  selectedModel,
  onSelect,
}: ModelSelectorProps) {
  const [profiles, setProfiles] = useState<ApiProviderProfile[]>([]);
  const [isOpen, setIsOpen] = useState(false);
  const [loading, setLoading] = useState(true);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);

  const fetchProfiles = useCallback(async () => {
    try {
      const data = await getProviderProfiles();
      setProfiles(data);

      // 초기 선택: 첫 번째 사용 가능한 Provider의 첫 모델
      if (!selectedProvider && !selectedModel) {
        for (const p of PROVIDERS) {
          const defaultName = PROVIDER_DEFAULT_PROFILE_NAMES[p];
          const profile = data.find((pr) => pr.name === defaultName);
          if (profile) {
            const firstModel = PROVIDER_MODELS[p][0];
            onSelect(p, firstModel, profile.id);
            break;
          }
        }
      }
    } catch {
      // ignore
    } finally {
      setLoading(false);
    }
  }, [selectedProvider, selectedModel, onSelect]);

  useEffect(() => {
    fetchProfiles();
  }, [fetchProfiles]);

  // 외부 클릭 시 닫기
  useEffect(() => {
    if (!isOpen) return;
    const handleClick = (e: MouseEvent) => {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(e.target as Node) &&
        buttonRef.current &&
        !buttonRef.current.contains(e.target as Node)
      ) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [isOpen]);

  // Provider별 프로필 매핑
  const getProfileForProvider = (provider: ProviderName) => {
    const defaultName = PROVIDER_DEFAULT_PROFILE_NAMES[provider];
    return profiles.find((p) => p.name === defaultName);
  };

  const availableProviders = PROVIDERS.filter((p) => getProfileForProvider(p));

  if (loading) {
    return (
      <div className="text-xs text-muted-foreground py-1">모델 불러오는 중...</div>
    );
  }

  if (availableProviders.length === 0) {
    return (
      <div className="text-xs text-muted-foreground py-1">
        <Link href="/settings" className="text-primary hover:underline">
          Settings에서 API Key를 등록하세요
        </Link>
      </div>
    );
  }

  const displayLabel = selectedProvider && selectedModel
    ? `${selectedProvider} / ${selectedModel}`
    : "모델 선택";

  return (
    <div className="relative">
      <Button
        ref={buttonRef}
        variant="outline"
        size="sm"
        className="text-xs text-muted-foreground h-7 px-3"
        onClick={() => setIsOpen((v) => !v)}
      >
        {displayLabel}
        <ChevronDown className="ml-1 h-3 w-3" />
      </Button>

      {isOpen && (
        <div
          ref={dropdownRef}
          className="absolute bottom-full left-0 mb-1 w-64 rounded-lg border bg-background shadow-lg z-20 max-h-72 overflow-y-auto"
        >
          {availableProviders.map((provider) => {
            const profile = getProfileForProvider(provider)!;
            const models = PROVIDER_MODELS[provider];

            return (
              <div key={provider}>
                <div className="px-3 py-1.5 text-xs font-semibold text-muted-foreground border-b">
                  {provider}
                </div>
                {models.map((model) => {
                  const isSelected =
                    selectedProvider === provider && selectedModel === model;
                  return (
                    <button
                      key={model}
                      className={`w-full text-left px-3 py-1.5 text-sm hover:bg-accent transition-colors ${
                        isSelected ? "bg-accent font-medium" : ""
                      }`}
                      onClick={() => {
                        onSelect(provider, model, profile.id);
                        setIsOpen(false);
                      }}
                    >
                      {model}
                    </button>
                  );
                })}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
