import type {
  ApiProviderProfile,
  ApiAgent,
  ApiSession,
  ApiSessionMessage,
  ApiSessionToolCall,
  ApiSource,
  ApiSpeedwagon,
  SseEventType,
  SseEvent,
  CreateProviderProfileRequest,
  UpdateProviderProfileRequest,
  CreateAgentRequest,
  UpdateAgentRequest,
  CreateSessionRequest,
  UpdateSessionRequest,
  CreateSpeedwagonRequest,
  UpdateSpeedwagonRequest,
} from "./types";

export type { SseEventType, SseEvent, ApiSessionToolCall };

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
        ...(fetchOptions?.body instanceof FormData
          ? {}
          : { "Content-Type": "application/json" }),
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

    if (response.status === 204 || response.status === 202) {
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

export async function createProviderProfile(
  data: CreateProviderProfileRequest,
): Promise<ApiProviderProfile> {
  return fetchApi<ApiProviderProfile>("/provider-profiles", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function updateProviderProfile(
  id: string,
  data: UpdateProviderProfileRequest,
): Promise<ApiProviderProfile> {
  return fetchApi<ApiProviderProfile>(`/provider-profiles/${id}`, {
    method: "PUT",
    body: JSON.stringify(data),
  });
}

export async function deleteProviderProfile(id: string): Promise<void> {
  return fetchApi<void>(`/provider-profiles/${id}`, { method: "DELETE" });
}

// --- Agents ---

export async function createAgent(data: CreateAgentRequest): Promise<ApiAgent> {
  return fetchApi<ApiAgent>("/agents", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function getAgent(id: string): Promise<ApiAgent> {
  return fetchApi<ApiAgent>(`/agents/${id}`);
}

export async function updateAgent(
  id: string,
  data: UpdateAgentRequest,
): Promise<ApiAgent> {
  return fetchApi<ApiAgent>(`/agents/${id}`, {
    method: "PUT",
    body: JSON.stringify(data),
  });
}

// --- Sessions ---

export async function createSession(
  data: CreateSessionRequest,
): Promise<ApiSession> {
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

export async function updateSessionTitle(
  id: string,
  title: string,
): Promise<ApiSession> {
  return fetchApi<ApiSession>(`/sessions/${id}`, {
    method: "PUT",
    body: JSON.stringify({ title }),
  });
}

export async function updateSession(
  id: string,
  data: UpdateSessionRequest,
): Promise<ApiSession> {
  return fetchApi<ApiSession>(`/sessions/${id}`, {
    method: "PUT",
    body: JSON.stringify(data),
  });
}

// --- Messages ---

export async function* sendMessageStream(
  sessionId: string,
  content: string,
): AsyncGenerator<{ event: SseEventType; data: SseEvent }> {
  const response = await fetch(`${API_BASE_URL}/sessions/${sessionId}/messages/stream`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ content }),
    signal: AbortSignal.timeout(120_000),
  });

  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new ApiError(
      response.status >= 500 ? "server" : "validation",
      body?.error ?? `HTTP ${response.status}`,
      response.status,
    );
  }

  const stream = response.body;
  if (!stream) return;

  const reader = stream.pipeThrough(new TextDecoderStream()).getReader();
  let buffer = "";
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += value;
    const parts = buffer.split("\n\n");
    buffer = parts.pop() ?? "";
    for (const part of parts) {
      const eventMatch = part.match(/^event: (.+)$/m);
      const dataMatch = part.match(/^data: (.+)$/m);
      if (dataMatch) {
        const event = (eventMatch?.[1] ?? "message") as SseEventType;
        try {
          const parsed = JSON.parse(dataMatch[1]) as SseEvent;
          yield { event, data: parsed };
        } catch {
          // Skip unparseable events
        }
      }
    }
  }
}

// --- Session Tool Calls ---

export async function getSessionToolCalls(
  sessionId: string,
): Promise<ApiSessionToolCall[]> {
  return fetchApi<ApiSessionToolCall[]>(`/sessions/${sessionId}/tool-calls`);
}

// --- Sources ---

export async function getSources(): Promise<ApiSource[]> {
  return fetchApi<ApiSource[]>("/sources");
}

export async function uploadSource(file: File): Promise<ApiSource> {
  const formData = new FormData();
  formData.append("file", file);
  return fetchApi<ApiSource>("/sources", {
    method: "POST",
    body: formData,
  });
}

export async function deleteSource(id: string): Promise<void> {
  return fetchApi<void>(`/sources/${id}`, { method: "DELETE" });
}

// --- Speedwagons ---

export async function getSpeedwagons(): Promise<ApiSpeedwagon[]> {
  return fetchApi<ApiSpeedwagon[]>("/speedwagons");
}

export async function getSpeedwagon(id: string): Promise<ApiSpeedwagon> {
  return fetchApi<ApiSpeedwagon>(`/speedwagons/${id}`);
}

export async function createSpeedwagon(
  data: CreateSpeedwagonRequest,
): Promise<ApiSpeedwagon> {
  return fetchApi<ApiSpeedwagon>("/speedwagons", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function updateSpeedwagon(
  id: string,
  data: UpdateSpeedwagonRequest,
): Promise<ApiSpeedwagon> {
  return fetchApi<ApiSpeedwagon>(`/speedwagons/${id}`, {
    method: "PUT",
    body: JSON.stringify(data),
  });
}

export async function deleteSpeedwagon(id: string): Promise<void> {
  return fetchApi<void>(`/speedwagons/${id}`, { method: "DELETE" });
}

export async function indexSpeedwagon(id: string): Promise<void> {
  return fetchApi<void>(`/speedwagons/${id}/index`, { method: "POST" });
}
