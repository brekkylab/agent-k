"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { MessageSquare, KeyRound, WifiOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import { PROVIDER_MODELS, type ProviderName } from "@/lib/constants";
import {
  getProviderProfiles,
  createAgent,
  createSession,
  ApiError,
} from "@/lib/api";
import type { ApiProviderProfile } from "@/lib/types";
import { useAppStore } from "@/lib/store";

const PROVIDER_NAMES = Object.keys(PROVIDER_MODELS) as ProviderName[];

type LoadState = "loading" | "connected" | "no-profiles" | "network-error";

export function NewChatSetup({
  onSessionCreated,
}: {
  onSessionCreated: (sessionId: string) => void;
}) {
  const [profiles, setProfiles] = useState<ApiProviderProfile[]>([]);
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [selectedProfileId, setSelectedProfileId] = useState<string>("");
  const [selectedProvider, setSelectedProvider] = useState<ProviderName>("OpenAI");
  const [selectedModel, setSelectedModel] = useState<string>(PROVIDER_MODELS.OpenAI[0]);
  const [instruction, setInstruction] = useState("");
  const [selectedKnowledgeIds, setSelectedKnowledgeIds] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const knowledges = useAppStore((s) => s.knowledges);

  const fetchProfiles = () => {
    setLoadState("loading");
    getProviderProfiles()
      .then((data) => {
        setProfiles(data);
        if (data.length > 0) {
          setSelectedProfileId(data[0].id);
          setLoadState("connected");
        } else {
          setLoadState("no-profiles");
        }
      })
      .catch((err) => {
        if (err instanceof ApiError && err.type === "network") {
          setLoadState("network-error");
        } else {
          setLoadState("network-error");
        }
      });
  };

  useEffect(() => {
    fetchProfiles();
  }, []);

  const handleProviderChange = (provider: ProviderName) => {
    setSelectedProvider(provider);
    setSelectedModel(PROVIDER_MODELS[provider][0]);
  };

  const toggleKnowledge = (id: string) => {
    setSelectedKnowledgeIds((prev) =>
      prev.includes(id) ? prev.filter((k) => k !== id) : [...prev, id],
    );
  };

  const handleStart = async () => {
    setError(null);
    setLoading(true);

    try {
      const agent = await createAgent({
        spec: {
          lm: selectedModel,
          instruction: instruction.trim() || undefined,
          tools: [],
        },
      });

      const session = await createSession({
        agent_id: agent.id,
        provider_profile_id: selectedProfileId || undefined,
      });

      if (selectedKnowledgeIds.length > 0) {
        useAppStore
          .getState()
          .updateSessionKnowledge(session.id, selectedKnowledgeIds);
      }

      onSessionCreated(session.id);
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message);
      } else {
        setError("세션 생성에 실패했습니다.");
      }
    } finally {
      setLoading(false);
    }
  };

  // P0-3: Backend 미연결 상태
  if (loadState === "network-error") {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="w-full max-w-md text-center space-y-4">
          <WifiOff className="mx-auto h-10 w-10 text-muted-foreground" />
          <h2 className="text-xl font-semibold">Backend 서버에 연결할 수 없습니다</h2>
          <p className="text-sm text-muted-foreground">
            서버가 실행 중인지 확인해 주세요.
            <br />
            <code className="text-xs bg-muted px-1.5 py-0.5 rounded">
              cargo run
            </code>
            {" "}으로 Backend를 시작할 수 있습니다.
          </p>
          <Button variant="outline" onClick={fetchProfiles}>
            다시 시도
          </Button>
        </div>
      </div>
    );
  }

  // P0-2: Provider Profile 미등록 상태
  if (loadState === "no-profiles") {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="w-full max-w-md text-center space-y-4">
          <KeyRound className="mx-auto h-10 w-10 text-muted-foreground" />
          <h2 className="text-xl font-semibold">LLM Provider를 등록해 주세요</h2>
          <p className="text-sm text-muted-foreground">
            AI 모델을 사용하려면 먼저 Provider의 API Key를 등록해야 합니다.
          </p>
          <Link href="/settings">
            <Button>Settings에서 등록하기</Button>
          </Link>
        </div>
      </div>
    );
  }

  // 로딩 중
  if (loadState === "loading") {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-sm text-muted-foreground">불러오는 중...</p>
      </div>
    );
  }

  // 정상 상태: Provider Profile이 1개 이상
  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="w-full max-w-lg space-y-6">
        <div className="text-center space-y-2">
          <MessageSquare className="mx-auto h-10 w-10 text-muted-foreground" />
          <h2 className="text-xl font-semibold">새 대화 시작</h2>
          <p className="text-sm text-muted-foreground">
            모델과 프롬프트를 설정하고 대화를 시작하세요.
          </p>
        </div>

        <div className="space-y-4">
          {/* Provider Profile 선택 */}
          <div className="space-y-2">
            <label className="text-sm font-medium">Provider Profile</label>
            <select
              value={selectedProfileId}
              onChange={(e) => setSelectedProfileId(e.target.value)}
              className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              {profiles.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </div>

          {/* 모델 선택 */}
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <label className="text-sm font-medium">Provider</label>
              <select
                value={selectedProvider}
                onChange={(e) =>
                  handleProviderChange(e.target.value as ProviderName)
                }
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
              <label className="text-sm font-medium">모델</label>
              <select
                value={selectedModel}
                onChange={(e) => setSelectedModel(e.target.value)}
                className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              >
                {PROVIDER_MODELS[selectedProvider].map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {/* 시스템 프롬프트 */}
          <div className="space-y-2">
            <label className="text-sm font-medium">
              시스템 프롬프트{" "}
              <span className="text-muted-foreground font-normal">(선택)</span>
            </label>
            <Textarea
              placeholder="AI에게 역할이나 지침을 설정합니다..."
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              rows={3}
            />
          </div>

          {/* Knowledge 선택 */}
          {knowledges.length > 0 && (
            <div className="space-y-2">
              <label className="text-sm font-medium">
                Knowledge{" "}
                <span className="text-muted-foreground font-normal">
                  (선택)
                </span>
              </label>
              <div className="space-y-2 rounded-md border p-3">
                {knowledges.map((k) => (
                  <label
                    key={k.id}
                    className="flex items-center gap-2 text-sm cursor-pointer"
                  >
                    <Checkbox
                      checked={selectedKnowledgeIds.includes(k.id)}
                      onCheckedChange={() => toggleKnowledge(k.id)}
                    />
                    {k.name}
                  </label>
                ))}
              </div>
            </div>
          )}

          {error && <p className="text-sm text-destructive">{error}</p>}

          {/* P0-4: Provider Profile 없으면 disabled */}
          <Button
            onClick={handleStart}
            disabled={loading || !selectedProfileId}
            className="w-full"
          >
            {loading ? "생성 중..." : "대화 시작"}
          </Button>
        </div>
      </div>
    </div>
  );
}
