"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { agentSetup, type SetupClient, type SetupProject } from "@/lib/agent-setup";
import styles from "./connection-manager.module.css";

export interface AgentConnection {
  id: string;
  client_name: string;
  permissions: string[];
  granted_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
  last_brief_project_ref: string | null;
  last_brief_fingerprint: string | null;
  last_brief_at: string | null;
}

export function ConnectionManager({ connections, endpoint, projects, workspaceName, membershipRole }: {
  connections: AgentConnection[];
  endpoint: string;
  projects: SetupProject[];
  workspaceName: string;
  membershipRole: "owner" | "reviewer";
}) {
  const router = useRouter();
  const [message, setMessage] = useState("");
  const [pending, setPending] = useState("");
  const [projectRef, setProjectRef] = useState(projects[0]?.ref ?? "");
  const [client, setClient] = useState<SetupClient>("codex");
  const selectedProject = projects.find(project => project.ref === projectRef);
  const setup = selectedProject ? agentSetup(endpoint, selectedProject.ref, client) : null;

  async function disconnect(id: string) {
    setPending(id); setMessage("");
    try {
      const response = await fetch(`/api/connections/${id}`, { method: "DELETE" });
      const data = await response.json();
      if (!response.ok) throw new Error();
      setMessage(data.cleanupPending ? "Access is blocked. Retry Disconnect to finish removing the saved authorization." : "Agent disconnected.");
      router.refresh();
    } catch { setMessage("Could not disconnect. Please try again."); }
    finally { setPending(""); }
  }

  async function copy(value: string, label: string) {
    try { await navigator.clipboard.writeText(value); setMessage(`${label} copied.`); }
    catch { setMessage("Could not copy automatically. Select and copy the text below."); }
  }

  function date(value: string) { return `${new Date(value).toLocaleString("en-US", { timeZone: "UTC" })} UTC`; }

  return <main className="content-page">
    <header className="page-heading"><div><span className="eyebrow">{workspaceName}</span><h1>Agent connections</h1></div>
      <p>Connect an agent, give it the project context, and verify its first briefing.</p></header>
    <section className="settings-card">
      <h2>1. Choose your project and agent</h2>
      <p>You are {membershipRole === "owner" ? "an owner" : "a member"} in <strong>{workspaceName}</strong>. Each teammate connects their own agent.</p>
      {projects.length ? <div className={`organization-form ${styles.choices}`}>
        <label><span>Project for this repository</span><select value={projectRef} onChange={event => setProjectRef(event.target.value)}>
          {projects.map(project => <option key={project.ref} value={project.ref}>{project.name} · {project.ref}</option>)}
        </select></label>
        <label><span>Agent client</span><select value={client} onChange={event => setClient(event.target.value as SetupClient)}>
          <option value="codex">Codex</option><option value="claude">Claude Code</option>
        </select></label>
      </div> : <p><a className="button primary" href="/projects">Create your first project</a></p>}
      <p className={styles.note}>Authorization covers every project in this workspace. This selection tells your agent where to work; it does not limit its access to one project.</p>
    </section>
    {setup && <>
      <section className="settings-card">
        <h2>2. Connect {client === "codex" ? "Codex" : "Claude Code"}</h2>
        <p>Run this in a terminal on the computer where you use your agent. Sign in with your own invited Google account.</p>
        <pre className="setup-code">{setup.commands}</pre>
        <button className="button secondary" onClick={() => copy(setup.commands, "Setup commands")}>Copy {client === "codex" ? "Codex" : "Claude Code"} setup</button>
        {client === "claude" ? <p>Open Claude Code in your repository, run <code>/mcp</code>, select <strong>{setup.server}</strong>, and authenticate in your browser.</p>
          : <p>Complete browser authorization, then restart the MCP connection or start a fresh Codex chat in your repository.</p>}
        <p>Review the workspace and permissions on the consent screen. Read and propose lets agents capture reasoning. Commitment actions require a separate human-owner grant to that named agent for one project. Capture alone never adopts a decision.</p>
        <details className={styles.disclosure}><summary>Already configured, or sign-in did not finish?</summary>
          <p>Keep the existing entry when it points to this endpoint. Skip the add command and authenticate again. If consent expired, restart sign-in from your agent.</p>
          <pre className="setup-code">{client === "codex" ? `codex mcp login ${setup.server} --scopes openid` : `/mcp → ${setup.server} → Authenticate`}</pre>
          <p>For a connection that still uses an old manual token or header helper, replace that configuration with the OAuth setup above. Do not paste a token into your project files.</p>
          <p>Endpoint: <code className={styles.breakable}>{endpoint}</code></p>
        </details>
      </section>
      <section className="settings-card">
        <h2>3. Add the project workflow</h2>
        <p>Add this block to your repository’s <code>AGENTS.md</code>. Preserve the file’s existing instructions; replace only an older Loomtracer or legacy Neurofilament workflow block.</p>
        <button className="button secondary" onClick={() => copy(setup.instructions, "Project workflow")}>Copy AGENTS.md block</button>
        <details className={styles.disclosure}><summary>Preview project instructions</summary><pre className="setup-code">{setup.instructions}</pre></details>
        {client === "claude" && <>
          <p>For Claude Code, add this import to the repository’s <code>CLAUDE.md</code> if it is not already present. Keep its existing content. Create the file with this line if it does not exist.</p>
          <pre className="setup-code">{setup.claudeImport}</pre><button className="button secondary" onClick={() => copy(setup.claudeImport, "CLAUDE.md import")}>Copy CLAUDE.md import</button>
          <p>Start a fresh session and use <code>/context</code> to check the loaded instructions.</p>
        </>}
      </section>
      <section className="settings-card">
        <h2>4. Verify the first briefing</h2>
        <p>Paste this into a fresh agent chat for <strong>{selectedProject?.name}</strong>. It checks the connection without creating graph entries.</p>
        <pre className="setup-code">{setup.verificationPrompt}</pre>
        <div className="detail-actions"><button className="button secondary" onClick={() => copy(setup.verificationPrompt, "Verification prompt")}>Copy verification prompt</button>
          <button className="button secondary" onClick={() => { router.refresh(); setMessage("Connection status refresh requested."); }}>Refresh connection status</button></div>
        <p className={styles.note}>A complete briefing confirms retrieval at that time. Review any scope questions and warnings before applying its guidance.</p>
      </section>
    </>}
    <section className="settings-card"><h2>Your connected agents</h2>
      {connections.length === 0 && <p>No agents connected yet.</p>}
      {connections.map(connection => <article className={`connection-row ${styles.connection}`} key={connection.id}>
        <div><strong>{connection.client_name}</strong>
          <p>{connection.revoked_at ? "Disconnected" : `Authorized · ${connection.permissions.includes("entries:write") ? "Read and propose" : "Read only"}`}</p>
          <dl className={styles.facts}>
            <div><dt>Authorized</dt><dd>{date(connection.granted_at)}</dd></div>
            <div><dt>Last authenticated use</dt><dd>{connection.last_used_at ? date(connection.last_used_at) : "Not used yet"}</dd></div>
            <div><dt>Last complete briefing</dt><dd>{connection.last_brief_at ? <>{date(connection.last_brief_at)}<br /><code>{connection.last_brief_project_ref}</code></> : "Not verified yet"}</dd></div>
          </dl>
          {connection.last_brief_at && <p className={styles.note}>{connection.revoked_at ? "This historical briefing does not restore disconnected access." : connection.last_brief_project_ref === projectRef ? "A complete briefing was retrieved for your selected project." : "Run the verification prompt to check the selected project."}</p>}
        </div>
        <button className="button secondary" disabled={pending === connection.id} onClick={() => disconnect(connection.id)}>{pending === connection.id ? "Disconnecting…" : "Disconnect"}</button>
      </article>)}
    </section>
    {message && <p className={styles.status} role="status">{message}</p>}
    <a className="button primary" href={selectedProject ? `/graph?project=${encodeURIComponent(selectedProject.ref)}` : "/graph"}>Open workspace graph</a>
  </main>;
}
