"use client";

import { useEffect, useState, useCallback } from "react";
import { Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { getProviderProfiles, deleteProviderProfile, ApiError } from "@/lib/api";
import type { ApiProviderProfile } from "@/lib/types";

export function ProviderList() {
  const [profiles, setProfiles] = useState<ApiProviderProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchProfiles = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await getProviderProfiles();
      setProfiles(data);
    } catch (err) {
      if (err instanceof ApiError) {
        setError(err.message);
      } else {
        setError("Provider 목록을 불러올 수 없습니다.");
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchProfiles();
  }, [fetchProfiles]);

  const handleDelete = async (id: string) => {
    try {
      await deleteProviderProfile(id);
      setProfiles((prev) => prev.filter((p) => p.id !== id));
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        alert("이 Provider를 사용하는 세션이 있어 삭제할 수 없습니다.\n해당 세션을 먼저 삭제해 주세요.");
      } else if (err instanceof ApiError) {
        setError(err.message);
      }
    }
  };

  if (loading) {
    return <p className="text-sm text-muted-foreground">불러오는 중...</p>;
  }

  if (error) {
    return (
      <div className="space-y-2">
        <p className="text-sm text-destructive">{error}</p>
        <Button variant="outline" size="sm" onClick={fetchProfiles}>
          다시 시도
        </Button>
      </div>
    );
  }

  if (profiles.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        등록된 Provider가 없습니다. 위에서 추가해주세요.
      </p>
    );
  }

  return (
    <ul className="space-y-2">
      {profiles.map((profile) => {
        const schema =
          typeof profile.provider?.lm === "object" &&
          profile.provider.lm.schema
            ? profile.provider.lm.schema
            : "Unknown";

        return (
          <li
            key={profile.id}
            className="flex items-center justify-between rounded-md border px-4 py-3"
          >
            <div className="space-y-0.5">
              <p className="text-sm font-medium">{profile.name}</p>
              <p className="text-xs text-muted-foreground">
                {schema}
                {profile.is_default && " · 기본"}
              </p>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => handleDelete(profile.id)}
            >
              <Trash2 className="h-4 w-4 text-muted-foreground" />
            </Button>
          </li>
        );
      })}
    </ul>
  );
}

// Re-fetch helper exposed for parent to call after creation
ProviderList.displayName = "ProviderList";
