"use client";

import { useEffect, useState } from "react";
import type { AgentTokenRecord } from "@/types/graph";

export function TokenManager({ workspaceId }: { workspaceId: string }) {
  const [tokens, setTokens] = useState<AgentTokenRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [name, setName] = useState("");
  const [platform, setPlatform] = useState("Codex");
  const [expiry, setExpiry] = useState(90);
  const [canWrite, setCanWrite] = useState(true);
  const [secret, setSecret] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    const response = await fetch("/api/tokens", { cache: "no-store" });
    const body = await response.json().catch(() => ({}));
    if (response.ok) setTokens(body.tokens ?? []);
    else setError(body.error ?? "Could not load tokens.");
    setLoading(false);
  }

  useEffect(() => {
    const controller = new AbortController();
    void fetch("/api/tokens", { cache: "no-store", signal: controller.signal })
      .then(async (response) => ({ response, body: await response.json().catch(() => ({})) }))
      .then(({ response, body }) => {
        if (response.ok) setTokens(body.tokens ?? []);
        else setError(body.error ?? "Could not load tokens.");
        setLoading(false);
      })
      .catch(() => undefined);
    return () => controller.abort();
  }, []);

  async function createToken(event: React.FormEvent) {
    event.preventDefault();
    setPending(true);
    setError(null);
    const response = await fetch("/api/tokens", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name, platform, expires_days: expiry, entries_write: canWrite }),
    });
    const body = await response.json().catch(() => ({}));
    setPending(false);
    if (!response.ok) return setError(body.error ?? "Could not create token.");
    setSecret(body.token);
    setCopied(false);
    setAcknowledged(false);
    setName("");
    await refresh();
  }

  async function copySecret() {
    if (!secret) return;
    try {
      await navigator.clipboard.writeText(secret);
      setCopied(true);
    } catch {
      setError("Clipboard access was blocked. Select and copy the token manually, then try Copy token again.");
    }
  }

  async function revoke(id: string) {
    if (!window.confirm("Revoke this token? Connected agents will immediately lose access.")) return;
    const response = await fetch(`/api/tokens/${id}`, { method: "DELETE" });
    if (response.ok) await refresh();
    else setError((await response.json().catch(() => ({}))).error ?? "Could not revoke token.");
  }

  return (
    <main className="content-page token-page">
      <header className="page-heading">
        <div><span className="eyebrow">Agent authentication</span><h1>Access tokens</h1></div>
        <p>Each token carries its own agent identity and bounded permissions. Secrets are shown once.</p>
      </header>
      <section className="token-grid">
        <form className="settings-card token-form" onSubmit={createToken}>
          <div><span className="eyebrow">New credential</span><h2>Create a personal access token</h2></div>
          <label><span>Name / agent identity</span><input value={name} onChange={(event) => setName(event.target.value)} required placeholder="Codex on MacBook" /></label>
          <label><span>Platform</span><input value={platform} onChange={(event) => setPlatform(event.target.value)} placeholder="Codex, Claude Desktop…" /></label>
          <label><span>Expires</span><select value={expiry} onChange={(event) => setExpiry(Number(event.target.value))}><option value={30}>30 days</option><option value={90}>90 days</option><option value={365}>365 days</option></select></label>
          <label className="checkbox-row"><input type="checkbox" checked={canWrite} onChange={(event) => setCanWrite(event.target.checked)} /><span><b>Allow entry proposals</b><small>Can record Attempts and propose Decisions or Assumptions. It can never ratify or reject.</small></span></label>
          <button className="button primary" disabled={pending || !name.trim()}>{pending ? "Creating…" : "Create token"}</button>
        </form>
        <section className="settings-card token-list">
          <div><span className="eyebrow">Issued credentials</span><h2>Connected agents</h2></div>
          {loading && <p>Loading…</p>}
          {tokens.map((token) => {
            const inactive = Boolean(token.revoked_at) || new Date(token.expires_at) <= new Date();
            return <article key={token.id} className={inactive ? "inactive" : ""}>
              <div><b>{token.name}</b><span>{token.actor?.platform || "Agent"} · {token.scopes.includes("entries:write") ? "read + propose" : "read only"}</span></div>
              <code>{token.token_prefix.slice(0, 24)}…</code>
              <time>Expires {new Date(token.expires_at).toLocaleDateString()}</time>
              {inactive ? <span className="status status-rejected">{token.revoked_at ? "revoked" : "expired"}</span> : <button className="text-button danger-text" type="button" onClick={() => revoke(token.id)}>Revoke</button>}
            </article>;
          })}
          {!loading && tokens.length === 0 && <p className="muted-copy">No agent tokens have been created.</p>}
        </section>
      </section>
      <section className="portable-card"><div><span className="eyebrow">Portability</span><h2>Keep your graph recoverable</h2><p>Export the complete hosted node and edge records without credentials.</p><code className="workspace-id">Workspace {workspaceId}</code></div><div><a className="button secondary" href="/api/export?format=jsonl">JSONL</a><a className="button secondary" href="/api/export?format=markdown">Markdown</a></div></section>
      {error && <p className="form-error page-error" role="alert">{error}</p>}
      {secret && (
        <div className="modal-backdrop"><section className="modal secret-modal" role="dialog" aria-modal="true" aria-labelledby="secret-title">
          <span className="eyebrow">Shown once</span><h2 id="secret-title">Save this token now</h2>
          <p>Neurofilament stores only its SHA-256 hash. This secret cannot be recovered later.</p>
          <code className="secret-value">{secret}</code>
          <button className="button primary" type="button" onClick={copySecret}>{copied ? "Copied" : "Copy token"}</button>
          <label className="checkbox-row acknowledge"><input type="checkbox" checked={acknowledged} onChange={(event) => setAcknowledged(event.target.checked)} /><span>I have stored this token securely.</span></label>
          <button className="button secondary" type="button" disabled={!copied || !acknowledged} onClick={() => setSecret(null)}>Close</button>
        </section></div>
      )}
    </main>
  );
}
