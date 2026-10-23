import type { Metadata } from "next";
import Link from "next/link";
import { AppShell } from "@/components/app-shell";
import { LoomtracerProblem } from "@/components/loomtracer-problem";
import { StatusBadge } from "@/components/status-badge";
import { loomtracer } from "@/lib/loomtracer/client";
import { load } from "@/lib/loomtracer/load";
import { projectPath, STATUSES, summarizeProjects } from "@/lib/loomtracer/projects";
import type { DecisionStatus, SearchPage } from "@/lib/loomtracer/types";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Search · Loomtracer" };

const PAGE_SIZE = 20;
type Params = { q?: string; status?: string; project?: string; cursor?: string };
const excerpt = (text: string | null) => !text ? "" : text.length > 240 ? `${text.slice(0, 237).trimEnd()}…` : text;

export default async function SearchPage({ searchParams }: { searchParams: Promise<Params> }) {
  const params = await searchParams;
  const q = (params.q ?? "").trim();
  const status = STATUSES.find(item => item === params.status);
  const project = (params.project ?? "").trim();
  const result = await load(async () => {
    const client = loomtracer();
    const [page, listed] = await Promise.all([client.search({ q: q || undefined, status, project: project || undefined, limit: PAGE_SIZE, cursor: params.cursor || undefined }), client.projects()]);
    return { page, projects: summarizeProjects(listed.items) };
  });
  const more = (cursor: string) => `/search?${new URLSearchParams({ ...(q ? { q } : {}), ...(status ? { status } : {}), ...(project ? { project } : {}), cursor })}`;
  return <AppShell active="Search"><section className="content-page">
    <header className="page-heading"><div><span className="eyebrow">Search</span><h1>What was decided</h1></div><p>Words from a title, a rationale, an option or a topic. Every result opens the decision with its reasons.</p></header>
    <form className="hm-search" role="search" action="/search">
      <input name="q" type="search" aria-label="Search decisions" placeholder="Search decisions, for example “seat pricing”" defaultValue={q} />
      {result.ok && result.value.projects.length > 0 && <select name="project" aria-label="Project" defaultValue={project}><option value="">Any project</option>{result.value.projects.map(item => <option key={item.ref} value={item.ref}>{projectPath(result.value.projects, item.ref)}</option>)}</select>}
      <select name="status" aria-label="Status" defaultValue={status ?? ""}><option value="">Any status</option>{STATUSES.map((item: DecisionStatus) => <option key={item} value={item}>{item[0]!.toUpperCase()}{item.slice(1)}</option>)}</select>
      <button className="button primary" type="submit">Search</button>
    </form>
    {!result.ok ? <LoomtracerProblem error={result.error} /> : <Results page={result.value.page} q={q} status={status} more={more} />}
  </section></AppShell>;
}

function Results({ page, q, status, more }: { page: SearchPage; q: string; status: DecisionStatus | undefined; more: (cursor: string) => string }) {
  return <>
      <p className="hm-count" data-testid="search-count" role="status">{page.total_matches} {page.total_matches === 1 ? "decision" : "decisions"}{q ? <> matching “{q}”</> : null}{status ? <> that are {status}</> : null}{page.scope ? <> asked from {page.scope.project_label}</> : null}</p>
      {page.scope && <p className="hm-note" data-testid="search-scope">{page.scope.note}</p>}
      {page.items.length ? <ol className="hm-results">{page.items.map(({ decision, matched_fields, scope }) => <li className="hm-result" key={decision.id} data-testid="search-result">
        <div className="hm-result-top"><StatusBadge status={decision.status} />{decision.project_label && <span>{decision.project_label}</span>}{scope && scope.relation !== "own" && <span data-testid="search-result-scope">{scope.label}</span>}</div>
        <h2><Link href={`/decisions/${encodeURIComponent(decision.id)}`}>{decision.title}</Link></h2>
        <p>{excerpt(decision.rationale)}</p>
        {matched_fields.length > 0 && <small>Matched in {[...new Set(matched_fields.map(field => field.replace("decision.", "").replace("option.", "option ").replace("_", " ")))].join(", ")}</small>}
      </li>)}</ol> : <p className="hm-empty">Nothing in the ledger matches. Try fewer or different words.</p>}
      {page.next_cursor && <p className="hm-more"><Link className="button secondary" href={more(page.next_cursor)}>More results</Link></p>}
  </>;
}
