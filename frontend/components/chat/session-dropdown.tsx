"use client";

import { useEffect, useState, useCallback } from "react";
import { useRouter } from "next/navigation";
import { ChevronDown, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useAppStore } from "@/lib/store";
import { getSessions, deleteSession as deleteSessionApi } from "@/lib/api";
import type { ApiSession } from "@/lib/types";

export function SessionDropdown() {
  const setActiveSession = useAppStore((s) => s.setActiveSession);
  const removeSessionLocalData = useAppStore((s) => s.removeSessionLocalData);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const sessionListVersion = useAppStore((s) => s.sessionListVersion);
  const router = useRouter();

  const [sessions, setSessions] = useState<ApiSession[]>([]);

  const fetchSessions = useCallback(async () => {
    try {
      const data = await getSessions(false);
      setSessions(data);
    } catch {
      // Backend 미연결 시 빈 목록 유지
    }
  }, []);

  // 드롭다운이 열릴 때마다 갱신 (open state로 관리하지 않고 mount + activeSessionId 변경 시)
  useEffect(() => {
    fetchSessions();
  }, [fetchSessions, activeSessionId, sessionListVersion]);

  const handleNewChat = () => {
    setActiveSession(null);
    router.push("/chat");
  };

  const handleSelectSession = (sessionId: string) => {
    setActiveSession(sessionId);
    router.push("/chat");
  };

  const handleDeleteSession = async (sessionId: string) => {
    try {
      await deleteSessionApi(sessionId);
      setSessions((prev) => prev.filter((s) => s.id !== sessionId));
      removeSessionLocalData(sessionId);
      if (activeSessionId === sessionId) {
        setActiveSession(null);
      }
    } catch {
      // 삭제 실패 시 무시
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={<Button variant="ghost" size="icon" className="h-7 w-7" />}
      >
        <ChevronDown className="h-3 w-3" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-48">
        <DropdownMenuItem onClick={handleNewChat}>
          새 채팅
        </DropdownMenuItem>
        {sessions.length > 0 && <DropdownMenuSeparator />}
        <div className="max-h-48 overflow-y-auto">
          {sessions.map((session) => (
            <div key={session.id} className="flex items-center">
              <DropdownMenuItem
                className="flex-1"
                onClick={() => handleSelectSession(session.id)}
              >
                <span className="truncate">
                  {session.title ?? "새 채팅"}
                </span>
              </DropdownMenuItem>
              <button
                className="mx-1 p-1 rounded hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-colors"
                onClick={(e) => {
                  e.stopPropagation();
                  handleDeleteSession(session.id);
                }}
                title="세션 삭제"
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          ))}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
