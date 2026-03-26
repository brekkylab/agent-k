"use client";

import { ApiRuntimeProvider } from "@/components/chat/api-runtime-provider";
import { ChatView } from "@/components/chat/chat-view";
import { NewChatWelcome } from "@/components/chat/new-chat-welcome";
import { SessionTitle } from "@/components/chat/session-title";
import { useAppStore } from "@/lib/store";

export default function ChatPage() {
  const activeSessionId = useAppStore((s) => s.activeSessionId);

  if (!activeSessionId) {
    return <NewChatWelcome />;
  }

  return (
    <ApiRuntimeProvider key={activeSessionId} sessionId={activeSessionId}>
      <div className="flex-1 flex flex-col overflow-hidden h-full">
        <SessionTitle sessionId={activeSessionId} />
        <div className="flex-1 overflow-hidden">
          <ChatView sessionId={activeSessionId} />
        </div>
      </div>
    </ApiRuntimeProvider>
  );
}
