"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import styles from "@/components/workspace-administration.module.css";

interface Invitation {
  id: string;
  email: string;
  kind: "owner";
  workspace_id: string | null;
  workspace_name: string | null;
  expires_at: string;
  revoked_at: string | null;
  redeemed_at: string | null;
}
interface Account {
  user_id: string;
  email: string;
  status: string;
  is_admin: boolean;
  workspace_id?: string | null;
  workspace_name?: string | null;
  membership_role?: "owner" | "reviewer" | null;
}
interface CreatedInvitation { id: string; email: string; workspace_name: string; link: string }

export function InvitationManager({ invitations, accounts, enabled, demo = false }: {
  invitations: Invitation[];
  accounts: Account[];
  enabled: boolean;
  demo?: boolean;
}) {
  const router = useRouter();
  const [now] = useState(() => Date.now());
  const [email, setEmail] = useState("");
  const [workspaceName, setWorkspaceName] = useState("");
  const [created, setCreated] = useState<CreatedInvitation | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  async function act(url: string, method: string, body?: object) {
    if (demo || busy) return;
    setBusy(true); setMessage("");
    try {
      const response = await fetch(url, { method, headers: { "Content-Type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error || "Could not complete that action. Please try again.");
      if (data.link) {
        setCreated(data); setEmail(""); setWorkspaceName("");
        setMessage("Owner invitation prepared. Copy the link and send it to the invited person.");
      } else {
        if (method === "DELETE" && created && url.endsWith(`/${created.id}`)) setCreated(null);
        setMessage("Access updated.");
      }
      router.refresh();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "Could not complete that action. Please try again.");
    } finally { setBusy(false); }
  }
  return <main className={`content-page ${styles.page}`}>
    <header className="page-heading"><div><span className="eyebrow">Pilot administration</span><h1>Invitations</h1></div>
      <p>Invite an owner into a separate workspace. They can invite teammates from Team settings after accepting.</p>
    </header>
    {(!enabled || demo) && <p className={styles.notice} role="status">
      {!enabled && "Tester admission is closed while release verification is pending. "}
      {demo ? "Local demonstration: invitation and account changes are disabled." : "Invitation links can be prepared now; acceptance opens when admission is enabled."}
    </p>}
    <form className="settings-card organization-form" onSubmit={event => {
      event.preventDefault();
      const name = workspaceName.trim();
      if (!name || name.length > 80) { setMessage("Enter a workspace name between 1 and 80 characters."); return; }
      void act("/api/invitations", "POST", { email: email.trim(), workspaceName: name });
    }}>
      <h2>Invite a team owner</h2>
      <div className={styles.fields}>
        <label>Workspace name<input type="text" autoComplete="organization" required maxLength={80} value={workspaceName} disabled={busy || demo} onChange={event => setWorkspaceName(event.target.value)} /></label>
        <label>Google email<input type="email" autoComplete="email" required maxLength={254} value={email} disabled={busy || demo} onChange={event => setEmail(event.target.value)} /></label>
      </div>
      <p>Accepting creates a new, empty workspace with this name. Its projects and decisions are separate from yours. One person can belong to one workspace during this pilot.</p>
      <p className={styles.help}>Links expire after seven days. A replacement invitation for the same person invalidates their previous unused owner link. No email is sent automatically.</p>
      <button className="button primary" disabled={busy || demo}>{busy ? "Saving…" : "Create invitation link"}</button>
    </form>
    {created && <section className={`settings-card organization-form ${styles.created}`} aria-label="Prepared owner invitation">
      <h2>Invitation for {created.email}</h2>
      <p>They will own <strong>{created.workspace_name}</strong>, a new, separate workspace.</p>
      <label>Send this invitation link<textarea readOnly value={created.link} rows={3} onFocus={event => event.target.select()} /></label>
      <button className="button secondary" type="button" onClick={async () => {
        try { await navigator.clipboard.writeText(created.link); setMessage("Invitation link copied."); }
        catch { setMessage("Select and copy the invitation link."); }
      }}>Copy invitation link</button>
      <small>The complete link is shown only here. No email has been sent. If you lose it, create a replacement invitation.</small>
    </section>}
    <section className="settings-card"><h2>Owner invitations</h2>
      {invitations.length === 0 && <p>No owner invitations yet.</p>}
      {invitations.map(invitation => {
        const expired = new Date(invitation.expires_at).getTime() <= now;
        const status = invitation.redeemed_at ? "Accepted" : invitation.revoked_at ? "Revoked" : expired ? "Expired" : "Pending";
        return <article className="connection-row" key={invitation.id}>
          <div><strong>{invitation.email}</strong><p>{invitation.workspace_name ?? "New workspace"} · Owner</p>
            <p className={styles.help}>{status}{status === "Pending" ? ` · Expires ${invitation.expires_at.slice(0, 10)} (UTC)` : ""}</p>
          </div>
          {!invitation.redeemed_at && !invitation.revoked_at && !expired && <button className="button secondary" disabled={busy || demo} onClick={() => void act(`/api/invitations/${invitation.id}`, "DELETE")}>Revoke</button>}
        </article>;
      })}
    </section>
    <section className="settings-card"><h2>Admitted people</h2>
      <p>Suspension blocks this person and their agent connections. Administration does not grant access to their workspace’s graph.</p>
      {accounts.length === 0 && <p>No admitted people to display.</p>}
      {accounts.map(account => <article className="connection-row" key={account.user_id}>
        <div><strong>{account.email}</strong>
          <p>{account.workspace_name ? `${account.workspace_name} · ${account.membership_role === "owner" ? "Owner" : "Member"}` : "No workspace membership"}</p>
          <p className={styles.help}>{account.status === "active" ? "Active" : "Suspended"}{account.is_admin ? " · Platform admin" : ""}</p>
        </div>
        {!account.is_admin && <button className="button secondary" disabled={busy || demo} onClick={() => void act(`/api/accounts/${account.user_id}`, "PATCH", { status: account.status === "active" ? "suspended" : "active" })}>{account.status === "active" ? "Suspend access" : "Restore access"}</button>}
      </article>)}
    </section>
    {message && <p role="status">{message}</p>}
  </main>;
}
