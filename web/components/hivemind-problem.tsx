import type { HiveMindError } from "@/lib/hivemind/client";

const TITLES: Record<HiveMindError["code"], string> = {
  not_configured: "Connect this workspace to HiveMind",
  unreachable: "HiveMind is not answering",
  unauthorized: "HiveMind refused this workspace's key",
  not_found: "That is not in the ledger",
  rejected: "HiveMind did not accept the request",
  bad_response: "HiveMind sent something unreadable",
  server: "HiveMind failed to answer",
};

export function HiveMindProblem({ error }: { error: HiveMindError }) {
  return <section className="content-page hm-problem" role="alert" data-testid="hivemind-problem">
    <span className="eyebrow">{error.code.replace("_", " ")}</span>
    <h1>{TITLES[error.code]}</h1>
    <p>{error.message}</p>
    {error.code === "not_configured" && <pre>{"HIVEMIND_API_URL=http://127.0.0.1:8080 \\\nHIVEMIND_API_KEY=<token, when the server has one> \\\nnpm run dev --workspace=web"}</pre>}
  </section>;
}
