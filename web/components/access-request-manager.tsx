"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import styles from "@/components/workspace-administration.module.css";

interface AccessRequest { id: string; email: string; use_case: string; status: string; created_at: string }
export function AccessRequestManager({ requests, demo = false }: { requests: AccessRequest[]; demo?: boolean }) {
  const router = useRouter();
  const [busy, setBusy] = useState("");
  const [message, setMessage] = useState("");
  const [workspaceNames, setWorkspaceNames] = useState<Record<string, string>>({});
  const [invitation, setInvitation] = useState<{ email: string; workspace_name: string; link: string } | null>(null);
  async function review(id: string, action: "approve" | "dismiss") {
    if (demo || busy) return;
    const workspaceName = workspaceNames[id]?.trim() ?? "";
    if (action === "approve" && (!workspaceName || workspaceName.length > 80)) {
      setMessage("Enter a workspace name between 1 and 80 characters before preparing the invitation."); return;
    }
    setBusy(id); setMessage("");
    try {
      const body = action === "approve" ? { action, workspaceName } : { action };
      const response = await fetch(`/api/admin/access-requests/${id}`, { method: "PATCH", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error || "Could not review this request.");
      if (data.link) {
        setInvitation({ email: data.email, workspace_name: data.workspace_name, link: data.link });
        setMessage("Owner invitation prepared. Copy the link and send it to the invited person.");
      } else setMessage("Request dismissed.");
      router.refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not review this request."); }
    finally { setBusy(""); }
  }
  return <main className={`content-page ${styles.page}`}>
    <header className="page-heading"><div><span className="eyebrow">Pilot administration</span><h1>Access requests</h1></div><p>Review how people want to use Loomtracer. Prepare an owner invitation with a name for their separate workspace.</p></header>
    {demo && <p role="status" className={styles.notice}>Local demonstration: access-request changes are disabled.</p>}
    {invitation && <section className={`settings-card organization-form ${styles.created}`} aria-label="Prepared owner invitation">
      <h2>Invitation for {invitation.email}</h2><p>Accepting will create <strong>{invitation.workspace_name}</strong>, a separate workspace with no projects or decisions yet.</p>
      <label>Send this link manually<textarea readOnly value={invitation.link} rows={3} onFocus={event => event.target.select()} /></label>
      <button className="button secondary" onClick={async () => { try { await navigator.clipboard.writeText(invitation.link); setMessage("Invitation copied."); } catch { setMessage("Select and copy the invitation link."); } }}>Copy invitation</button>
      <p>The full link is shown only here. No email has been sent. If you lose it, create a replacement in Invitations.</p>
    </section>}
    <section className="settings-card"><h2>Requests</h2>{requests.length === 0 && <p>No access requests yet.</p>}
      {requests.map(request => <article className={`review-card ${styles.request}`} key={request.id} aria-label={`Access request from ${request.email}`}>
        <div className="review-card-top"><strong>{request.email}</strong><span>{request.status}</span></div>
        <p>{request.use_case}</p><small>Requested {request.created_at.slice(0, 10)} (UTC)</small>
        {request.status === "pending" && <form className={`organization-form ${styles.requestForm}`} onSubmit={event => { event.preventDefault(); void review(request.id, "approve"); }}>
          <label>Workspace name<input type="text" autoComplete="organization" required maxLength={80} disabled={!!busy || demo} value={workspaceNames[request.id] ?? ""} onChange={event => setWorkspaceNames(names => ({ ...names, [request.id]: event.target.value }))} /></label>
          <p className={styles.help}>The invited person will own this new workspace. Its projects and decisions will be separate from yours.</p>
          <div className="review-actions">
            <button type="button" className="button secondary" disabled={!!busy || demo} onClick={() => void review(request.id, "dismiss")}>Dismiss</button>
            <button type="submit" className="button primary" disabled={!!busy || demo}>{busy === request.id ? "Saving…" : "Prepare owner invitation"}</button>
          </div>
        </form>}
      </article>)}
    </section>{message && <p role="status">{message}</p>}
  </main>;
}
