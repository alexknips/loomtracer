import { HiveMindError } from "./client";

export type Loaded<T> = { ok: true; value: T } | { ok: false; error: HiveMindError };

/** Runs a HiveMind read for a page; an error from the server becomes data the page can show. */
export async function load<T>(work: () => Promise<T>): Promise<Loaded<T>> {
  try {
    return { ok: true, value: await work() };
  } catch (error) {
    if (error instanceof HiveMindError) return { ok: false, error };
    throw error;
  }
}
