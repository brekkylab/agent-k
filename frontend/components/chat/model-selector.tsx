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

interface ModelOption {
  provider: ProviderName;
  model: string;
  profileId: string;
}

export function ModelSelector({
  selectedProvider,
  selectedModel,
  onSelect,
}: ModelSelectorProps) {
  const [profiles, setProfiles] = useState<ApiProviderProfile[]>([]);
  const [isOpen, setIsOpen] = useState(false);
  const [loading, setLoading] = useState(true);
  const [focusIndex, setFocusIndex] = useState(-1);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const optionRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const fetchProfiles = useCallback(async () => {
    try {
      const data = await getProviderProfiles();
      setProfiles(data);

      if (!selectedProvider && !selectedModel) {
        for (const p of PROVIDERS) {
          const defaultName = PROVIDER_DEFAULT_PROFILE_NAMES[p];
          const profile = data.find((pr) => pr.name === defaultName);
          if (profile) {
            onSelect(p, PROVIDER_MODELS[p][0], profile.id);
            break;
          }
        }
      }
    } catch (err: unknown) {
      console.warn("Failed to load provider profiles:", err);
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

  const getProfileForProvider = (provider: ProviderName) => {
    const defaultName = PROVIDER_DEFAULT_PROFILE_NAMES[provider];
    return profiles.find((p) => p.name === defaultName);
  };

  const availableProviders = PROVIDERS.filter((p) => getProfileForProvider(p));

  // 평탄화된 옵션 목록 (키보드 내비게이션용)
  const flatOptions: ModelOption[] = availableProviders.flatMap((provider) => {
    const profile = getProfileForProvider(provider)!;
    return PROVIDER_MODELS[provider].map((model) => ({
      provider,
      model,
      profileId: profile.id,
    }));
  });

  const handleToggle = () => {
    setIsOpen((v) => {
      if (!v) setFocusIndex(-1);
      return !v;
    });
  };

  const handleSelect = (option: ModelOption) => {
    onSelect(option.provider, option.model, option.profileId);
    setIsOpen(false);
    buttonRef.current?.focus();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (!isOpen) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        setIsOpen(true);
        setFocusIndex(0);
      }
      return;
    }

    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setFocusIndex((i) => {
          const next = Math.min(i + 1, flatOptions.length - 1);
          optionRefs.current[next]?.focus();
          return next;
        });
        break;
      case "ArrowUp":
        e.preventDefault();
        setFocusIndex((i) => {
          const next = Math.max(i - 1, 0);
          optionRefs.current[next]?.focus();
          return next;
        });
        break;
      case "Escape":
        e.preventDefault();
        setIsOpen(false);
        buttonRef.current?.focus();
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        if (focusIndex >= 0 && focusIndex < flatOptions.length) {
          handleSelect(flatOptions[focusIndex]);
        }
        break;
    }
  };

  // 옵션이 열릴 때 첫 항목에 포커스
  useEffect(() => {
    if (isOpen && focusIndex >= 0) {
      optionRefs.current[focusIndex]?.focus();
    }
  }, [isOpen, focusIndex]);

  if (loading) {
    return <div className="text-xs text-muted-foreground py-1">모델 불러오는 중...</div>;
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

  const displayLabel =
    selectedProvider && selectedModel
      ? `${selectedProvider} / ${selectedModel}`
      : "모델 선택";

  let optionIndex = 0;

  return (
    <div className="relative" onKeyDown={handleKeyDown}>
      <Button
        ref={buttonRef}
        variant="outline"
        size="sm"
        className="text-xs text-muted-foreground h-7 px-3"
        onClick={handleToggle}
        aria-haspopup="listbox"
        aria-expanded={isOpen}
        aria-label={`모델 선택: ${displayLabel}`}
      >
        {displayLabel}
        <ChevronDown className="ml-1 h-3 w-3" />
      </Button>

      {isOpen && (
        <div
          ref={dropdownRef}
          role="listbox"
          aria-label="모델 목록"
          className="absolute bottom-full left-0 mb-1 w-64 rounded-lg border bg-background shadow-lg z-20 max-h-72 overflow-y-auto"
        >
          {availableProviders.map((provider) => {
            const models = PROVIDER_MODELS[provider];

            return (
              <div key={provider} role="group" aria-label={provider}>
                <div className="px-3 py-1.5 text-xs font-semibold text-muted-foreground border-b">
                  {provider}
                </div>
                {models.map((model) => {
                  const isSelected =
                    selectedProvider === provider && selectedModel === model;
                  const currentIndex = optionIndex++;
                  const option = flatOptions[currentIndex];

                  return (
                    <button
                      key={model}
                      ref={(el) => { optionRefs.current[currentIndex] = el; }}
                      role="option"
                      aria-selected={isSelected}
                      tabIndex={focusIndex === currentIndex ? 0 : -1}
                      className={`w-full text-left px-3 py-1.5 text-sm hover:bg-accent transition-colors outline-none focus-visible:bg-accent ${
                        isSelected ? "bg-accent font-medium" : ""
                      }`}
                      onClick={() => handleSelect(option)}
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
