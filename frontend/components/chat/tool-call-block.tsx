"use client";

import { useState } from "react";
import { ChevronRight, ChevronDown, Zap, CheckCircle, Loader2 } from "lucide-react";

interface ToolCallBlockProps {
  tool: string;
  status: "calling" | "done" | "error";
  args?: Record<string, unknown>;
  result?: unknown;
  error?: string;
  level?: "info" | "debug";
}

export function ToolCallBlock({ tool, status, args, result, error }: ToolCallBlockProps) {
  const [expanded, setExpanded] = useState(false);

  const hasDetails = args !== undefined || (result !== undefined && result !== null) || error !== undefined;

  return (
    <div className="my-2 rounded-lg border bg-muted/30 text-sm">
      <button
        type="button"
        onClick={() => hasDetails && setExpanded(!expanded)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-muted/50 transition-colors rounded-lg"
        disabled={!hasDetails}
      >
        {/* Status icon */}
        {status === "calling" && <Loader2 className="h-3.5 w-3.5 animate-spin text-blue-500 shrink-0" />}
        {status === "done" && <CheckCircle className="h-3.5 w-3.5 text-green-500 shrink-0" />}
        {status === "error" && <Zap className="h-3.5 w-3.5 text-red-500 shrink-0" />}

        {/* Tool name + status text */}
        <span className="flex-1 font-medium">
          {status === "calling" && `${tool}에 질의 중...`}
          {status === "done" && `${tool} 완료`}
          {status === "error" && `${tool} 오류`}
        </span>

        {/* Expand chevron (only if has details) */}
        {hasDetails && (
          expanded
            ? <ChevronDown className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            : <ChevronRight className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
        )}
      </button>

      {/* Expanded details (debug level) */}
      {expanded && hasDetails && (
        <div className="border-t px-3 py-2 space-y-1.5 text-xs text-muted-foreground font-mono">
          {args && (
            <div>
              <span className="font-semibold text-foreground/70">Args:</span>
              <pre className="mt-0.5 overflow-x-auto whitespace-pre-wrap">{JSON.stringify(args, null, 2)}</pre>
            </div>
          )}
          {result !== undefined && result !== null && (
            <div>
              <span className="font-semibold text-foreground/70">Result:</span>
              <pre className="mt-0.5 overflow-x-auto whitespace-pre-wrap">
                {typeof result === "string" ? result : JSON.stringify(result, null, 2)}
              </pre>
            </div>
          )}
          {error && (
            <div className="text-destructive">
              <span className="font-semibold">Error:</span> {error}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
