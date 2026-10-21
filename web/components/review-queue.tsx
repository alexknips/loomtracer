"use client";

import { useState } from "react";
import type { ProjectRecord, ReviewRecord } from "@/types/graph";
import { projectPath } from "@/lib/projects";

type Action = { kind: "ratify" | "reject"; node: ReviewRecord } | null;

export function ReviewQueue({ initialRecords, projects = [] }: { initialRecords: ReviewRecord[]; projects?: ProjectRecord[] }) {
  const [records, setRecords] = useState(initialRecords);
  const [action, setAction] = useState<Action>(null);
  const [conditions, setConditions] = useState("");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  function open(next: NonNullable<Action>) {
    setAction(next);
    setConditions(next.node.reconsider_conditions.join("\n"));
    setReason("");
    setError(null);
  }

  async function submit() {
    if (!action) return;
    const reconsider = conditions.split("\n").map((item) => item.trim()).filter(Boolean);
    if (action.kind === "ratify" && action.node.type === "Decision" && reconsider.length === 0) {
      setError("A ratified Decision needs at least one reconsider condition.");
      return;
    }
    if (action.kind === "reject" && !reason.trim()) {
      setError("A rejection reason is required.");
      return;
    }
    setPending(true);
    setError(null);
    try {
    const response = await fetch(`/api/review/${encodeURIComponent(action.node.id)}/${action.kind}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ expected_updated_at: action.node.updated_at, ...(action.kind === "ratify" ? { reconsider_conditions: reconsider } : { reason: reason.trim() }) }),
    });
    const result = await response.json().catch(() => ({}));
    if (!response.ok) {
      setError(result.error ?? "The review action failed.");
      setPending(false);
      return;
    }
    setRecords((current) => current.filter((record) => record.id !== action.node.id));
    setAction(null);
    } catch {
      setError("The review could not be saved. Check your connection and try again.");
    }
    setPending(false);
  }

  return (
    <main className="content-page review-page">
      <header className="page-heading">
        <div><span className="eyebrow">Human checkpoint</span><h1>Review queue</h1></div>
        <p>Agents suggest durable guidance. Human teammates decide what becomes authoritative, with each approval attributed to its reviewer.</p>
      </header>
      <div className="review-summary"><b>{records.length}</b><span>items awaiting judgment</span></div>
      <section className="review-list" aria-label="Entries awaiting review">
        {records.map((node) => (
          <article className="review-card" key={node.id} id={node.id}>
            <div className="review-card-top">
              <div><span className="kind-label">{node.type}</span><span className={`status status-${node.status}`}>{node.status}</span></div>
              <time>{new Date(node.created_at).toLocaleString()}</time>
            </div>
            <h2>{node.title}</h2>
            <p>{node.summary || node.body || "No summary provided."}</p>
            <details>
              <summary>Full record and evidence</summary>
              <p style={{ whiteSpace: "pre-wrap" }}>{node.body || "No additional body."}</p>
              <p><b>Evidence:</b> {String(node.metadata.evidence_summary || "Not supplied")}</p>
              <p><b>Source:</b> {String(node.metadata.source_ref || "Not supplied")}</p>
              <p><b>Record ID:</b> {node.id}</p>
            </details>
            <dl className="review-meta">
              <div><dt>Project</dt><dd>{projectPath(projects, node.project_id)}</dd></div>
              <div><dt>Proposed by</dt><dd>{node.actor?.display_name ?? "Unknown agent"}</dd></div>
              <div><dt>Scope</dt><dd>{[node.scope_type, ...node.scope_refs].filter(Boolean).join(" · ") || "Unspecified"}</dd></div>
            </dl>
            {node.proposed_supersedes && (
              <div className="supersession-callout">
                <b>Atomic supersession</b>
                <span>Approval will activate this Decision and supersede “{node.proposed_supersedes.title}”.</span>
              </div>
            )}
            {node.reconsider_conditions.length > 0 && (
              <div className="condition-preview"><b>Reconsider if</b><ul>{node.reconsider_conditions.map((item) => <li key={item}>{item}</li>)}</ul></div>
            )}
            <div className="review-actions">
              <button className="button secondary danger" type="button" onClick={() => open({ kind: "reject", node })}>Reject</button>
              <button className="button primary" type="button" onClick={() => open({ kind: "ratify", node })}>{node.type === "Assumption" ? "Accept" : "Ratify"}</button>
            </div>
          </article>
        ))}
        {records.length === 0 && <div className="empty-state"><span>✓</span><h2>Queue clear</h2><p>No candidate Decisions or proposed Assumptions need review.</p></div>}
      </section>
      {action && (
        <div className="modal-backdrop" role="presentation">
          <section className="modal" role="dialog" aria-modal="true" aria-labelledby="review-dialog-title">
            <span className="eyebrow">Confirm {action.kind}</span>
            <h2 id="review-dialog-title">{action.node.title}</h2>
            {action.kind === "ratify" ? (
              <label><span>Reconsider conditions {action.node.type === "Decision" && "(required)"}</span><textarea value={conditions} onChange={(event) => setConditions(event.target.value)} rows={5} placeholder="One condition per line" /></label>
            ) : (
              <label><span>Why reject this entry? (required)</span><textarea value={reason} onChange={(event) => setReason(event.target.value)} rows={4} /></label>
            )}
            {action.node.proposed_supersedes && action.kind === "ratify" && <p className="modal-warning">This commits both lifecycle changes in one transaction. A stale supersession target will abort approval.</p>}
            {error && <p className="form-error" role="alert">{error}</p>}
            <div className="modal-actions">
              <button className="button secondary" type="button" onClick={() => setAction(null)} disabled={pending}>Cancel</button>
              <button className={`button ${action.kind === "reject" ? "danger" : "primary"}`} type="button" onClick={submit} disabled={pending}>{pending ? "Saving…" : "Confirm"}</button>
            </div>
          </section>
        </div>
      )}
    </main>
  );
}
