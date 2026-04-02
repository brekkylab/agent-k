"use client";

import { useState, useEffect, useRef } from "react";
import { ArrowUpIcon, Loader2 } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { ModelSelector } from "@/components/chat/model-selector";
import { useAppStore } from "@/lib/store";
import {
  createAgent,
  createSession,
  sendMessageStream,
  ApiError,
} from "@/lib/api";

export function NewChatWelcome() {
  const [message, setMessage] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  // loading state와 별도로 ref로 중복 방지 — React 비동기 state 업데이트 사이의 race condition 방어
  const creatingRef = useRef(false);

  const speedwagons = useAppStore((s) => s.speedwagons);
  const fetchSpeedwagons = useAppStore((s) => s.fetchSpeedwagons);
  const selectedProvider = useAppStore((s) => s.selectedProvider);
  const selectedModel = useAppStore((s) => s.selectedModel);
  const selectedProfileId = useAppStore((s) => s.selectedProfileId);
  const setSelectedModel = useAppStore((s) => s.setSelectedModel);
  const pendingSpeedwagonIds = useAppStore((s) => s.pendingSpeedwagonIds);
  const setPendingSpeedwagonIds = useAppStore((s) => s.setPendingSpeedwagonIds);
  const setActiveSession = useAppStore((s) => s.setActiveSession);
  const bumpSessionListVersion = useAppStore((s) => s.bumpSessionListVersion);

  useEffect(() => {
    textareaRef.current?.focus();
    fetchSpeedwagons();
  }, [fetchSpeedwagons]);

  const handleSubmit = async () => {
    if (!message.trim() || !selectedModel || !selectedProfileId) return;
    if (creatingRef.current) return; // 중복 호출 방지
    creatingRef.current = true;

    setError(null);
    setLoading(true);

    try {
      const systemMessage = localStorage.getItem("agentwebui_system_message") || undefined;
      const agent = await createAgent({
        spec: {
          lm: selectedModel,
          instruction: systemMessage?.trim() || undefined,
          tools: [],
        },
      });

      const session = await createSession({
        agent_id: agent.id,
        provider_profile_id: selectedProfileId,
        speedwagon_ids: pendingSpeedwagonIds.length > 0 ? pendingSpeedwagonIds : undefined,
      });

      // pendingSpeedwagonIds 초기화
      if (pendingSpeedwagonIds.length > 0) {
        setPendingSpeedwagonIds([]);
      }

      // 메시지 전송
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      for await (const _ of sendMessageStream(session.id, message.trim())) { /* drain stream */ }

      // 세션 활성화 → ChatView로 전환
      bumpSessionListVersion();
      setActiveSession(session.id);
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message);
      } else {
        setError("세션 생성에 실패했습니다. Settings에서 API Key를 확인해주세요.");
      }
    } finally {
      creatingRef.current = false;
      setLoading(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const canSubmit = message.trim() && selectedModel && selectedProfileId && !loading;

  return (
    <div className="relative flex h-full flex-col items-center justify-center px-4 overflow-hidden">
      {/* Aurora Background */}
      <div className="pointer-events-none absolute inset-0 -z-10 overflow-hidden">
        <div className="absolute -top-[20%] left-[5%] h-[40%] w-[120%] animate-aurora-1 rounded-[50%] bg-gradient-to-r from-violet-500/15 via-cyan-400/10 to-transparent blur-[40px]" />
        <div className="absolute top-[15%] -left-[10%] h-[35%] w-[130%] animate-aurora-2 rounded-[50%] bg-gradient-to-r from-emerald-400/10 via-blue-500/12 to-purple-500/8 blur-[50px]" />
        <div className="absolute top-[40%] left-[10%] h-[30%] w-[110%] animate-aurora-3 rounded-[50%] bg-gradient-to-r from-cyan-400/8 via-indigo-500/10 to-transparent blur-[40px]" />
      </div>

      <div className="w-full max-w-2xl space-y-6">
        {/* Welcome */}
        <div className="text-center space-y-2">
          <h2 className="text-xl font-semibold text-muted-foreground">
            무엇이든 물어보세요
          </h2>
        </div>

        {/* Composer */}
        <div className="relative flex w-full flex-col rounded-2xl border border-input bg-background px-1 pt-2 shadow-sm transition-colors focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/50">
          <textarea
            ref={textareaRef}
            placeholder="메시지를 입력하세요..."
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            onKeyDown={handleKeyDown}
            className="mb-1 max-h-32 min-h-[60px] w-full resize-none bg-transparent px-3.5 pt-1.5 pb-3 text-base outline-none placeholder:text-muted-foreground"
            rows={1}
            disabled={loading}
          />
          <div className="mx-1 mb-2 flex items-center justify-between">
            <ModelSelector
              selectedProvider={selectedProvider}
              selectedModel={selectedModel}
              onSelect={setSelectedModel}
            />
            <button
              onClick={handleSubmit}
              disabled={!canSubmit}
              aria-label="메시지 전송"
              className="inline-flex items-center justify-center h-8 w-8 rounded-full bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {loading ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <ArrowUpIcon className="h-4 w-4" />
              )}
            </button>
          </div>
        </div>

        {/* Speedwagon 선택 */}
        {speedwagons.length > 0 && (
          <div className="rounded-xl border border-input bg-background/60 px-4 py-3 space-y-2">
            <span className="text-xs font-medium text-muted-foreground">Speedwagon</span>
            <div className="space-y-1.5">
              {speedwagons.map((sw) => {
                const isBuilt = sw.index_status === "indexed";
                const isChecked = pendingSpeedwagonIds.includes(sw.id);
                return (
                  <label
                    key={sw.id}
                    className={`flex items-center gap-2 rounded-md px-2 py-1.5 transition-colors ${
                      isBuilt
                        ? "cursor-pointer hover:bg-accent"
                        : "opacity-50 cursor-not-allowed"
                    } ${isChecked && isBuilt ? "bg-primary/10" : ""}`}
                    title={!isBuilt ? "인덱싱 후 사용 가능합니다" : undefined}
                  >
                    <Checkbox
                      checked={isChecked}
                      disabled={!isBuilt}
                      onCheckedChange={() => {
                        if (!isBuilt) return;
                        const updated = isChecked
                          ? pendingSpeedwagonIds.filter((id) => id !== sw.id)
                          : [...pendingSpeedwagonIds, sw.id];
                        setPendingSpeedwagonIds(updated);
                      }}
                    />
                    <span className="text-sm flex-1 truncate">{sw.name}</span>
                    {!isBuilt && (
                      <span className="text-xs text-muted-foreground">
                        ({sw.index_status === "indexing" ? "인덱싱 중" : "인덱싱 필요"})
                      </span>
                    )}
                  </label>
                );
              })}
            </div>
          </div>
        )}

        {/* Error */}
        {error && (
          <p className="text-sm text-destructive text-center">{error}</p>
        )}
      </div>
    </div>
  );
}
