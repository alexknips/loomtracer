import { createHash } from 'node:crypto';

export const VERSIONS = Object.freeze({ corpus: 'functionality-v5-corpus-1.0.2',
  protocol: 'functionality-v5-isolated-workflows-1.1.1', rubric: 'functionality-v5-strict-units-1.0.1',
  report: 'functionality-v5-report-1.0.2' });
export const LEVELS = Object.freeze(['easy', 'intermediate', 'demanding']);
export const IDENTITIES = Object.freeze(['same', 'distinct', 'unresolved']);
export const FAMILIES = Object.freeze(['incident', 'experiment', 'release']);
export const LIMITS = Object.freeze({ brokerSteps: 16, invocationSeconds: 120, minDispatchIntervalMs: 60000,
  historicalPerTask: 32768, historicalPerInvocation: 32768, initialHistorical: 8192, lookupBytes: 8192,
  memoryBytes: 65536, memoryFiles: 24, episodeBytes: 65536, deliveryUnit: 'UTF-8 bytes', planReserve: 4 });
export const APPROVED_SCREEN_BUDGET = 4250000;
export const hash = value => createHash('sha256').update(typeof value === 'string' || Buffer.isBuffer(value) ? value : JSON.stringify(value)).digest('hex');
export const check = (condition, message) => { if (!condition) throw new Error(message); };
export const clone = structuredClone;
