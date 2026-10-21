import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { AppShell } from "@/components/app-shell";
import { HiveMindProblem } from "@/components/hivemind-problem";
import { StatusBadge } from "@/components/status-badge";
import { hivemind } from "@/lib/hivemind/client";
import { decisionIdFromSegment, readDecision } from "@/lib/hivemind/decision";
import { load } from "@/lib/hivemind/load";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Decision · Loomtracer" };

const when = (value: string | null) => value ? new Date(value).toLocaleString("en", { dateStyle: "medium", timeStyle: "short", timeZone: "UTC" }) + " UTC" : "Not recorded";
const decisionHref = (id: string) => `/decisions/${encodeURIComponent(id)}`;
const EVENT_WORDS: Record<string, string> = { recorded: "Recorded", accepted: "Accepted", rejected: "Rejected", superseded: "Superseded" };

export default async function DecisionPage({ params }: { params: Promise<{ id: string }> }) {
  const id = decisionIdFromSegment((await params).id);
  if (id === null) notFound();
  const client = hivemind();
  const result = await load(() => readDecision(client, id));
  if (!result.ok && result.error.code === "not_found") notFound();
  if (!result.ok) return <AppShell active="Search"><HiveMindProblem error={result.error} /></AppShell>;
  const d = result.value;
  return <AppShell active="Search"><article className="content-page" data-testid="decision-detail">
    <header className="page-heading"><div><span className="eyebrow">Decision{d.project ? <> · <Link className="hm-link" href={`/graph?${new URLSearchParams({ project: d.project.ref })}`}>{d.project.label}</Link></> : null}</span><h1>{d.title}</h1></div><p><StatusBadge status={d.status} /></p></header>
    <div className="hm-decision">
      <div>
        {d.rationale && <section><h2>Why</h2><p data-testid="decision-rationale">{d.rationale}</p>{d.quote && <blockquote>{d.quote}</blockquote>}{d.question && <p className="hm-id">Answering: {d.question}</p>}</section>}
        {d.contest && <section><h2>Where people disagree</h2><div className="hm-contest" data-testid="decision-contest">
          <div><h3>Accepted by</h3><ul>{d.contest.acceptedBy.map(actor => <li key={actor}>{actor}</li>)}</ul></div>
          <div><h3>Rejected by</h3><ul>{d.contest.rejectedBy.map(actor => <li key={actor}>{actor}</li>)}</ul></div>
        </div></section>}
        {d.options.length > 0 && <section><h2>What it weighed</h2><ul data-testid="decision-options">{d.options.map(option => <li key={option.id} className={option.chosen ? "hm-chosen" : undefined}>{option.label}</li>)}</ul></section>}
        {(d.assumptions.length > 0 || d.evidence.length > 0 || d.premises.length > 0) && <section><h2>What it rests on</h2><ul data-testid="decision-grounds">
          {d.premises.map(premise => <li key={premise.id}>The decision <Link className="hm-link" href={decisionHref(premise.id)}>{premise.title}</Link> <StatusBadge status={premise.status} /></li>)}
          {d.evidence.map(item => <li key={item.id}>Observed: {item.text}</li>)}
          {d.assumptions.map(item => <li key={item.id}>{item.kind === "bet" ? "Bet" : "Assumed"}: {item.statement} <span className="hm-id">({item.status})</span></li>)}
        </ul></section>}
        {(d.supersedes.length > 0 || d.supersededBy.length > 0) && <section><h2>Replacements</h2><ul data-testid="decision-supersession">
          {d.supersededBy.map(other => <li key={other.id}>Superseded by <Link className="hm-link" href={decisionHref(other.id)}>{other.title}</Link></li>)}
          {d.supersedes.map(other => <li key={other.id}>Supersedes <Link className="hm-link" href={decisionHref(other.id)}>{other.title}</Link></li>)}
        </ul></section>}
        <section><h2>History</h2><ol className="hm-timeline" data-testid="decision-timeline">{d.timeline.map(entry => <li key={entry.citation_id}>
          <strong>{EVENT_WORDS[entry.kind] ?? entry.kind}</strong> by {entry.actor_id}{entry.reason ? <> — {entry.reason}</> : null}
          <small><time dateTime={entry.ts}>{when(entry.ts)}</time> · {entry.citation_id}</small>
        </li>)}</ol></section>
      </div>
      <aside><dl className="hm-facts">
        <div><dt>Decided</dt><dd>{when(d.decidedAt)}</dd></div>
        <div><dt>Recorded by</dt><dd>{d.proposedBy ?? "Not recorded"}</dd></div>
        <div><dt>Accepted by</dt><dd>{d.deciders.length ? d.deciders.map(decider => decider.id).join(", ") : "Nobody yet"}</dd></div>
        {d.binds && <div><dt>What it binds</dt><dd>{d.binds.statement ?? d.binds.level}</dd></div>}
        {d.undoCost && <div><dt>What undoing it costs</dt><dd>{d.undoCost.statement ?? d.undoCost.level}</dd></div>}
        {d.topics.length > 0 && <div><dt>Topics</dt><dd>{d.topics.join(", ")}</dd></div>}
        <div><dt>Id</dt><dd className="hm-id">{d.id}</dd></div>
      </dl></aside>
    </div>
  </article></AppShell>;
}
