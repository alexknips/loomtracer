import { createHash } from 'node:crypto';
export const hash = value => createHash('sha256').update(typeof value === 'string' || Buffer.isBuffer(value) ? value : JSON.stringify(value)).digest('hex');
export const same = (a,b) => JSON.stringify(a) === JSON.stringify(b);
export const LIMITS = Object.freeze({ brokerSteps:16, invocationSeconds:120, minDispatchIntervalMs:60000, historicalPerInvocation:16384, initialHistorical:12288, historicalPerTask:131072 });
export const VERSIONS = Object.freeze({ protocol:'functionality-v3-continuity-1.0.0', report:'functionality-v3-continuity-report-1.0.0', rubric:'functionality-v3-strict-1.0.0' });
export const ARMS = Object.freeze(['stock-recent-history','stock-maintained-notes','loomtracer-previous','loomtracer-candidate']);
