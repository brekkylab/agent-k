"use client";

import { useState, useRef, useEffect, useCallback } from "react";
import { BookOpen } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Thread } from "@/components/assistant-ui/thread";
import { SpeedwagonPanel } from "@/components/chat/speedwagon-panel";
import { IndexStatusBanner } from "@/components/chat/index-status-banner";
import { ModelSelector } from "@/components/chat/model-selector";
import { useAppStore } from "@/lib/store";
import { toast } from "sonner";
import { getSession, getAgent, createAgent, updateSession } from "@/lib/api";
import type { ProviderName } from "@/lib/constants";

interface ChatViewProps {
  sessionId: string;
}

export function ChatView({ sessionId }: ChatViewProps) {
  const [isPanelOpen, setIsPanelOpen] = useState(false);
  const [sessionSpeedwagonIds, setSessionSpeedwagonIds] = useState<string[]>([]);
  const [sessionSourceIds, setSessionSourceIds] = useState<string[]>([]);
  const panelRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);

  const selectedProvider = useAppStore((s) => s.selectedProvider);
  const selectedModel = useAppStore((s) => s.selectedModel);
  const setSelectedModel = useAppStore((s) => s.setSelectedModel);

  // 활성 세션의 현재 모델/speedwagon 로드
  const [currentAgentId, setCurrentAgentId] = useState<string | null>(null);
  const [currentProfileId, setCurrentProfileId] = useState<string | null>(null);

  useEffect(() => {
    getSession(sessionId).then((session) => {
      setCurrentAgentId(session.agent_id);
      setCurrentProfileId(session.provider_profile_id);
      setSessionSpeedwagonIds(session.speedwagon_ids ?? []);
      setSessionSourceIds(session.source_ids ?? []);
      getAgent(session.agent_id).then((agent) => {
        const { selectedProvider: currentProvider, setSelectedModel: setModel } =
          useAppStore.getState();
        setModel(
          currentProvider ?? "OpenAI",
          agent.spec.lm,
          session.provider_profile_id,
        );
      }).catch(() => toast.error("에이전트 로드에 실패했습니다"));
    }).catch(() => toast.error("세션 로드에 실패했습니다"));
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
        // 1. 새 Agent 생성 (copy-on-write)
        const agent = await createAgent({ spec: { lm: model, tools: [] } });

        // 2. Session에 새 Agent 연결 (agent_id 변경된 경우) + Provider 변경 시 provider_profile_id도 업데이트
        const sessionUpdate: { agent_id?: string; provider_profile_id?: string } = {};
        if (agent.id !== currentAgentId) {
          sessionUpdate.agent_id = agent.id;
        }
        if (profileId !== currentProfileId) {
          sessionUpdate.provider_profile_id = profileId;
        }
        if (Object.keys(sessionUpdate).length > 0) {
          await updateSession(sessionId, sessionUpdate);
        }

        // Update local state only after successful backend call
        if (sessionUpdate.agent_id) {
          setCurrentAgentId(agent.id);
        }
        if (sessionUpdate.provider_profile_id) {
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
      {/* 인덱싱 상태 배너 */}
      <IndexStatusBanner sessionSpeedwagonIds={sessionSpeedwagonIds} />

      <div className="absolute top-3 left-3 z-10">
        <Button
          ref={buttonRef}
          variant={isPanelOpen ? "default" : "outline"}
          size="icon"
          onClick={() => setIsPanelOpen((v) => !v)}
          aria-label="Speedwagon 패널 열기"
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
            <SpeedwagonPanel
              onClose={() => setIsPanelOpen(false)}
              sessionId={sessionId}
              initialSpeedwagonIds={sessionSpeedwagonIds}
              initialSourceIds={sessionSourceIds}
            />
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
