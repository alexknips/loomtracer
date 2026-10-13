import type { BriefRequest } from "./task-brief";

export const DEFAULT_BRIEF_DELIVERY = "compact";
export const DEFAULT_BRIEF_RESPONSE_BYTES = 64000;
export const DEFAULT_BRIEF_TASK_EVIDENCE_NODES = 1;

/** Bounded delivery is independent of the experimental pilot feature switches. */
export function boundedBriefsEnabled() {
  const configured = process.env.NEUROFILAMENT_BRIEF_DELIVERY;
  // `legacy` is the rollback; unknown values also retain the incumbent path.
  return configured === undefined || configured === "" || configured === "compact";
}

/** Fill omitted delivery choices without enabling stored baselines or deltas. */
export function withBriefDefaults(request: BriefRequest): BriefRequest {
  if (!boundedBriefsEnabled()) return request;
  return {
    ...request,
    delivery: request.delivery ?? DEFAULT_BRIEF_DELIVERY,
    max_response_bytes: request.max_response_bytes ?? DEFAULT_BRIEF_RESPONSE_BYTES,
    max_task_evidence_nodes: request.max_task_evidence_nodes ?? DEFAULT_BRIEF_TASK_EVIDENCE_NODES,
  };
}
