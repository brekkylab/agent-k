"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import type { ChatModelAdapter, AssistantRuntime } from "@assistant-ui/react";
import {
  useLocalRuntime,
  AssistantRuntimeProvider,
} from "@assistant-ui/react";
import { toast } from "sonner";
import { sendMessageStream, getSession } from "@/lib/api";
import { useAppStore } from "@/lib/store";
import type { ApiSessionMessage } from "@/lib/types";

/** Convert a DB message to initial content: plain string for user, content parts array for assistant with tool calls */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- assistant-ui initial message content accepts string | ContentPart[]
function buildInitialContent(m: ApiSessionMessage): any {
  if (m.role === "assistant" && m.tool_calls && m.tool_calls.length > 0) {
    const toolParts = m.tool_calls.map((tc) => ({
      type: "tool-call" as const,
      toolCallId: tc.id,
      toolName: tc.tool_name,
      args: tc.tool_args ?? {},
      argsText: JSON.stringify(tc.tool_args ?? {}),
      result: tc.tool_result,
    }));
    return [...toolParts, { type: "text" as const, text: m.content }];
  }
  return m.content;
}

function createApiAdapter(sessionId: string): ChatModelAdapter {
  return {
    async *run({ messages }) {
      const lastMsg = messages[messages.length - 1];
      if (!lastMsg || lastMsg.role !== "user") return;

      const textPart = lastMsg.content.find(
        (p): p is { type: "text"; text: string } => p.type === "text",
      );
      if (!textPart) return;

      // Track tool calls across events keyed by insertion order
      const toolCalls: Map<string, { toolName: string; args: unknown; result?: unknown }> = new Map();

      // Build the tool-call content parts array from current state
      function buildToolParts() {
        return [...toolCalls.entries()].map(([id, tc]) => ({
          type: "tool-call" as const,
          toolCallId: id,
          toolName: tc.toolName,
          args: tc.args ?? {},
          argsText: JSON.stringify(tc.args ?? {}),
          ...(tc.result !== undefined ? { result: tc.result } : {}),
        }));
      }

      try {
        for await (const { event, data } of sendMessageStream(sessionId, textPart.text)) {
          switch (event) {
            case "thinking":
              yield {
                content: [{ type: "text" as const, text: "" }],
                status: { type: "running" as const, reason: "thinking" },
              };
              break;
            case "tool_call": {
              const callId = `tc_${Date.now()}_${data.tool ?? "unknown"}`;
              toolCalls.set(callId, { toolName: data.tool ?? "", args: data.args });
              yield {
                // eslint-disable-next-line @typescript-eslint/no-explicit-any -- assistant-ui lacks public tool-call content-part type
                content: buildToolParts() as any,
                status: { type: "running" as const, reason: "tool-call" },
              };
              break;
            }
            case "tool_result": {
              // Update the last tool call entry that has no result yet
              const lastEntry = [...toolCalls.entries()].reverse().find(([, tc]) => tc.result === undefined);
              if (lastEntry) {
                lastEntry[1].result = data.result;
              }
              yield {
                // eslint-disable-next-line @typescript-eslint/no-explicit-any -- assistant-ui lacks public tool-call content-part type
                content: buildToolParts() as any,
                status: { type: "running" as const, reason: "tool-result" },
              };
              break;
            }
            case "message": {
              if (data.content) {
                const finalToolParts = buildToolParts();
                yield {
                  content: [
                    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- assistant-ui lacks public tool-call content-part type
                    ...(finalToolParts as any[]),
                    { type: "text" as const, text: data.content },
                  ],
                };
              }
              break;
            }
            case "done":
              // Stream complete, final message already yielded via "message" event
              break;
            case "error":
              toast.error(data.message || "메시지 전송 중 오류가 발생했습니다");
              break;
          }
        }
      } catch (err) {
        if (err instanceof Error && err.name !== "AbortError") {
          toast.error("연결이 끊어졌습니다");
        }
      }
    },
  };
}

// Inner component: only mounts AFTER messages are loaded
// This ensures useLocalRuntime receives correct initialMessages on first call
function PendingMessageSender({ runtime }: { runtime: AssistantRuntime }) {
  const pendingMessage = useAppStore((s) => s.pendingMessage);
  const setPendingMessage = useAppStore((s) => s.setPendingMessage);
  const sentRef = useRef(false);

  useEffect(() => {
    if (pendingMessage && !sentRef.current) {
      sentRef.current = true;
      // Defer to next tick so runtime is fully initialized
      queueMicrotask(() => {
        runtime.thread.append(pendingMessage);
        setPendingMessage(null);
      });
    }
  }, [pendingMessage, runtime, setPendingMessage]);

  return null;
}

function ApiRuntimeInner({
  children,
  sessionId,
  initialMessages,
}: {
  children: React.ReactNode;
  sessionId: string;
  initialMessages: { role: "user" | "assistant"; content: string; createdAt: Date }[];
}) {
  const adapter = useMemo(() => createApiAdapter(sessionId), [sessionId]);
  const runtime = useLocalRuntime(adapter, { initialMessages });

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <PendingMessageSender runtime={runtime} />
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
            content: buildInitialContent(m),
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
