import type {
  ApiProviderProfile,
  ApiAgent,
  ApiSession,
  ApiSessionMessage,
} from "./types";

const API_BASE_URL =
  process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

// --- Error Types ---

export type ApiErrorType = "network" | "validation" | "server" | "timeout";

export class ApiError extends Error {
  type: ApiErrorType;
  status?: number;

  constructor(type: ApiErrorType, message: string, status?: number) {
    super(message);
    this.type = type;
    this.status = status;
  }
}

// --- Fetch Wrapper ---

async function fetchApi<T>(
  path: string,
  options?: RequestInit & { timeout?: number },
): Promise<T> {
  const { timeout = 10_000, ...fetchOptions } = options ?? {};

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);

  try {
    const response = await fetch(`${API_BASE_URL}${path}`, {
      ...fetchOptions,
      signal: controller.signal,
      headers: {
        "Content-Type": "application/json",
        ...fetchOptions?.headers,
      },
    });

    if (!response.ok) {
      const body = await response.json().catch(() => null);
      const message =
        body?.error ?? `HTTP ${response.status}: ${response.statusText}`;

      if (response.status >= 400 && response.status < 500) {
        throw new ApiError("validation", message, response.status);
      }
      throw new ApiError("server", message, response.status);
    }

    if (response.status === 204) {
      return undefined as T;
    }

    return (await response.json()) as T;
  } catch (error) {
    if (error instanceof ApiError) throw error;

    if (error instanceof DOMException && error.name === "AbortError") {
      throw new ApiError("timeout", `Request timed out after ${timeout}ms`);
    }

    throw new ApiError(
      "network",
      error instanceof Error ? error.message : "Network error",
    );
  } finally {
    clearTimeout(timer);
  }
}

// --- Provider Profiles ---

export async function getProviderProfiles(): Promise<ApiProviderProfile[]> {
  return fetchApi<ApiProviderProfile[]>("/provider-profiles");
}

export async function createProviderProfile(data: {
  name: string;
  provider: ApiProviderProfile["provider"];
  is_default?: boolean;
}): Promise<ApiProviderProfile> {
  return fetchApi<ApiProviderProfile>("/provider-profiles", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function deleteProviderProfile(id: string): Promise<void> {
  return fetchApi<void>(`/provider-profiles/${id}`, { method: "DELETE" });
}

// --- Agents ---

export async function createAgent(data: {
  spec: { lm: string; instruction?: string; tools?: unknown[] };
}): Promise<ApiAgent> {
  return fetchApi<ApiAgent>("/agents", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

// --- Sessions ---

export async function createSession(data: {
  agent_id: string;
  provider_profile_id?: string;
  title?: string;
}): Promise<ApiSession> {
  return fetchApi<ApiSession>("/sessions", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function getSessions(
  includeMessages = false,
): Promise<ApiSession[]> {
  const params = new URLSearchParams();
  if (includeMessages) params.set("include_messages", "true");
  const query = params.toString();
  return fetchApi<ApiSession[]>(`/sessions${query ? `?${query}` : ""}`);
}

export async function getSession(id: string): Promise<ApiSession> {
  return fetchApi<ApiSession>(`/sessions/${id}?include_messages=true`);
}

export async function deleteSession(id: string): Promise<void> {
  return fetchApi<void>(`/sessions/${id}`, { method: "DELETE" });
}

// --- Session Title ---

export async function updateSessionTitle(
  id: string,
  title: string,
): Promise<ApiSession> {
  return fetchApi<ApiSession>(`/sessions/${id}`, {
    method: "PUT",
    body: JSON.stringify({ title }),
  });
}

// --- Messages ---

export async function sendMessage(
  sessionId: string,
  content: string,
): Promise<{ assistant_message: ApiSessionMessage | null }> {
  return fetchApi<{ assistant_message: ApiSessionMessage | null }>(
    `/sessions/${sessionId}/messages`,
    {
      method: "POST",
      body: JSON.stringify({ role: "user", content }),
      timeout: 60_000,
    },
  );
}
