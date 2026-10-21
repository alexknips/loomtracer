import { z } from "zod/v4";

export const relationshipType = z.enum(["supports", "contradicts", "depends_on", "based_on_assumption"]);
export const operationId = z.string().min(1).max(200);
export const captureRelationship = z.object({
  target_id: z.string().min(1).max(200),
  edge_type: relationshipType,
  direction: z.enum(["outgoing", "incoming"]),
  rationale: z.string().trim().min(1).max(4000),
  source_ref: z.string().trim().min(1).max(2000),
}).strict();
export type CaptureRelationship = z.infer<typeof captureRelationship>;

export const relationshipProposal = z.object({
  project_ref: z.string().regex(/^project:[a-z0-9][a-z0-9._/-]*$/),
  source_id: z.string().min(1).max(200),
  target_id: z.string().min(1).max(200),
  edge_type: relationshipType,
  rationale: z.string().trim().min(1).max(4000),
  source_ref: z.string().trim().min(1).max(2000),
  operation_id: operationId.optional(),
}).strict().refine(value => value.source_id !== value.target_id, "Self relationships are not supported.");
export type RelationshipProposal = z.infer<typeof relationshipProposal>;
