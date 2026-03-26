"use client";

import { useState, useEffect } from "react";
import { Info, TriangleAlert } from "lucide-react";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { ProviderSettings } from "@/components/settings/provider-settings";

const SYSTEM_MESSAGE_KEY = "agentwebui_system_message";

export default function SettingsPage() {
  const [systemMessage, setSystemMessage] = useState("");
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    const stored = localStorage.getItem(SYSTEM_MESSAGE_KEY);
    if (stored) setSystemMessage(stored);
  }, []);

  const handleSave = () => {
    localStorage.setItem(SYSTEM_MESSAGE_KEY, systemMessage);
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  };

  return (
    <div className="mx-auto max-w-2xl p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">Settings</h1>
        <p className="text-sm text-muted-foreground mt-1">
          Provider API Key와 시스템 메시지를 관리합니다.
        </p>
      </div>

      {/* Provider API Keys */}
      <div className="space-y-3">
        <h2 className="text-sm font-medium">Provider API Keys</h2>
        <ProviderSettings />
      </div>

      <Separator />

      {/* System Message */}
      <div className="space-y-3">
        <h2 className="text-sm font-medium">시스템 메시지</h2>
        <Textarea
          placeholder="모든 새 채팅에 적용될 시스템 프롬프트를 입력하세요..."
          value={systemMessage}
          onChange={(e) => setSystemMessage(e.target.value)}
          rows={4}
        />
        <div className="flex items-center justify-between">
          <div className="space-y-1.5">
            <p className="flex items-center gap-1.5 text-xs text-amber-600 dark:text-amber-400">
              <TriangleAlert className="h-3 w-3" />
              새로 시작하는 채팅에만 적용됩니다. 기존 세션에는 영향 없습니다.
            </p>
            <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Info className="h-3 w-3" />
              현재 브라우저의 localStorage에 저장됩니다. 브라우저 데이터 초기화 시 사라집니다.
            </p>
          </div>
          <Button
            size="sm"
            onClick={handleSave}
            className="shrink-0"
          >
            {saved ? "저장됨" : "저장"}
          </Button>
        </div>
      </div>
    </div>
  );
}
