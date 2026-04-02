"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { Loader2, Trash2, Zap } from "lucide-react";
import {
  getSpeedwagon,
  updateSpeedwagon,
  deleteSpeedwagon,
  indexSpeedwagon,
} from "@/lib/api";
import type { ApiSpeedwagon } from "@/lib/types";
import { PROVIDER_MODELS } from "@/lib/constants";
import { useAppStore } from "@/lib/store";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

const ALL_MODELS: string[] = [
  ...PROVIDER_MODELS.OpenAI,
  ...PROVIDER_MODELS.Anthropic,
  ...PROVIDER_MODELS.Gemini,
];

interface Props {
  id: string;
}

function IndexStatusBadge({ sw }: { sw: ApiSpeedwagon }) {
  if (sw.index_status === "indexed") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-green-100 px-2.5 py-0.5 text-xs font-medium text-green-800 dark:bg-green-900/30 dark:text-green-400">
        <span className="h-1.5 w-1.5 rounded-full bg-green-500" />
        인덱싱 완료
        {sw.indexed_at && (
          <span className="text-green-600 dark:text-green-500">
            · {new Date(sw.indexed_at).toLocaleString()}
          </span>
        )}
      </span>
    );
  }
  if (sw.index_status === "indexing") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-blue-100 px-2.5 py-0.5 text-xs font-medium text-blue-800 dark:bg-blue-900/30 dark:text-blue-400">
        <Loader2 className="h-3 w-3 animate-spin" />
        인덱싱 중...
        {sw.index_started_at && (
          <span className="text-blue-600 dark:text-blue-500">
            · {new Date(sw.index_started_at).toLocaleString()}
          </span>
        )}
      </span>
    );
  }
  if (sw.index_status === "error") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-red-100 px-2.5 py-0.5 text-xs font-medium text-red-800 dark:bg-red-900/30 dark:text-red-400">
        <span className="h-1.5 w-1.5 rounded-full bg-red-500" />
        Error
        {sw.index_error && (
          <span className="ml-1 text-red-600 dark:text-red-500">
            · {sw.index_error}
          </span>
        )}
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full bg-muted px-2.5 py-0.5 text-xs font-medium text-muted-foreground">
      <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/50" />
      인덱싱 필요
    </span>
  );
}

export function SpeedwagonDetail({ id }: Props) {
  const router = useRouter();
  const [sw, setSw] = useState<ApiSpeedwagon | null>(null);
  const sources = useAppStore((s) => s.sources);
  const fetchSources = useAppStore((s) => s.fetchSources);
  const fetchSpeedwagons = useAppStore((s) => s.fetchSpeedwagons);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [indexing, setIndexing] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Local editable state
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [instruction, setInstruction] = useState("");
  const [lm, setLm] = useState<string>("");
  const [selectedSourceIds, setSelectedSourceIds] = useState<string[]>([]);

  // Track initial source_ids to detect re-indexing needed
  const lastBuiltSourceIds = useRef<string[]>([]);

  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const load = useCallback(async () => {
    try {
      const [swData] = await Promise.all([
        getSpeedwagon(id),
        fetchSources(),
      ]);
      setSw(swData);
      setName(swData.name);
      setDescription(swData.description);
      setInstruction(swData.instruction ?? "");
      setLm(swData.lm ?? "");
      setSelectedSourceIds(swData.source_ids);
      if (swData.index_status === "indexed") {
        lastBuiltSourceIds.current = swData.source_ids;
      }
    } catch {
      setError("Failed to load speedwagon");
    } finally {
      setLoading(false);
    }
  }, [id, fetchSources]);

  useEffect(() => {
    load();
  }, [load]);

  // Poll while indexing
  useEffect(() => {
    if (sw?.index_status === "indexing") {
      pollRef.current = setInterval(async () => {
        try {
          const updated = await getSpeedwagon(id);
          setSw(updated);
          if (updated.index_status !== "indexing") {
            if (pollRef.current) clearInterval(pollRef.current);
            if (updated.index_status === "indexed") {
              lastBuiltSourceIds.current = updated.source_ids;
            }
            fetchSpeedwagons();
          }
        } catch {
          // ignore poll errors
        }
      }, 3000);
    }
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [id, sw?.index_status, fetchSpeedwagons]);

  // Debounced save
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const scheduleAutoSave = useCallback(
    (patch: {
      name: string;
      description: string;
      instruction: string;
      lm: string;
      sourceIds: string[];
    }) => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(async () => {
        setSaving(true);
        try {
          const updated = await updateSpeedwagon(id, {
            name: patch.name,
            description: patch.description,
            instruction: patch.instruction || null,
            lm: patch.lm || null,
            source_ids: patch.sourceIds,
          });
          setSw(updated);
          fetchSpeedwagons();
        } catch {
          // ignore save errors silently
        } finally {
          setSaving(false);
        }
      }, 800);
    },
    [id, fetchSpeedwagons],
  );

  const handleNameChange = (v: string) => {
    setName(v);
    scheduleAutoSave({ name: v, description, instruction, lm, sourceIds: selectedSourceIds });
  };

  const handleDescriptionChange = (v: string) => {
    setDescription(v);
    scheduleAutoSave({ name, description: v, instruction, lm, sourceIds: selectedSourceIds });
  };

  const handleInstructionChange = (v: string) => {
    setInstruction(v);
    scheduleAutoSave({ name, description, instruction: v, lm, sourceIds: selectedSourceIds });
  };

  const handleLmChange = (v: string) => {
    setLm(v);
    scheduleAutoSave({ name, description, instruction, lm: v, sourceIds: selectedSourceIds });
  };

  const handleSourceToggle = (sourceId: string, checked: boolean) => {
    const next = checked
      ? [...selectedSourceIds, sourceId]
      : selectedSourceIds.filter((s) => s !== sourceId);
    setSelectedSourceIds(next);
    scheduleAutoSave({ name, description, instruction, lm, sourceIds: next });
  };

  const handleIndex = async () => {
    setIndexing(true);
    try {
      await indexSpeedwagon(id);
      // Optimistically set indexing status
      setSw((prev) => prev ? { ...prev, index_status: "indexing", index_started_at: new Date().toISOString() } : prev);
      fetchSpeedwagons();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "인덱싱 실패");
    } finally {
      setIndexing(false);
    }
  };

  const handleDelete = async () => {
    setDeleting(true);
    try {
      await deleteSpeedwagon(id);
      fetchSpeedwagons();
      router.push("/sources");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "삭제에 실패했습니다");
      setDeleting(false);
      setDeleteOpen(false);
    }
  };

  const reindexNeeded =
    sw?.index_status === "indexed" &&
    selectedSourceIds.slice().sort().join(",") !==
      lastBuiltSourceIds.current.slice().sort().join(",");

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (error && !sw) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-sm text-destructive">{error}</p>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-2xl space-y-8 p-6">
      {/* Header */}
      <div className="flex items-start justify-between gap-4">
        <div className="flex items-center gap-2">
          <Zap className="h-5 w-5 text-muted-foreground" />
          <h1 className="text-xl font-semibold">Speedwagon</h1>
          {saving && <span className="text-xs text-muted-foreground">저장 중...</span>}
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="text-destructive hover:text-destructive"
          onClick={() => setDeleteOpen(true)}
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>

      {error && (
        <p className="rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive">{error}</p>
      )}

      {/* Name / Description */}
      <div className="space-y-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">이름</label>
          <Input value={name} onChange={(e) => handleNameChange(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">설명</label>
          <Input
            value={description}
            onChange={(e) => handleDescriptionChange(e.target.value)}
            placeholder="이 Speedwagon이 답할 수 있는 내용..."
          />
        </div>
      </div>

      {/* Instruction */}
      <div className="space-y-1.5">
        <label className="text-sm font-medium">서브에이전트 시스템 프롬프트</label>
        <p className="text-xs text-muted-foreground">
          비워두면 기본 시스템 프롬프트를 사용합니다.
        </p>
        <textarea
          className="w-full rounded-md border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-ring min-h-[100px] resize-y"
          value={instruction}
          onChange={(e) => handleInstructionChange(e.target.value)}
          placeholder="You are a helpful assistant specialized in..."
        />
      </div>

      {/* LM Model */}
      <div className="space-y-1.5">
        <label className="text-sm font-medium">서브에이전트 모델</label>
        <p className="text-xs text-muted-foreground">
          비워두면 메인 에이전트와 같은 모델을 사용합니다.
        </p>
        <select
          className="w-full rounded-md border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
          value={lm}
          onChange={(e) => handleLmChange(e.target.value)}
        >
          <option value="">메인 에이전트와 동일</option>
          {ALL_MODELS.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </div>

      {/* Sources */}
      <div className="space-y-2">
        <label className="text-sm font-medium">연결된 Sources</label>
        {sources.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            업로드된 Source가 없습니다.{" "}
            <a href="/sources" className="underline">
              Sources 페이지
            </a>
            에서 파일을 업로드하세요.
          </p>
        ) : (
          <div className="space-y-2">
            {sources.map((src) => (
              <label
                key={src.id}
                className="flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-1.5 hover:bg-muted"
              >
                <Checkbox
                  checked={selectedSourceIds.includes(src.id)}
                  onCheckedChange={(checked) =>
                    handleSourceToggle(src.id, checked === true)
                  }
                />
                <span className="text-sm">{src.name}</span>
                <span className="ml-auto text-xs text-muted-foreground">
                  {(src.size / 1024).toFixed(1)} KB
                </span>
              </label>
            ))}
          </div>
        )}
      </div>

      {/* Indexing section */}
      <div className="rounded-md border p-4 space-y-3">
        <div className="flex items-center justify-between gap-4">
          <div className="space-y-1">
            <p className="text-sm font-medium">인덱싱</p>
            {sw && <IndexStatusBadge sw={sw} />}
          </div>
          <Button
            onClick={handleIndex}
            disabled={
              indexing ||
              sw?.index_status === "indexing" ||
              selectedSourceIds.length === 0
            }
            size="sm"
          >
            {indexing || sw?.index_status === "indexing" ? (
              <>
                <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                인덱싱 중...
              </>
            ) : (
              <>
                <Zap className="mr-1.5 h-3.5 w-3.5" />
                인덱싱
              </>
            )}
          </Button>
        </div>

        {reindexNeeded && (
          <p className="text-xs text-amber-600 dark:text-amber-400">
            Source가 변경되었습니다. 재인덱싱이 필요합니다.
          </p>
        )}

        {selectedSourceIds.length === 0 && (
          <p className="text-xs text-muted-foreground">
            인덱싱하려면 Source를 하나 이상 연결하세요.
          </p>
        )}
      </div>

      {/* Delete dialog */}
      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Speedwagon 삭제</DialogTitle>
            <DialogDescription>
              <strong>{name}</strong>을(를) 삭제하시겠습니까? 빌드된 인덱스도 함께 삭제됩니다. 이
              작업은 되돌릴 수 없습니다.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)} disabled={deleting}>
              취소
            </Button>
            <Button variant="destructive" onClick={handleDelete} disabled={deleting}>
              {deleting ? <Loader2 className="mr-1.5 h-4 w-4 animate-spin" /> : null}
              삭제
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
