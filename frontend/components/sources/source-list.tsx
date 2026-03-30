"use client";

import { useState, useEffect, useCallback } from "react";
import { FileText, Trash2, ChevronDown, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { getSources, getSpeedwagons, deleteSource } from "@/lib/api";
import type { ApiSource, ApiSpeedwagon } from "@/lib/types";

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1048576) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / 1048576).toFixed(1) + " MB";
}

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString("ko-KR", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });
}

export function SourceList({ refreshKey }: { refreshKey?: number }) {
  const [sources, setSources] = useState<ApiSource[]>([]);
  const [speedwagons, setSpeedwagons] = useState<ApiSpeedwagon[]>([]);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [deleteTargetId, setDeleteTargetId] = useState<string | null>(null);

  const fetchData = useCallback(async () => {
    try {
      const [srcData, swData] = await Promise.all([
        getSources(),
        getSpeedwagons(),
      ]);
      setSources(srcData);
      setSpeedwagons(swData);
    } catch (error) {
      console.error("Failed to load sources:", error);
    }
  }, []);

  useEffect(() => {
    fetchData();
  }, [fetchData, refreshKey]);

  const getSpeedwagonsForSource = (sourceId: string) =>
    speedwagons.filter((sw) => sw.source_ids.includes(sourceId));

  const handleDelete = async (id: string) => {
    try {
      await deleteSource(id);
      setDeleteTargetId(null);
      setExpandedId(null);
      fetchData();
    } catch (error) {
      console.error("Failed to delete source:", error);
    }
  };

  const deleteTarget = sources.find((s) => s.id === deleteTargetId);
  const deleteSpeedwagonCount = deleteTargetId
    ? getSpeedwagonsForSource(deleteTargetId).length
    : 0;

  return (
    <>
      <div className="space-y-1">
        {sources.map((src) => {
          const isExpanded = expandedId === src.id;
          const srcSpeedwagons = getSpeedwagonsForSource(src.id);

          return (
            <div key={src.id} className="rounded-lg border bg-card">
              <button
                onClick={() => setExpandedId(isExpanded ? null : src.id)}
                className="flex w-full items-center gap-3 p-3 text-left hover:bg-accent/50 transition-colors rounded-lg"
              >
                {isExpanded ? (
                  <ChevronDown className="h-4 w-4 shrink-0" />
                ) : (
                  <ChevronRight className="h-4 w-4 shrink-0" />
                )}
                <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
                <div className="flex-1 min-w-0">
                  <div className="font-medium text-sm truncate">{src.name}</div>
                  <div className="text-xs text-muted-foreground">
                    {formatFileSize(src.size)} · {formatDate(src.created_at)} 추가
                  </div>
                </div>
              </button>

              {isExpanded && (
                <div className="border-t px-4 py-3 space-y-3">
                  <div>
                    <span className="text-xs font-medium text-muted-foreground">
                      소속 Speedwagon:
                    </span>
                    <div className="mt-1 flex flex-wrap gap-1">
                      {srcSpeedwagons.length > 0 ? (
                        srcSpeedwagons.map((sw) => (
                          <Badge key={sw.id} variant="secondary">
                            {sw.name}
                          </Badge>
                        ))
                      ) : (
                        <span className="text-xs text-muted-foreground">없음</span>
                      )}
                    </div>
                  </div>
                  <div className="flex justify-end">
                    <Button
                      variant="destructive"
                      size="sm"
                      onClick={() => setDeleteTargetId(src.id)}
                    >
                      <Trash2 className="h-3 w-3 mr-1" /> 삭제
                    </Button>
                  </div>
                </div>
              )}
            </div>
          );
        })}

        {sources.length === 0 && (
          <div className="text-center py-12 text-muted-foreground">
            등록된 소스가 없습니다
          </div>
        )}
      </div>

      <Dialog
        open={deleteTargetId !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTargetId(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Source 삭제</DialogTitle>
            <DialogDescription>
              &ldquo;{deleteTarget?.name}&rdquo;을(를) 삭제하시겠습니까?
              {deleteSpeedwagonCount > 0 && (
                <>
                  {" "}이 Source는 {deleteSpeedwagonCount}개 Speedwagon에서 사용 중입니다.
                </>
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTargetId(null)}>
              취소
            </Button>
            <Button
              variant="destructive"
              onClick={() => deleteTargetId && handleDelete(deleteTargetId)}
            >
              삭제
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
