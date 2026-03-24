"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { MessageSquare, FileText, BookOpen, Settings } from "lucide-react";
import { cn } from "@/lib/utils";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { ThemeToggle } from "./theme-toggle";
import { SessionDropdown } from "@/components/chat/session-dropdown";

const NAV_ITEMS = [
  { href: "/chat", label: "Chat", icon: MessageSquare, hasDropdown: true },
  { href: "/documents", label: "Documents", icon: FileText, hasDropdown: false },
  { href: "/knowledge", label: "Knowledge", icon: BookOpen, hasDropdown: false },
] as const;

export function Sidebar() {
  const pathname = usePathname();

  return (
    <aside className="flex h-full w-64 flex-col border-r bg-background">
      <div className="flex h-14 items-center border-b px-4">
        <span className="text-lg font-semibold">agentwebui-test</span>
      </div>

      <nav className="flex-1 space-y-1 p-2">
        {NAV_ITEMS.map((item) => {
          const isActive = pathname.startsWith(item.href);
          const linkClasses = cn(
            "flex flex-1 items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
            isActive
              ? "bg-accent text-accent-foreground"
              : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          );

          return (
            <div key={item.href} className="flex items-center">
              {item.href === "/chat" ? (
                <Link
                  href="/chat"
                  className={linkClasses}
                >
                  <item.icon className="h-4 w-4" />
                  {item.label}
                </Link>
              ) : (
                <Link href={item.href} className={linkClasses}>
                  <item.icon className="h-4 w-4" />
                  {item.label}
                </Link>
              )}
              {item.hasDropdown && <SessionDropdown />}
            </div>
          );
        })}
      </nav>

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
