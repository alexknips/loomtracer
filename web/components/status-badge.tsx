import type { DecisionStatus } from "@/lib/loomtracer/types";

const WORDS: Record<DecisionStatus, string> = {
  proposed: "Proposed", accepted: "Accepted", rejected: "Rejected", contested: "Contested", superseded: "Superseded",
};

/** Contested is a status of its own, drawn like the others: never folded into accepted or rejected. */
export function StatusBadge({ status }: { status: DecisionStatus }) {
  return <span className={`hm-status hm-status-${status}`} data-status={status}>{WORDS[status] ?? status}</span>;
}
