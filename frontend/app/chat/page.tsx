"use client";

import { useState } from "react";
import { ApiRuntimeProvider } from "@/components/chat/api-runtime-provider";
import { ChatView } from "@/components/chat/chat-view";
import { KnowledgePanel } from "@/components/chat/knowledge-panel";
import { NewChatSetup } from "@/components/chat/new-chat-setup";
import { SessionTitle } from "@/components/chat/session-title";
import { useAppStore } from "@/lib/store";

export default function ChatPage() {
  const [isKnowledgePanelOpen, setIsKnowledgePanelOpen] = useState(false);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const setActiveSession = useAppStore((s) => s.setActiveSession);

  // 세션이 없으면 시작 화면 표시
  if (!activeSessionId) {
    return (
      <NewChatSetup
        onSessionCreated={(sessionId) => setActiveSession(sessionId)}
      />
    );
  }

  return (
    <ApiRuntimeProvider key={activeSessionId} sessionId={activeSessionId}>
      <div className="flex h-full">
        {isKnowledgePanelOpen && (
          <KnowledgePanel onClose={() => setIsKnowledgePanelOpen(false)} />
        )}
        <div className="flex-1 flex flex-col">
          <SessionTitle sessionId={activeSessionId} />
          <div className="flex-1">
            <ChatView
              onToggleKnowledgePanel={() => setIsKnowledgePanelOpen((v) => !v)}
              isKnowledgePanelOpen={isKnowledgePanelOpen}
            />
          </div>
        </div>
      </div>
    </ApiRuntimeProvider>
  );
}
