"use client";

import { useEffect, useState } from "react";
import type { ChatModelAdapter } from "@assistant-ui/react";
import {
  useLocalRuntime,
  AssistantRuntimeProvider,
} from "@assistant-ui/react";
import { sendMessage, getSession } from "@/lib/api";

function createApiAdapter(sessionId: string): ChatModelAdapter {
  return {
    async *run({ messages }) {
      const lastMsg = messages[messages.length - 1];
      if (!lastMsg || lastMsg.role !== "user") return;

      const textPart = lastMsg.content.find(
        (p): p is { type: "text"; text: string } => p.type === "text",
      );
      if (!textPart) return;

      const response = await sendMessage(sessionId, textPart.text);

      if (response.assistant_message) {
        yield {
          content: [
            { type: "text" as const, text: response.assistant_message.content },
          ],
        };
      }
    },
  };
}

// Inner component: only mounts AFTER messages are loaded
// This ensures useLocalRuntime receives correct initialMessages on first call
function ApiRuntimeInner({
  children,
  sessionId,
  initialMessages,
}: {
  children: React.ReactNode;
  sessionId: string;
  initialMessages: { role: "user" | "assistant"; content: string; createdAt: Date }[];
}) {
  const adapter = createApiAdapter(sessionId);
  const runtime = useLocalRuntime(adapter, { initialMessages });

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      {children}
    </AssistantRuntimeProvider>
  );
}

// Outer component: handles message loading
export function ApiRuntimeProvider({
  children,
  sessionId,
}: {
  children: React.ReactNode;
  sessionId: string;
}) {
  const [initialMessages, setInitialMessages] = useState<
    { role: "user" | "assistant"; content: string; createdAt: Date }[] | null
  >(null);

  useEffect(() => {
    let cancelled = false;

    getSession(sessionId)
      .then((session) => {
        if (cancelled) return;
        const msgs = (session.messages ?? [])
          .filter((m) => m.role === "user" || m.role === "assistant")
          .map((m) => ({
            role: m.role as "user" | "assistant",
            content: m.content,
            createdAt: new Date(m.created_at),
          }));
        setInitialMessages(msgs);
      })
      .catch(() => {
        if (!cancelled) setInitialMessages([]);
      });

    return () => {
      cancelled = true;
    };
  }, [sessionId]);

  if (initialMessages === null) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-sm text-muted-foreground">대화를 불러오는 중...</p>
      </div>
    );
  }

  return (
    <ApiRuntimeInner key={sessionId} sessionId={sessionId} initialMessages={initialMessages}>
      {children}
    </ApiRuntimeInner>
  );
}
