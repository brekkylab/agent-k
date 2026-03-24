"use client";

import { useRef } from "react";
import { X, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Separator } from "@/components/ui/separator";
import { useAppStore } from "@/lib/store";

interface KnowledgePanelProps {
  onClose: () => void;
}

export function KnowledgePanel({ onClose }: KnowledgePanelProps) {
  const knowledges = useAppStore((s) => s.knowledges);
  const activeSessionId = useAppStore((s) => s.activeSessionId);
  const getSessionLocalData = useAppStore((s) => s.getSessionLocalData);
  const updateSessionKnowledge = useAppStore((s) => s.updateSessionKnowledge);
  const addSessionDocument = useAppStore((s) => s.addSessionDocument);
  const removeSessionDocument = useAppStore((s) => s.removeSessionDocument);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const localData = activeSessionId
    ? getSessionLocalData(activeSessionId)
    : { knowledgeIds: [], sessionDocuments: [] };

  const selectedKnowledgeIds = localData.knowledgeIds;
  const sessionDocs = localData.sessionDocuments;
  const allSelected =
    knowledges.length > 0 &&
    knowledges.every((k) => selectedKnowledgeIds.includes(k.id));

  const toggleKnowledge = (knowledgeId: string) => {
    if (!activeSessionId) return;
    const updated = selectedKnowledgeIds.includes(knowledgeId)
      ? selectedKnowledgeIds.filter((id) => id !== knowledgeId)
      : [...selectedKnowledgeIds, knowledgeId];
    updateSessionKnowledge(activeSessionId, updated);
  };

  const toggleAll = () => {
    if (!activeSessionId) return;
    if (allSelected) {
      updateSessionKnowledge(activeSessionId, []);
    } else {
      updateSessionKnowledge(
        activeSessionId,
        knowledges.map((k) => k.id),
      );
    }
  };

  const handleExtraFile = (files: FileList) => {
    if (!activeSessionId) return;
    Array.from(files).forEach((file) => {
      addSessionDocument(activeSessionId, {
        name: file.name,
        size: file.size,
      });
    });
    if (fileInputRef.current) {
      fileInputRef.current.value = "";
    }
  };

  return (
    <div className="w-64 border-r bg-background flex flex-col">
      <div className="flex items-center justify-between p-3 border-b">
        <span className="font-semibold text-sm">Knowledge</span>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={onClose}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <label className="flex items-center gap-2 cursor-pointer">
          <Checkbox checked={allSelected} onCheckedChange={toggleAll} />
          <span className="text-sm font-medium">전체 문서 사용</span>
        </label>

        <Separator />

        <div className="space-y-2">
          {knowledges.map((kn) => (
            <label
              key={kn.id}
              className={`flex items-center gap-2 rounded-md p-2 cursor-pointer transition-colors ${
                selectedKnowledgeIds.includes(kn.id)
                  ? "bg-primary/10"
                  : "hover:bg-accent"
              }`}
            >
              <Checkbox
                checked={selectedKnowledgeIds.includes(kn.id)}
                onCheckedChange={() => toggleKnowledge(kn.id)}
              />
              <span className="text-sm">{kn.name}</span>
            </label>
          ))}
        </div>

        <Separator />

        <div>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            className="hidden"
            onChange={(e) => e.target.files && handleExtraFile(e.target.files)}
          />
          <Button
            variant="outline"
            size="sm"
            className="w-full"
            onClick={() => fileInputRef.current?.click()}
          >
            <Plus className="h-3 w-3 mr-1" /> 별도 문서 포함
          </Button>
          {sessionDocs.length > 0 && (
            <div className="mt-2 space-y-1">
              {sessionDocs.map((doc, i) => (
                <div
                  key={i}
                  className="flex items-center justify-between text-xs text-muted-foreground px-1 group"
                >
                  <span className="truncate">{doc.name}</span>
                  <button
                    className="opacity-0 group-hover:opacity-100 p-0.5 rounded hover:bg-destructive/10 hover:text-destructive transition-all"
                    onClick={() =>
                      activeSessionId &&
                      removeSessionDocument(activeSessionId, i)
                    }
                    title="문서 제거"
                  >
                    <Trash2 className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
