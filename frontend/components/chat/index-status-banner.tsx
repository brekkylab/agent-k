"use client";

import { useState, useEffect, useRef, useCallback } from "react";
import { X, Loader2, CheckCircle } from "lucide-react";
import { getSpeedwagons } from "@/lib/api";

interface IndexStatusBannerProps {
  sessionSpeedwagonIds: string[];
}

type BannerState =
  | { type: "indexing"; names: string[] }
  | { type: "success"; names: string[] }
  | { type: "hidden" };

const POLL_INTERVAL_MS = 10_000;
const SUCCESS_DISPLAY_MS = 3_000;

export function IndexStatusBanner({ sessionSpeedwagonIds }: IndexStatusBannerProps) {
  const [banner, setBanner] = useState<BannerState>({ type: "hidden" });
  const [dismissed, setDismissed] = useState(false);
  const prevIndexingRef = useRef<Set<string>>(new Set());
  const successTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const checkStatus = useCallback(async () => {
    if (sessionSpeedwagonIds.length === 0) return;
    try {
      const all = await getSpeedwagons();
      const relevant = all.filter((sw) => sessionSpeedwagonIds.includes(sw.id));

      const indexingNames = relevant
        .filter((sw) => sw.index_status === "indexing")
        .map((sw) => sw.name);

      const prevIndexing = prevIndexingRef.current;
      const justFinished = [...prevIndexing].filter((id) => {
        const sw = relevant.find((s) => s.id === id);
        return sw && sw.index_status === "indexed";
      });

      // Track currently indexing ids
      const nowIndexingIds = relevant
        .filter((sw) => sw.index_status === "indexing")
        .map((sw) => sw.id);
      prevIndexingRef.current = new Set(nowIndexingIds);

      if (justFinished.length > 0) {
        // indexing → indexed transition detected: show success message
        const finishedNames = justFinished
          .map((id) => relevant.find((sw) => sw.id === id)?.name)
          .filter(Boolean) as string[];

        setBanner({ type: "success", names: finishedNames });
        setDismissed(false);

        if (successTimerRef.current) clearTimeout(successTimerRef.current);
        successTimerRef.current = setTimeout(() => {
          setBanner({ type: "hidden" });
        }, SUCCESS_DISPLAY_MS);
      } else if (indexingNames.length > 0 && !dismissed) {
        setBanner({ type: "indexing", names: indexingNames });
      } else if (indexingNames.length === 0 && banner.type === "indexing") {
        setBanner({ type: "hidden" });
      }
    } catch {
      // 폴링 실패 시 무시
    }
  }, [sessionSpeedwagonIds, dismissed, banner.type]);

  // Poll while any session speedwagon is indexing
  useEffect(() => {
    if (sessionSpeedwagonIds.length === 0) {
      setBanner({ type: "hidden" });
      return;
    }

    checkStatus();
    pollRef.current = setInterval(checkStatus, POLL_INTERVAL_MS);

    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
      if (successTimerRef.current) clearTimeout(successTimerRef.current);
    };
  }, [sessionSpeedwagonIds, checkStatus]);

  const handleDismiss = () => {
    setDismissed(true);
    setBanner({ type: "hidden" });
  };

  if (banner.type === "hidden") return null;

  if (banner.type === "success") {
    return (
      <div className="flex items-center gap-2 px-4 py-2 bg-green-500/10 border-b border-green-500/20 text-sm text-green-700 dark:text-green-400">
        <CheckCircle className="h-4 w-4 shrink-0" />
        <span className="flex-1">
          <strong>{banner.names.join(", ")}</strong> 인덱싱 완료
        </span>
      </div>
    );
  }

  // indexing
  return (
    <div className="flex items-center gap-2 px-4 py-2 bg-yellow-500/10 border-b border-yellow-500/20 text-sm text-yellow-700 dark:text-yellow-400">
      <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
      <span className="flex-1">
        <strong>{banner.names.join(", ")}</strong> 인덱싱 중...
      </span>
      <button
        onClick={handleDismiss}
        className="shrink-0 p-0.5 rounded hover:bg-yellow-500/20 transition-colors"
        aria-label="배너 닫기"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
