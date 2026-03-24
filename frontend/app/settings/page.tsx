"use client";

import { useState } from "react";
import { ProviderForm } from "@/components/settings/provider-form";
import { ProviderList } from "@/components/settings/provider-list";
import { Separator } from "@/components/ui/separator";

export default function SettingsPage() {
  const [refreshKey, setRefreshKey] = useState(0);

  return (
    <div className="mx-auto max-w-2xl p-6 space-y-8">
      <div>
        <h1 className="text-2xl font-semibold">Settings</h1>
        <p className="text-sm text-muted-foreground mt-1">
          LLM Provider를 등록하여 AI 에이전트를 사용할 수 있습니다.
        </p>
      </div>

      <div className="space-y-4">
        <h2 className="text-lg font-medium">Provider Profile 추가</h2>
        <ProviderForm onCreated={() => setRefreshKey((k) => k + 1)} />
      </div>

      <Separator />

      <div className="space-y-4">
        <h2 className="text-lg font-medium">등록된 Providers</h2>
        <ProviderList key={refreshKey} />
      </div>
    </div>
  );
}
