/** Candidate behavior is opt-in on the isolated pilot server; never inferred from client input. */
export function pilotEnabled() {
  return process.env.NEUROFILAMENT_PILOT_ENABLED === "1";
}

export type PilotFeature = "briefs" | "plan-scope" | "capture" | "review";
export function pilotFeatureEnabled(feature: PilotFeature) {
  if (!pilotEnabled()) return false;
  const configured = process.env.NEUROFILAMENT_PILOT_FEATURES;
  return configured === undefined || configured.split(",").map(value => value.trim()).includes(feature);
}

export function requirePilot(feature?: PilotFeature) {
  if (!(feature ? pilotFeatureEnabled(feature) : pilotEnabled())) throw new Error("Candidate memory features are disabled on this server.");
}
