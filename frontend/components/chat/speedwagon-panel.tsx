"use client";

import { useState, useEffect } from "react";
import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Separator } from "@/components/ui/separator";
import { useAppStore } from "@/lib/store";
import { updateSession } from "@/lib/api";
import type { ApiSpeedwagon } from "@/lib/types";
import { toast } from "sonner";

interface SpeedwagonPanelProps {
  onClose: () => void;
  sessionId: string;
  initialSpeedwagonIds: string[];
  initialSourceIds: string[];
}

export function SpeedwagonPanel({
  onClose,
  sessionId,
  initialSpeedwagonIds,
  initialSourceIds,
}: SpeedwagonPanelProps) {
  const speedwagons = useAppStore((s) => s.speedwagons);
  const sources = useAppStore((s) => s.sources);
  const fetchSpeedwagons = useAppStore((s) => s.fetchSpeedwagons);
  const fetchSources = useAppStore((s) => s.fetchSources);
  const [selectedSpeedwagonIds, setSelectedSpeedwagonIds] = useState<string[]>(initialSpeedwagonIds);
  const [selectedSourceIds, setSelectedSourceIds] = useState<string[]>(initialSourceIds);
  const [saving, setSaving] = useState(false);
  const bumpSessionListVersion = useAppStore((s) => s.bumpSessionListVersion);

  useEffect(() => {
    fetchSpeedwagons();
    fetchSources();
  }, [fetchSpeedwagons, fetchSources]);

  const toggleSpeedwagon = async (id: string) => {
    const updated = selectedSpeedwagonIds.includes(id)
      ? selectedSpeedwagonIds.filter((sid) => sid !== id)
      : [...selectedSpeedwagonIds, id];
    setSelectedSpeedwagonIds(updated);
    await saveChanges(updated, selectedSourceIds);
  };

  const toggleSource = async (id: string) => {
    const updated = selectedSourceIds.includes(id)
      ? selectedSourceIds.filter((sid) => sid !== id)
      : [...selectedSourceIds, id];
    setSelectedSourceIds(updated);
    await saveChanges(selectedSpeedwagonIds, updated);
  };

  const saveChanges = async (speedwagonIds: string[], sourceIds: string[]) => {
    setSaving(true);
    try {
      await updateSession(sessionId, { speedwagon_ids: speedwagonIds, source_ids: sourceIds });
      bumpSessionListVersion();
    } catch {
      toast.error("세션 업데이트에 실패했습니다");
    } finally {
      setSaving(false);
    }
  };

  const indexStatusBadge = (sw: ApiSpeedwagon) => {
    if (sw.index_status === "indexed") return null;
    const labels: Record<string, string> = {
      not_indexed: "인덱싱 필요",
      indexing: "인덱싱 중",
      error: "오류",
    };
    return (
      <span className="ml-1 text-xs text-muted-foreground">
        ({labels[sw.index_status] ?? sw.index_status})
      </span>
    );
  };

  return (
    <div className="w-72 flex flex-col" role="dialog" aria-label="Speedwagon 선택">
      <div className="flex items-center justify-between p-3 border-b">
        <span className="font-semibold text-sm">
          Speedwagon {saving && <span className="text-xs text-muted-foreground ml-1">저장 중...</span>}
        </span>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={onClose}
          aria-label="패널 닫기"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <div className="max-h-96 overflow-y-auto p-3 space-y-3">
        {/* Speedwagon 섹션 */}
        <div>
          <span className="text-xs font-medium text-muted-foreground">Speedwagons</span>
          <div className="mt-2 space-y-1.5">
            {speedwagons.length === 0 && (
              <span className="text-xs text-muted-foreground">등록된 Speedwagon이 없습니다</span>
            )}
            {speedwagons.map((sw) => {
              const isBuilt = sw.index_status === "indexed";
              const isChecked = selectedSpeedwagonIds.includes(sw.id);
              return (
                <label
                  key={sw.id}
                  className={`flex items-center gap-2 rounded-md p-2 transition-colors ${
                    isBuilt ? "cursor-pointer hover:bg-accent" : "opacity-50 cursor-not-allowed"
                  } ${isChecked && isBuilt ? "bg-primary/10" : ""}`}
                  title={!isBuilt ? "인덱싱 후 사용 가능합니다" : undefined}
                >
                  <Checkbox
                    checked={isChecked}
                    disabled={!isBuilt}
                    onCheckedChange={() => isBuilt && toggleSpeedwagon(sw.id)}
                  />
                  <span className="text-sm flex-1 truncate">{sw.name}</span>
                  {indexStatusBadge(sw)}
                </label>
              );
            })}
          </div>
        </div>

        <Separator />

        {/* ad-hoc Source 섹션 */}
        <div>
          <span className="text-xs font-medium text-muted-foreground">ad-hoc Sources</span>
          <div className="mt-2 space-y-1.5">
            {sources.length === 0 && (
              <span className="text-xs text-muted-foreground">등록된 Source가 없습니다</span>
            )}
            {sources.map((src) => {
              const isChecked = selectedSourceIds.includes(src.id);
              return (
                <label
                  key={src.id}
                  className={`flex items-center gap-2 rounded-md p-2 cursor-pointer transition-colors hover:bg-accent ${
                    isChecked ? "bg-primary/10" : ""
                  }`}
                >
                  <Checkbox
                    checked={isChecked}
                    onCheckedChange={() => toggleSource(src.id)}
                  />
                  <span className="text-sm flex-1 truncate">{src.name}</span>
                </label>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
