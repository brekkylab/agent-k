"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { Database, Settings, Plus, Zap, Loader2, ChevronDown, ChevronRight } from "lucide-react";
import { useEffect, useState } from "react";
import { cn } from "@/lib/utils";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { ThemeToggle } from "./theme-toggle";
import { SessionList } from "@/components/chat/session-list";
import { getSpeedwagons, createSpeedwagon } from "@/lib/api";
import type { ApiSpeedwagon } from "@/lib/types";
import { useAppStore } from "@/lib/store";

const NAV_ITEMS = [
  { href: "/sources", label: "Sources", icon: Database },
] as const;

function SpeedwagonStatusDot({ status }: { status: ApiSpeedwagon["index_status"] }) {
  if (status === "indexed")
    return <span className="h-2 w-2 rounded-full bg-green-500 shrink-0" />;
  if (status === "indexing")
    return <Loader2 className="h-3 w-3 animate-spin text-blue-500 shrink-0" />;
  if (status === "error")
    return <span className="h-2 w-2 rounded-full bg-red-500 shrink-0" />;
  return <span className="h-2 w-2 rounded-full bg-muted-foreground/30 shrink-0" />;
}

export function Sidebar() {
  const pathname = usePathname();
  const router = useRouter();
  const [speedwagons, setSpeedwagons] = useState<ApiSpeedwagon[]>([]);
  const [creating, setCreating] = useState(false);
  const [speedwagonsOpen, setSpeedwagonsOpen] = useState(true);
  const speedwagonListVersion = useAppStore((s) => s.speedwagonListVersion);

  useEffect(() => {
    getSpeedwagons()
      .then(setSpeedwagons)
      .catch(() => {});
  }, [speedwagonListVersion]);

  const handleCreateSpeedwagon = async () => {
    setCreating(true);
    try {
      const sw = await createSpeedwagon({ name: "새 Speedwagon", description: "" });
      setSpeedwagons((prev) => [...prev, sw]);
      useAppStore.getState().bumpSpeedwagonListVersion();
      router.push(`/speedwagons/${sw.id}`);
    } catch {
      // ignore
    } finally {
      setCreating(false);
    }
  };

  return (
    <aside className="flex h-full w-64 flex-col border-r bg-background">
      <div className="flex h-14 items-center border-b px-4">
        <span className="text-lg font-semibold">agentwebui-test</span>
      </div>

      <nav className="space-y-1 p-2">
        {NAV_ITEMS.map((item) => {
          const isActive = pathname.startsWith(item.href);
          return (
            <Link
              key={item.href}
              href={item.href}
              className={cn(
                "flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                isActive
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              )}
            >
              <item.icon className="h-4 w-4" />
              {item.label}
            </Link>
          );
        })}
      </nav>

      <div className="px-3">
        <Separator />
      </div>

      {/* Speedwagons section */}
      <div className="p-2 space-y-1">
        <div className="flex items-center justify-between px-3 py-1.5">
          <button
            type="button"
            className="flex flex-1 items-center gap-2 hover:text-foreground transition-colors"
            onClick={() => setSpeedwagonsOpen((v) => !v)}
          >
            {speedwagonsOpen ? (
              <ChevronDown className="h-3 w-3 text-muted-foreground" />
            ) : (
              <ChevronRight className="h-3 w-3 text-muted-foreground" />
            )}
            <Zap className="h-4 w-4 text-muted-foreground" />
            <span className="text-sm font-medium text-muted-foreground">Speedwagons</span>
          </button>
          <Button
            variant="ghost"
            size="icon"
            className="h-5 w-5"
            onClick={handleCreateSpeedwagon}
            disabled={creating}
            title="새 Speedwagon 만들기"
          >
            {creating ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <Plus className="h-3 w-3" />
            )}
          </Button>
        </div>

        {speedwagonsOpen && (
          speedwagons.length === 0 ? (
            <p className="px-3 py-1 text-xs text-muted-foreground">
              Speedwagon이 없습니다.
            </p>
          ) : (
            speedwagons.map((sw) => {
              const isActive = pathname === `/speedwagons/${sw.id}`;
              return (
                <Link
                  key={sw.id}
                  href={`/speedwagons/${sw.id}`}
                  className={cn(
                    "flex items-center gap-2 rounded-md px-3 py-1.5 text-sm transition-colors",
                    isActive
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                  )}
                >
                  <SpeedwagonStatusDot status={sw.index_status} />
                  <span className="flex-1 truncate">{sw.name}</span>
                </Link>
              );
            })
          )
        )}
      </div>

      <div className="px-3">
        <Separator />
      </div>

      <div className="flex-1 overflow-hidden p-2">
        <SessionList />
      </div>

      <div className="border-t p-3 space-y-3">
        <ThemeToggle />
        <div className="flex items-center gap-2 px-1">
          <Avatar className="h-7 w-7">
            <AvatarFallback className="text-xs">U</AvatarFallback>
          </Avatar>
          <span className="flex-1 text-sm text-muted-foreground">Test User</span>
          <Link href="/settings">
            <Button variant="ghost" size="icon" className="h-7 w-7">
              <Settings className="h-4 w-4" />
            </Button>
          </Link>
        </div>
      </div>
    </aside>
  );
}
