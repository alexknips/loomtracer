/**
 * Fictional pricing study illustrating how separate explorations can inform
 * a shared proposal. All findings and sources are invented for this sample.
 * Never import private workspace data or imply these are actual study results.
 */
export type PublicDemoKind = "Decision" | "Evidence" | "Attempt";
export type PublicDemoStatus = "Candidate" | "Observed";

export interface PublicDemoNode {
  id: string;
  kind: PublicDemoKind;
  status: PublicDemoStatus;
  title: string;
  summary: string;
  detail: string;
  stage: string;
  source: string;
  reconsider: string[];
  position: { x: number; y: number };
}

export const publicDemoNodes: PublicDemoNode[] = [
  {
    id: "seat-study", kind: "Attempt", status: "Observed",
    title: "Explore per-seat pricing",
    summary: "Predictable bills, but an added cost for occasional collaborators.",
    detail: "In this fictional study, Agent A explores charging for each seat in a collaboration app. A predictable seat count makes spending easier to forecast, but inviting an occasional collaborator creates an added charge. This branch raises a question: how can pricing keep the bill understandable without making teams hesitate to invite people?",
    stage: "Exploration · Agent A", source: "Illustrative study by Agent A; findings invented for this fictional scenario",
    reconsider: [],
    position: { x: 0, y: 0 },
  },
  {
    id: "usage-study", kind: "Attempt", status: "Observed",
    title: "Explore usage-based pricing",
    summary: "Costs follow activity, but the next bill is harder to anticipate.",
    detail: "Agent B explores charging as teams use the collaboration app. In this invented scenario, spending follows activity without requiring a paid seat for every collaborator. A busy month, however, can make the bill uncertain. This branch identifies a trade-off to investigate rather than ruling the model in or out.",
    stage: "Exploration · Agent B", source: "Illustrative study by Agent B; findings invented for this fictional scenario",
    reconsider: [], position: { x: 0, y: 270 },
  },
  {
    id: "shared-criteria", kind: "Evidence", status: "Observed",
    title: "Connect the pricing findings",
    summary: "Teams need room to collaborate and predictable spending.",
    detail: "The two illustrative branches point to shared criteria: teams should be able to bring collaborators into the work and anticipate what they will spend. Per-seat pricing and usage-based pricing each address part of that need while introducing a different concern. Keeping both findings connected gives another agent grounds for exploring a combined approach.",
    stage: "Synthesis · shared findings", source: "Illustrative synthesis of the invented findings from Agent A and Agent B",
    reconsider: [], position: { x: 365, y: 0 },
  },
  {
    id: "team-plan", kind: "Decision", status: "Candidate",
    title: "Test a team plan with included usage",
    summary: "Propose an experiment that combines collaboration with a predictable allowance.",
    detail: "Agent C draws on both studies and the shared criteria to propose testing a team plan with included usage. The experiment would investigate whether teams can collaborate freely while anticipating their spending, including what happens beyond the allowance. This remains a candidate for human review and further testing. No price has been chosen or shown to be optimal.",
    stage: "Proposed next test · Agent C", source: "Illustrative proposal by Agent C, drawing on both fictional pricing studies",
    reconsider: ["Teams still restrict invitations under the proposed plan.", "Bills remain difficult to anticipate when included usage is exceeded.", "A pricing experiment favors a different approach."],
    position: { x: 365, y: 270 },
  },
];

export const publicDemoEdges = [
  { id: "seat-findings", source: "seat-study", target: "shared-criteria", label: "informs", sourceHandle: "right", targetHandle: "left" },
  { id: "usage-findings", source: "usage-study", target: "shared-criteria", label: "informs", sourceHandle: "right", targetHandle: "left" },
  { id: "proposal-support", source: "shared-criteria", target: "team-plan", label: "supports", sourceHandle: "bottom", targetHandle: "top" },
];

export function filterPublicDemo(query: string, kind: "all" | PublicDemoKind) {
  const term = query.trim().toLowerCase();
  return publicDemoNodes.filter((node) => (kind === "all" || node.kind === kind)
    && `${node.title} ${node.summary} ${node.detail} ${node.status}`.toLowerCase().includes(term));
}
