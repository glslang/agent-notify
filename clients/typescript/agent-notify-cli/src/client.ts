export type WireAgentState = "running" | "waiting_input" | "done" | "failed";

export interface AgentEventInput {
  agent: string;
  host: string;
  repo?: string;
  state: WireAgentState;
  summary?: string;
  priority?: number;
  ttl_seconds?: number;
  run_id?: string;
}

type Fetch = typeof fetch;

export async function postEvent(
  server: string,
  token: string,
  input: AgentEventInput,
  fetchImpl: Fetch = fetch,
): Promise<void> {
  const response = await fetchImpl(`${trimTrailingSlashes(server)}/v1/events`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(input),
  });

  await ensureSuccess(response);
}

export async function dismissLatest(
  server: string,
  token: string,
  fetchImpl: Fetch = fetch,
): Promise<void> {
  const response = await fetchImpl(`${trimTrailingSlashes(server)}/v1/events/latest`, {
    method: "DELETE",
    headers: {
      authorization: `Bearer ${token}`,
    },
  });

  await ensureSuccess(response);
}

function trimTrailingSlashes(value: string): string {
  return value.replace(/\/+$/, "");
}

async function ensureSuccess(response: Response): Promise<void> {
  if (response.ok) {
    return;
  }

  const body = await response.text().catch(() => "");
  throw new Error(`server returned ${response.status}: ${body}`);
}
