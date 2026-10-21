"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";

interface Member { user_id: string; email: string; role: "owner" | "reviewer" }
interface Invitation { id: string; email: string; expires_at: string; revoked_at: string | null; redeemed_at: string | null }
export function TeamManager({ workspaceName, isOwner, members, invitations, demo }: { workspaceName: string; isOwner: boolean; members: Member[]; invitations: Invitation[]; demo: boolean }) {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [link, setLink] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<Member | null>(null);
  const [now] = useState(() => Date.now());
  async function act(url: string, method: string, body?: object) {
    setBusy(true); setMessage(""); setLink("");
    try {
      const response = await fetch(url, { method, headers: { "Content-Type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error || "Could not update team access. Refresh and try again.");
      if (data.link) { setLink(data.link); setEmail(""); }
      else setMessage("Team access updated.");
      setRemoving(null); router.refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not update team access."); }
    finally { setBusy(false); }
  }
  return <main className="content-page">
    <header className="page-heading"><div><span className="eyebrow">Shared account</span><h1>{workspaceName}</h1></div><p>Every teammate can explore, review decisions, and manage projects. Each person signs in separately and authorizes their own agents.</p></header>
    <section className="settings-card"><h2>Team members</h2><p>All projects in this account are visible to every member and their authorized agents.</p>
      {members.map(member => <article className="connection-row" key={member.user_id}><div><strong>{member.email}</strong><p>{member.role === "owner" ? "Owner · manages membership" : "Member · reviews and manages projects"}</p></div>
        {isOwner && member.role !== "owner" && <button className="button secondary danger" disabled={busy || demo} onClick={() => setRemoving(member)}>Remove access</button>}</article>)}
    </section>
    {isOwner ? <>
      <form className="settings-card organization-form" onSubmit={event => { event.preventDefault(); void act("/api/team/invitations", "POST", { email }); }}>
        <h2>Invite a teammate</h2><label>Google email<input type="email" required maxLength={254} value={email} onChange={event => setEmail(event.target.value)} /></label>
        <p>This invitation joins {workspaceName}. People already in a different account cannot join during this pilot. Links expire after seven days.</p>
        <button className="button primary" disabled={busy || demo}>Create team invitation</button>
        {link && <div><label>Send this invitation link<textarea readOnly value={link} /></label><button className="button secondary" type="button" onClick={async () => { try { await navigator.clipboard.writeText(link); setMessage("Invitation copied."); } catch { setMessage("Select and copy the invitation link."); } }}>Copy invitation</button><p>The complete link is shown only here. No email has been sent.</p></div>}
      </form>
      <section className="settings-card"><h2>Team invitations</h2>{invitations.length === 0 && <p>No team invitations yet.</p>}
        {invitations.map(invitation => <article className="connection-row" key={invitation.id}><div><strong>{invitation.email}</strong><p>{invitation.redeemed_at ? "Accepted" : invitation.revoked_at ? "Revoked" : new Date(invitation.expires_at).getTime() < now ? "Expired" : "Pending"}</p></div>
          {!invitation.redeemed_at && !invitation.revoked_at && <button className="button secondary" disabled={busy || demo} onClick={() => void act(`/api/team/invitations/${invitation.id}`, "DELETE")}>Revoke invitation</button>}</article>)}
      </section>
    </> : <p>Ask the owner to invite or remove teammates. Manage your own agent access in Agent connections.</p>}
    {demo && <p role="status">Local demonstration: membership changes are disabled.</p>}
    {message && <p role="status">{message}</p>}
    {removing && <section className="settings-card" aria-label="Confirm member removal"><h2>Remove {removing.email}?</h2><p>Their access and agent connections will stop immediately. Their authored decisions and history will remain.</p><div className="modal-actions"><button className="button secondary" disabled={busy} onClick={() => setRemoving(null)}>Cancel</button><button className="button secondary danger" disabled={busy} onClick={() => void act(`/api/team/members/${removing.user_id}`, "DELETE")}>{busy ? "Removing…" : "Confirm removal"}</button></div></section>}
  </main>;
}
