import type { LoomtracerError } from "@/lib/loomtracer/client";

const TITLES: Record<LoomtracerError["code"], string> = {
  not_configured: "Connect this workspace to Loomtracer",
  unreachable: "Loomtracer is not answering",
  unauthorized: "Loomtracer refused this workspace's key",
  not_found: "That is not in the ledger",
  rejected: "Loomtracer did not accept the request",
  bad_response: "Loomtracer sent something unreadable",
  server: "Loomtracer failed to answer",
};

export function LoomtracerProblem({ error }: { error: LoomtracerError }) {
  return <section className="content-page hm-problem" role="alert" data-testid="loomtracer-problem">
    <span className="eyebrow">{error.code.replace("_", " ")}</span>
    <h1>{TITLES[error.code]}</h1>
    <p>{error.message}</p>
    {error.code === "not_configured" && <pre>{"LOOMTRACER_API_URL=http://127.0.0.1:8080 \\\nLOOMTRACER_API_KEY=<token, when the server has one> \\\nnpm run dev --workspace=web"}</pre>}
  </section>;
}
