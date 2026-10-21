import type { Metadata } from "next";
import Link from "next/link";
import { AppShell } from "@/components/app-shell";
import { HiveMindProblem } from "@/components/hivemind-problem";
import { StatusBadge } from "@/components/status-badge";
import { hivemind } from "@/lib/hivemind/client";
import { load } from "@/lib/hivemind/load";
import type { AttentionItem } from "@/lib/hivemind/types";

export const dynamic = "force-dynamic";
export const metadata: Metadata = { title: "Review · Loomtracer" };

const LIMIT = 25;
const day = (value?: string | null) => value ? new Date(value).toLocaleDateString("en", { dateStyle: "medium", timeZone: "UTC" }) : null;
const href = (id: string) => `/decisions/${encodeURIComponent(id)}`;

function Decisions({ items, total, empty, testid }: { items: AttentionItem[]; total: number; empty: string; testid: string }) {
  if (!items.length) return <p className="hm-empty" data-testid={`${testid}-empty`}>{empty}</p>;
  return <>
    <ol className="hm-results" data-testid={testid}>{items.map(item => <li className="hm-result" key={item.decision_id}>
      <div className="hm-result-top"><StatusBadge status={item.status} />{item.project_label && <span>{item.project_label}</span>}{day(item.decided_at ?? item.captured_at) && <span>{day(item.decided_at ?? item.captured_at)}</span>}</div>
      <h2><Link href={href(item.decision_id)}>{item.title}</Link></h2>
      {item.contest && (item.contest.accepted_by || item.contest.rejected_by) && <p>{item.contest.accepted_by?.length ? <>Accepted by {item.contest.accepted_by.join(", ")}. </> : null}{item.contest.rejected_by?.length ? <>Rejected by {item.contest.rejected_by.join(", ")}.</> : null}</p>}
    </li>)}</ol>
    {total > items.length && <p className="hm-count">Showing {items.length} of {total}.</p>}
  </>;
}

export default async function ReviewPage() {
  const client = hivemind();
  const result = await load(() => Promise.all([
    client.search({ status: "proposed", limit: LIMIT }), client.attention("contested", LIMIT), client.waiting(LIMIT), client.attention("open-why", LIMIT),
  ]));
  if (!result.ok) return <AppShell active="Review"><HiveMindProblem error={result.error} /></AppShell>;
  const [proposed, contested, waiting, openWhy] = result.value;
  return <AppShell active="Review"><section className="content-page">
    <header className="page-heading"><div><span className="eyebrow">Review</span><h1>What needs a person</h1></div><p>Read from the ledger each time. A decision that gets answered, replaced or resolved leaves its list on the next load.</p></header>
    <div className="hm-review">
      <section aria-labelledby="proposed-heading"><h2 id="proposed-heading">Proposed</h2><p>Recorded, and nobody has accepted or rejected them yet.</p>
        <Decisions testid="review-proposed" total={proposed.total_matches} empty="Nothing is waiting to be decided." items={proposed.items.map(({ decision }) => ({ decision_id: decision.id, title: decision.title, slug: null, project: decision.project, project_label: decision.project_label, status: decision.status }))} />
      </section>
      <section aria-labelledby="contested-heading"><h2 id="contested-heading">Contested</h2><p>Two people stand on opposite sides. Neither side is chosen for them.</p>
        <Decisions testid="review-contested" total={contested.total_matches} empty="No decision is contested." items={contested.items} />
      </section>
      <section aria-labelledby="waiting-heading"><h2 id="waiting-heading">Questions waiting for a decision</h2><p>Someone asked, and no decision answers it yet.</p>
        {waiting.items.length ? <ol className="hm-results" data-testid="review-waiting">{waiting.items.map(item => <li className="hm-result" key={item.request_id}>
          <div className="hm-result-top"><span>Asked by {item.requested_by}</span>{day(item.asked_at) && <span>{day(item.asked_at)}</span>}</div>
          <h2>{item.text}</h2>
        </li>)}</ol> : <p className="hm-empty" data-testid="review-waiting-empty">No question is waiting.</p>}
      </section>
      <section aria-labelledby="why-heading"><h2 id="why-heading">Decisions with no stated ground</h2><p>In force, binding, and not saying what they rest on.</p>
        <Decisions testid="review-open-why" total={openWhy.total_matches} empty="Every binding decision in force says what it rests on." items={openWhy.items} />
      </section>
    </div>
    <p className="hm-note">This page lists; it does not decide. Accept, disagree with or replace a decision with <code>hivemind review</code>, <code>hivemind disagree</code> and <code>hivemind supersede</code>, or through the MCP tools. The HiveMind HTTP API has no endpoint to accept or reject one.</p>
  </section></AppShell>;
}
