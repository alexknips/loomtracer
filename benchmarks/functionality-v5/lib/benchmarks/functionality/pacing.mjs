const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

export class PacingInterruptedError extends Error {
  constructor() {
    super('Dispatch pacing interrupted before invocation');
    this.name = 'PacingInterruptedError';
    this.code = 'PACING_INTERRUPTED';
  }
}

// Call once, immediately before each sequential invocation. This imposes a
// start-to-start interval; time spent executing the previous call counts toward it.
export function createDispatchPacer({ minStartIntervalMs = 0, now = Date.now, wait = sleep } = {}) {
  if (!Number.isSafeInteger(minStartIntervalMs) || minStartIntervalMs < 0) {
    throw new Error('minStartIntervalMs must be a nonnegative safe integer');
  }
  if (typeof now !== 'function' || typeof wait !== 'function') throw new TypeError('now and wait must be functions');
  const clock = () => {
    const value = now();
    if (!Number.isSafeInteger(value) || value < 0) throw new Error('Pacing clock must return nonnegative safe integer milliseconds');
    return value;
  };
  let previousStart = null;
  let waiting = false;

  return {
    async waitForStart({ shouldStop = () => false } = {}) {
      if (typeof shouldStop !== 'function') throw new TypeError('shouldStop must be a function');
      if (waiting) throw new Error('Dispatch pacing requires sequential callers');
      waiting = true;
      try {
        const checkStop = () => { if (shouldStop()) throw new PacingInterruptedError(); };
        checkStop();
        const requestedAt = clock();
        const plannedStart = previousStart === null ? requestedAt : Math.max(requestedAt, previousStart + minStartIntervalMs);
        if (!Number.isSafeInteger(plannedStart)) throw new Error('Planned pacing timestamp exceeds safe integer range');
        let actualStart = requestedAt;
        while (actualStart < plannedStart) {
          checkStop();
          // Keep signal-driven stop flags responsive even during long pacing gaps.
          await wait(Math.min(1000, plannedStart - actualStart));
          checkStop();
          actualStart = clock();
        }
        checkStop();
        const metadata = {
          min_start_interval_ms: minStartIntervalMs,
          previous_start_ms: previousStart,
          requested_at_ms: requestedAt,
          planned_start_ms: plannedStart,
          actual_start_ms: actualStart,
          planned_delay_ms: plannedStart - requestedAt,
          actual_delay_ms: Math.max(0, actualStart - requestedAt),
        };
        previousStart = actualStart;
        return metadata;
      } finally {
        waiting = false;
      }
    },
  };
}
