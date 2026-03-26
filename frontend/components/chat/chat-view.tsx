"use client";

import { useState, useRef, useEffect, useCallback } from "react";
import { BookOpen } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Thread } from "@/components/assistant-ui/thread";
import { KnowledgePanel } from "@/components/chat/knowledge-panel";
import { ModelSelector } from "@/components/chat/model-selector";
import { useAppStore } from "@/lib/store";
import { getSession, getAgent, updateAgent, updateSession } from "@/lib/api";
import type { ProviderName } from "@/lib/constants";

interface ChatViewProps {
  sessionId: string;
}

export function ChatView({ sessionId }: ChatViewProps) {
  const [isPanelOpen, setIsPanelOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);

  const selectedProvider = useAppStore((s) => s.selectedProvider);
  const selectedModel = useAppStore((s) => s.selectedModel);
  const setSelectedModel = useAppStore((s) => s.setSelectedModel);

  // 활성 세션의 현재 모델 로드
  const [currentAgentId, setCurrentAgentId] = useState<string | null>(null);
  const [currentProfileId, setCurrentProfileId] = useState<string | null>(null);

  useEffect(() => {
    getSession(sessionId).then((session) => {
      setCurrentAgentId(session.agent_id);
      setCurrentProfileId(session.provider_profile_id);
      getAgent(session.agent_id).then((agent) => {
        const { selectedProvider: currentProvider, setSelectedModel: setModel } =
          useAppStore.getState();
        setModel(
          currentProvider ?? "OpenAI",
          agent.spec.lm,
          session.provider_profile_id,
        );
      }).catch((err) => console.warn("Failed to load agent:", err));
    }).catch((err) => console.warn("Failed to load session:", err));
  }, [sessionId]);

  // Mid-session 모델 변경 핸들러
  const handleModelChange = useCallback(
    async (provider: ProviderName, model: string, profileId: string) => {
      const prevProvider = selectedProvider;
      const prevModel = selectedModel;
      const prevProfileId = currentProfileId;

      // Optimistic UI update
      setSelectedModel(provider, model, profileId);

      try {
        // 1. Agent 모델 변경
        if (currentAgentId) {
          await updateAgent(currentAgentId, { spec: { lm: model, tools: [] } });
        }

        // 2. Provider가 변경된 경우 Session의 provider_profile_id 업데이트
        if (profileId !== currentProfileId) {
          await updateSession(sessionId, { provider_profile_id: profileId });
          setCurrentProfileId(profileId);
        }
      } catch {
        // Rollback on error
        if (prevProvider && prevModel && prevProfileId) {
          setSelectedModel(prevProvider, prevModel, prevProfileId);
        }
      }
    },
    [sessionId, currentAgentId, currentProfileId, selectedProvider, selectedModel, setSelectedModel],
  );

  // 패널 외부 클릭 시 닫기
  useEffect(() => {
    if (!isPanelOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (
        panelRef.current &&
        !panelRef.current.contains(e.target as Node) &&
        buttonRef.current &&
        !buttonRef.current.contains(e.target as Node)
      ) {
        setIsPanelOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isPanelOpen]);

  return (
    <div className="flex flex-col h-full relative">
      <div className="absolute top-3 left-3 z-10">
        <Button
          ref={buttonRef}
          variant={isPanelOpen ? "default" : "outline"}
          size="icon"
          onClick={() => setIsPanelOpen((v) => !v)}
          aria-label="Knowledge 패널 열기"
          aria-expanded={isPanelOpen}
        >
          <BookOpen className="h-4 w-4" />
        </Button>

        {isPanelOpen && (
          <div
            ref={panelRef}
            className="absolute top-12 left-0 z-20 rounded-lg border bg-background shadow-lg"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setIsPanelOpen(false);
                buttonRef.current?.focus();
              }
            }}
          >
            <KnowledgePanel onClose={() => setIsPanelOpen(false)} />
          </div>
        )}
      </div>

      <Thread
        composerLeft={
          <ModelSelector
            selectedProvider={selectedProvider}
            selectedModel={selectedModel}
            onSelect={handleModelChange}
          />
        }
      />
    </div>
  );
}
