import type { Metadata } from "next";
import Link from "next/link";
import { AppShell } from "@/components/app-shell";
import { HiveMindProblem } from "@/components/hivemind-problem";
import { StatusBadge } from "@/components/status-badge";
import { hivemind } from "@/lib/hivemind/client";
import { load } from "@/lib/hivemind/load";
import { STATUSES } from "@/lib/hivemind/projects";
import type { DecisionStatus } from "@/lib/hivemind/types";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Search · Loomtracer" };

const PAGE_SIZE = 20;
type Params = { q?: string; status?: string; cursor?: string };
const excerpt = (text: string | null) => !text ? "" : text.length > 240 ? `${text.slice(0, 237).trimEnd()}…` : text;

export default async function SearchPage({ searchParams }: { searchParams: Promise<Params> }) {
  const params = await searchParams;
  const q = (params.q ?? "").trim();
  const status = STATUSES.find(item => item === params.status);
  const result = await load(() => hivemind().search({ q: q || undefined, status, limit: PAGE_SIZE, cursor: params.cursor || undefined }));
  const more = (cursor: string) => `/search?${new URLSearchParams({ ...(q ? { q } : {}), ...(status ? { status } : {}), cursor })}`;
  return <AppShell active="Search"><section className="content-page">
    <header className="page-heading"><div><span className="eyebrow">Search</span><h1>What was decided</h1></div><p>Words from a title, a rationale, an option or a topic. Every result opens the decision with its reasons.</p></header>
    <form className="hm-search" role="search" action="/search">
      <input name="q" type="search" aria-label="Search decisions" placeholder="Search decisions, for example “seat pricing”" defaultValue={q} />
      <select name="status" aria-label="Status" defaultValue={status ?? ""}><option value="">Any status</option>{STATUSES.map((item: DecisionStatus) => <option key={item} value={item}>{item[0]!.toUpperCase()}{item.slice(1)}</option>)}</select>
      <button className="button primary" type="submit">Search</button>
    </form>
    {!result.ok ? <HiveMindProblem error={result.error} /> : <>
      <p className="hm-count" data-testid="search-count" role="status">{result.value.total_matches} {result.value.total_matches === 1 ? "decision" : "decisions"}{q ? <> matching “{q}”</> : null}{status ? <> that are {status}</> : null}</p>
      {result.value.items.length ? <ol className="hm-results">{result.value.items.map(({ decision, matched_fields }) => <li className="hm-result" key={decision.id} data-testid="search-result">
        <div className="hm-result-top"><StatusBadge status={decision.status} />{decision.project_label && <span>{decision.project_label}</span>}</div>
        <h2><Link href={`/decisions/${encodeURIComponent(decision.id)}`}>{decision.title}</Link></h2>
        <p>{excerpt(decision.rationale)}</p>
        {matched_fields.length > 0 && <small>Matched in {[...new Set(matched_fields.map(field => field.replace("decision.", "").replace("option.", "option ").replace("_", " ")))].join(", ")}</small>}
      </li>)}</ol> : <p className="hm-empty">Nothing in the ledger matches. Try fewer or different words.</p>}
      {result.value.next_cursor && <p className="hm-more"><Link className="button secondary" href={more(result.value.next_cursor)}>More results</Link></p>}
    </>}
  </section></AppShell>;
}
