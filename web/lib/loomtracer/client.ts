/**
 * Server-side client for the Loomtracer HTTP API. The base URL and bearer token come from the
 * environment and are read here only: client components never import this module, so the token
 * never reaches a browser (lib/loomtracer/boundary.test.ts holds that line).
 */
import type { AttentionPage, DecisionBrief, Envelope, GraphResponse, ProjectEntry, ProjectsPage, SearchPage, SupersessionChain, Timeline, WaitingPage, Whoami } from "./types";

export type LoomtracerErrorCode = "not_configured" | "unreachable" | "unauthorized" | "not_found" | "rejected" | "bad_response" | "server";

export class LoomtracerError extends Error {
  constructor(message: string, readonly code: LoomtracerErrorCode, readonly status: number) {
    super(message);
    this.name = "LoomtracerError";
  }
}

export type LoomtracerEnv = Record<string, string | undefined>;
export type LoomtracerConfig = { baseUrl: string; token: string | null; tenant: string | null };

export function loomtracerConfig(env: LoomtracerEnv = process.env): LoomtracerConfig {
  const raw = env.LOOMTRACER_API_URL?.trim();
  if (!raw) throw new LoomtracerError("LOOMTRACER_API_URL is not set. Point it at a running `loomtracer serve`, for example http://127.0.0.1:8080.", "not_configured", 503);
  let url: URL;
  try { url = new URL(raw); } catch { throw new LoomtracerError("LOOMTRACER_API_URL is not a valid URL.", "not_configured", 503); }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new LoomtracerError("LOOMTRACER_API_URL must start with http:// or https://.", "not_configured", 503);
  return { baseUrl: `${url.origin}${url.pathname.replace(/\/+$/, "")}`, token: env.LOOMTRACER_API_KEY?.trim() || null, tenant: env.LOOMTRACER_TENANT?.trim() || null };
}

type Params = Record<string, string | number | undefined>;
const TIMEOUT_MS = 15_000;
const PROJECTS_PAGE = 100;
/** A ledger with more projects than this is not a ledger this UI's picker is built for; it says so rather than cutting the list short. */
const MAX_PROJECT_PAGES = 50;

function errorMessage(body: unknown, fallback: string): string {
  const error = (body as { error?: unknown } | null)?.error;
  if (typeof error === "string") return error;
  const message = (error as { message?: unknown } | undefined)?.message;
  return typeof message === "string" ? message : fallback;
}

export function createLoomtracerClient(env: LoomtracerEnv = process.env, fetchImpl: typeof fetch = fetch) {
  async function get<T>(path: string, params: Params = {}): Promise<T> {
    const config = loomtracerConfig(env);
    const url = new URL(`${config.baseUrl}${path}`);
    for (const [key, value] of Object.entries(params)) if (value !== undefined && value !== "") url.searchParams.set(key, String(value));
    const headers: Record<string, string> = { Accept: "application/json" };
    if (config.token) headers.Authorization = `Bearer ${config.token}`;
    if (config.tenant) headers["X-Loomtracer-Tenant"] = config.tenant;
    let response: Response;
    try {
      response = await fetchImpl(url, { headers, cache: "no-store", signal: AbortSignal.timeout(TIMEOUT_MS) });
    } catch {
      throw new LoomtracerError(`Could not reach the Loomtracer server at ${config.baseUrl}. Is \`loomtracer serve\` running?`, "unreachable", 502);
    }
    let body: unknown = null;
    try { body = await response.json(); } catch { /* handled below */ }
    if (response.ok) {
      if (body === null || typeof body !== "object") throw new LoomtracerError("The Loomtracer server sent a response this UI cannot read.", "bad_response", 502);
      return body as T;
    }
    const message = errorMessage(body, `The Loomtracer server answered ${response.status}.`);
    if (response.status === 401 || response.status === 403) throw new LoomtracerError(`${message} Check LOOMTRACER_API_KEY.`, "unauthorized", response.status);
    if (response.status === 404) throw new LoomtracerError(message, "not_found", 404);
    if (response.status >= 400 && response.status < 500) throw new LoomtracerError(message, "rejected", response.status);
    throw new LoomtracerError(message, "server", 502);
  }
  const data = async <T>(path: string, params?: Params) => (await get<Envelope<T>>(path, params)).data;
  const segment = encodeURIComponent;
  return {
    whoami: () => get<Whoami>("/v1/whoami"),
    health: () => get<{ status: string }>("/v1/health"),
    /** One project's newest decisions and what hangs off them. The whole tenant graph (no `project`) is for the pages that still need it. */
    graph: (params: { project?: string; limit?: number; cursor?: string } = {}) => get<GraphResponse>("/v1/graph", params),
    /** One decision and what hangs off it: its options, evidence, assumptions, who proposed and decided it, what it follows from, replaced and was replaced by. The same shape as `graph`, with no paging. */
    decisionGraph: (id: string) => get<GraphResponse>("/v1/graph", { decision: id }),
    /** Every project the ledger holds, registered or personal, with its decision counts. Follows the server's pages to the end. */
    projects: async (): Promise<{ items: ProjectEntry[]; unfiled_decisions: number }> => {
      const items: ProjectEntry[] = [];
      let cursor: string | undefined;
      for (let page = 0; page < MAX_PROJECT_PAGES; page += 1) {
        const result = await data<ProjectsPage>("/v1/projects", { limit: PROJECTS_PAGE, cursor });
        items.push(...result.items);
        if (!result.next_cursor) return { items, unfiled_decisions: result.unfiled_decisions };
        cursor = result.next_cursor;
      }
      throw new LoomtracerError("The ledger holds more projects than this UI lists.", "bad_response", 502);
    },
    search: (params: { q?: string; status?: string; topic?: string; project?: string; limit?: number; cursor?: string }) => data<SearchPage>("/v1/decisions/search", params),
    /** The decision itself. (`compact-view` answers for the head of its supersession chain instead.) */
    decision: (id: string) => data<DecisionBrief>(`/v1/decisions/${segment(id)}`),
    timeline: (id: string) => data<Timeline>(`/v1/decisions/${segment(id)}/timeline`),
    supersessionChain: (id: string) => data<SupersessionChain>(`/v1/decisions/${segment(id)}/supersession-chain`),
    attention: (list: "contested" | "open-why", limit = 25) => data<AttentionPage>(`/v1/attention/${list}`, { limit }),
    waiting: (limit = 25) => data<WaitingPage>("/v1/attention/waiting", { limit }),
  };
}

export type LoomtracerClient = ReturnType<typeof createLoomtracerClient>;
/** The client every page and route uses: configured from the process environment on each call. */
export const loomtracer = (): LoomtracerClient => createLoomtracerClient();
