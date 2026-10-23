'use client';

import { useEffect, useRef, useState } from 'react';
import Link from 'next/link';
import type { Actor, ReasoningBundle } from '@reasoning';

export type WorkspaceActor = Actor;
/** The project open in the workspace, and where in the ledger it was read. */
export type HostedState = { projectRef: string; head: string };
type Project = { ref: string; name: string; decisions: number };
/** How many of the project's decisions the bundle draws, of how many it holds. */
type Shown = { shown: number; total: number; truncated: boolean };
type Payload = { error?: string; projects?: Project[]; bundle?: ReasoningBundle; head?: string; decisions?: Shown };
/** The most decisions Loomtracer returns for one project in one read. */
const MOST_DECISIONS = 1000;

async function request(url: string, signal: AbortSignal): Promise<Payload> {
  const response = await fetch(url, { cache: 'no-store', signal });
  const result = await response.json().catch(() => ({})) as Payload;
  if (!response.ok) throw new Error(result.error ?? `The server returned ${response.status}.`);
  return result;
}

export function HostedControls({ mode, initialProject, hosted, setHosted, onLoad, onError }: {
  mode: 'local' | 'hosted'; initialProject: string; hosted: HostedState | null;
  setHosted: (state: HostedState | null) => void; onLoad: (bundle: ReasoningBundle, message: string, preserveView?: boolean) => boolean; onError: (message: string) => void;
}) {
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [projectRef, setProjectRef] = useState(initialProject);
  const [reload, setReload] = useState(0);
  // How many of the newest decisions to read; null is Loomtracer's own page.
  const [limit, setLimit] = useState<number | null>(null);
  const [shown, setShown] = useState<Shown | null>(null);
  // Which read has finished (or failed): a read is under way while it is not the one asked for.
  const [loaded, setLoaded] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const readKey = `${projectRef}#${reload}#${limit}`;
  const busy = mode === 'hosted' && !failed && (projects === null || (projectRef !== '' && loaded !== readKey));
  const callbacks = useRef({ onError, onLoad, setHosted });
  useEffect(() => { callbacks.current = { onError, onLoad, setHosted }; }, [onError, onLoad, setHosted]);

  useEffect(() => {
    if (mode !== 'hosted') return;
    const controller = new AbortController();
    void request('/api/reasoning', controller.signal).then(data => {
      const list = data.projects ?? [];
      setProjects(list);
      // Open the asked-for project, else the first one that holds a decision.
      setProjectRef(ref => list.some(project => project.ref === ref) ? ref : (list.find(project => project.decisions > 0) ?? list[0])?.ref ?? '');
    }).catch(error => { if (!controller.signal.aborted) { callbacks.current.onError(error.message); setFailed(true); } });
    return () => controller.abort();
  }, [mode]);

  const ready = projects !== null;
  useEffect(() => {
    if (mode !== 'hosted' || !ready || !projectRef) return;
    const controller = new AbortController();
    void request(`/api/reasoning?${new URLSearchParams({ project_ref: projectRef, ...(limit ? { limit: String(limit) } : {}) })}`, controller.signal).then(data => {
      if (controller.signal.aborted || !data.bundle) return;
      setShown(data.decisions ?? null);
      if (callbacks.current.onLoad(data.bundle, `${data.bundle.title} read from the Loomtracer ledger.`, false)) callbacks.current.setHosted({ projectRef, head: data.head ?? '' });
      window.history.replaceState(null, '', `/graph?${new URLSearchParams({ project: projectRef })}`);
    }).catch(error => { if (!controller.signal.aborted) callbacks.current.onError(error.message); })
      .finally(() => { if (!controller.signal.aborted) setLoaded(readKey); });
    return () => controller.abort();
  }, [mode, ready, projectRef, reload, limit, readKey]);

  return <section className="hosted-controls" aria-label="Project history">
    <div className="hosted-controls-top"><div><p className="eyebrow">{mode === 'local' ? 'Private inspection' : 'Your ledger'}</p><p className="muted">{mode === 'local' ? 'Opening a file does not upload it. Nothing here is sent to a server.' : 'A read-only view of the decisions Loomtracer holds. Follow each one to what it weighed, what it rests on and who decided.'}</p></div><Link className="text-button" href={mode === 'local' ? '/graph' : '/inspect'}>{mode === 'local' ? 'Open your projects →' : 'Inspect a private file →'}</Link></div>
    {mode === 'hosted' && projects && (projects.length ? <div className="project-controls">
      <label>Project<select aria-label="Reasoning project" value={projectRef} disabled={busy} onChange={event => { setHosted(null); setShown(null); setLimit(null); setProjectRef(event.target.value); }}>{projects.map(project => <option key={project.ref} value={project.ref}>{project.name} ({project.decisions})</option>)}</select></label>
      <button className="text-button" disabled={busy} onClick={() => { setHosted(null); setReload(count => count + 1); }}>Reload from the ledger</button>
      {hosted && <span className="muted" data-testid="ledger-position">Ledger position {hosted.head}</span>}
      {hosted && shown?.truncated && <span className="muted" data-testid="decisions-shown">The newest {shown.shown} of {shown.total} decisions are drawn.</span>}
      {hosted && shown?.truncated && shown.shown < MOST_DECISIONS && <button className="text-button" disabled={busy} data-testid="show-more-decisions" onClick={() => { setHosted(null); setLimit(MOST_DECISIONS); }}>Draw up to {MOST_DECISIONS}</button>}
      {busy && <span role="status">Reading the ledger…</span>}
    </div> : <p className="muted" data-testid="no-projects">This ledger holds no decisions yet. Capture one with <code>loomtracer emit decision.capture</code> and reload.</p>)}
  </section>;
}
