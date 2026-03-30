"use client";

import { useEffect, useState, useCallback } from "react";
import { useRouter } from "next/navigation";
import { Plus, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/lib/store";
import { getSessions, deleteSession as deleteSessionApi } from "@/lib/api";
import type { ApiSession } from "@/lib/types";

export function SessionList() {
  const setActiveSession = useAppStore((s) => s.setActiveSession);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const sessionListVersion = useAppStore((s) => s.sessionListVersion);
  const router = useRouter();

  const [sessions, setSessions] = useState<ApiSession[]>([]);

  const fetchSessions = useCallback(async () => {
    try {
      const data = await getSessions(false);
      setSessions(data);
    } catch (err) {
      console.warn("Failed to load sessions:", err);
    }
  }, []);

  useEffect(() => {
    fetchSessions();
  }, [fetchSessions, sessionListVersion]);

  const handleNewChat = () => {
    setActiveSession(null);
    router.push("/chat");
  };

  const handleSelectSession = (sessionId: string) => {
    setActiveSession(sessionId);
    router.push("/chat");
  };

  const handleDeleteSession = async (
    e: React.MouseEvent,
    sessionId: string
  ) => {
    e.stopPropagation();
    try {
      await deleteSessionApi(sessionId);
      setSessions((prev) => prev.filter((s) => s.id !== sessionId));
      if (activeSessionId === sessionId) {
        setActiveSession(null);
      }
    } catch (err) {
      console.warn("Failed to delete session:", err);
    }
  };

  return (
    <div className="flex flex-col">
      <button
        onClick={handleNewChat}
        className="flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors"
      >
        <Plus className="h-4 w-4" />
        새 채팅
      </button>

      <div className="mt-1 space-y-0.5 overflow-y-auto flex-1">
        {sessions.map((session) => (
          <div
            key={session.id}
            className={cn(
              "group flex items-center rounded-md pr-1 transition-colors",
              activeSessionId === session.id
                ? "bg-accent text-accent-foreground"
                : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            )}
          >
            <button
              className="flex flex-1 items-center gap-3 px-3 py-1.5 text-sm min-w-0"
              onClick={() => handleSelectSession(session.id)}
            >
              <span className="truncate">{session.title ?? "새 채팅"}</span>
            </button>
            <button
              className="shrink-0 p-1 rounded opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 [@media(hover:none)]:opacity-100 hover:bg-destructive/10 hover:text-destructive transition-all"
              onClick={(e) => handleDeleteSession(e, session.id)}
              aria-label="세션 삭제"
            >
              <X className="h-3 w-3" />
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
