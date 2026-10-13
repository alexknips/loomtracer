/** Writing guidance only: never transforms, truncates or validates authored records. */
export const CONCISE_CAPTURE_GUIDANCE = [
  "Capture reusable findings, constraints, tested outcomes, tradeoffs or unresolved dependencies that could change future work or prevent repeated investigation.",
  "Skip routine status and duplicates; zero entries is valid.",
  "Keep one reusable finding with its qualifications per node and use a specific, searchable title.",
  "Summarize the finding and when it applies; add only necessary rationale, limits, exceptions or open questions in the body.",
  "Distinguish observations from interpretations and proposed actions; do not generalize beyond the evidence.",
  "Preserve exact names, quantities, conditions, contrary evidence and material rejected alternatives.",
  "Put the supporting basis in evidence_summary and a stable, retrievable pointer in source_ref; avoid repeating prose across fields.",
  "Propose accurate links to relevant existing records when supported by the tools.",
  "Keep observations, assumptions and proposed decisions distinct from human approval.",
  "Use the shortest wording that preserves those facts; length is not a reason to drop necessary information.",
].join(" ");

export function withCaptureGuidance(instructions: string) {
  // Independent default; `off` or an unknown value restores the original text.
  const configured = process.env.NEUROFILAMENT_CAPTURE_GUIDANCE;
  return configured === undefined || configured === "" || configured === "concise-v1"
    ? `${instructions}\n\n${CONCISE_CAPTURE_GUIDANCE}`
    : instructions;
}
