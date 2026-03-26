"use client";

import { useState, useRef, useEffect } from "react";
import { Pencil } from "lucide-react";
import { updateSessionTitle, getSession, ApiError } from "@/lib/api";
import { useAppStore } from "@/lib/store";

interface SessionTitleProps {
  sessionId: string;
}

export function SessionTitle({ sessionId }: SessionTitleProps) {
  const [title, setTitle] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [editValue, setEditValue] = useState("");
  const [saving, setSaving] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Load title from backend
  useEffect(() => {
    getSession(sessionId)
      .then((session) => setTitle(session.title))
      .catch(() => {});
  }, [sessionId]);

  // Focus input when editing starts
  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  const startEditing = () => {
    setEditValue(title ?? "");
    setEditing(true);
  };

  const cancelEditing = () => {
    setEditing(false);
  };

  const saveTitle = async () => {
    const trimmed = editValue.trim();
    if (!trimmed || trimmed === title) {
      setEditing(false);
      return;
    }

    setSaving(true);
    try {
      await updateSessionTitle(sessionId, trimmed);
      setTitle(trimmed);
      setEditing(false);
      useAppStore.getState().bumpSessionListVersion();
    } catch (err) {
      if (err instanceof ApiError) {
        console.error("Failed to update title:", err.message);
      }
    } finally {
      setSaving(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      saveTitle();
    } else if (e.key === "Escape") {
      cancelEditing();
    }
  };

  const displayTitle = title ?? "새 채팅";

  if (editing) {
    return (
      <div className="flex items-center gap-2 px-4 py-2 border-b">
        <input
          ref={inputRef}
          type="text"
          value={editValue}
          onChange={(e) => setEditValue(e.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={saveTitle}
          disabled={saving}
          className="flex-1 text-sm font-medium bg-transparent border-b border-primary outline-none"
          placeholder="세션 제목 입력..."
        />
      </div>
    );
  }

  return (
    <button
      className="flex w-full items-center gap-2 px-4 py-2 border-b cursor-pointer group text-left hover:bg-accent/50 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      onClick={startEditing}
      aria-label={`세션 제목: ${displayTitle}. 클릭하여 편집`}
    >
      <span className="text-sm font-medium truncate">{displayTitle}</span>
      <Pencil className="h-3 w-3 shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100 transition-opacity" />
    </button>
  );
}
