/**
 * Narrow production intake check. Dry-run by default. No auth, invitation,
 * membership, graph, or credential mutation. Uses one synthetic request address.
 * Run preflight, confirm its operator SQL returns 0, then run submit with the
 * SAME --email. Credentials come only from an explicitly supplied public key.
 */
import { randomUUID } from 'node:crypto';

const app = 'https://neurofilament-public.vercel.app';
const database = 'https://auvuznfkxaqcfsnfhymn.supabase.co';
const purpose = 'Synthetic production intake smoke test; no access or contact requested.';
const args = process.argv.slice(2);
const execute = args.includes('--execute');
function argument(name) {
  const index = args.indexOf(name);
  return index < 0 ? undefined : args[index + 1];
}
const phase = argument('--phase') ?? 'preflight';
const suppliedEmail = argument('--email');
const email = suppliedEmail ?? `release-smoke-${randomUUID()}@example.test`;
if (!['preflight', 'submit'].includes(phase)) throw new Error('Phase must be preflight or submit.');
if (!/^release-smoke-[a-f0-9-]{36}@example\.test$/.test(email)) throw new Error('Use the generated synthetic release-smoke address.');
if (execute && !suppliedEmail) throw new Error('Execution requires an explicit --email reused across both phases.');
const sql = phase === 'preflight'
  ? `select count(*) as matching_rows from private.access_requests where email = '${email}'; -- Expected: 0`
  : `select count(*) as matching_rows, bool_and(status = 'pending') as all_pending, bool_and(use_case = '${purpose}') as original_purpose_preserved from private.access_requests where email = '${email}'; -- Expected: 1, true, true`;

if (!execute) {
  console.log(JSON.stringify({ mode: 'dry-run; no requests sent', app, database, phase, email,
    instruction: 'Supply NF_PUBLIC_SUPABASE_KEY, add --execute, and reuse --email across phases. Verify operator SQL after preflight before submitting.',
    expected: phase === 'preflight'
      ? ['Invalid web payloads rejected', 'Wrong origin rejected', 'Web and direct-RPC honeypots return generic success without storage', 'Direct-RPC invalid purpose rejected', 'Anonymous table and inbox reads denied']
      : ['Valid web submission returns 202', 'Normalized web duplicate returns 202', 'Direct-RPC duplicate returns 200', 'Exactly one pending row; original purpose retained'],
    operator_sql: sql }, null, 2));
  process.exit(0);
}

const key = process.env.NF_PUBLIC_SUPABASE_KEY?.trim();
let legacyAnon = false;
if (key?.startsWith('eyJ')) {
  try { legacyAnon = JSON.parse(Buffer.from(key.split('.')[1], 'base64url').toString()).role === 'anon'; } catch { /* fail closed below */ }
}
if (!key || (!key.startsWith('sb_publishable_') && !legacyAnon)) {
  throw new Error('NF_PUBLIC_SUPABASE_KEY must be a public publishable key or legacy anon key. Secret and service-role keys are refused.');
}
const dbHeaders = { apikey: key, ...(legacyAnon ? { Authorization: `Bearer ${key}` } : {}), 'Content-Type': 'application/json' };
const webHeaders = { Origin: app, 'Content-Type': 'application/json' };
const payload = { email, use_case: purpose, website: '' };
const rpcPayload = { request_email: email, request_use_case: purpose, honeypot: '' };
const checks = [];
async function check(label, url, init, expectedStatuses, predicate) {
  // Never follow redirects with key headers or print response bodies/credentials.
  const response = await fetch(url, { ...init, redirect: 'manual', signal: AbortSignal.timeout(15000) });
  let body = null;
  try { body = await response.json(); } catch { /* non-JSON failure is rejected when a predicate is supplied */ }
  const passed = expectedStatuses.includes(response.status) && (!predicate || predicate(body));
  checks.push({ label, status: response.status, passed });
  console.log(JSON.stringify(checks.at(-1)));
  if (!passed) throw new Error(`Verification failed: ${label}; status ${response.status}. Response body omitted.`);
}
const web = (label, body, status, headers = webHeaders, predicate) => check(label, `${app}/api/access-requests`,
  { method: 'POST', headers, body: typeof body === 'string' ? body : JSON.stringify(body) }, [status], predicate);
const rpc = (label, body, status, predicate) => check(label, `${database}/rest/v1/rpc/submit_access_request`,
  { method: 'POST', headers: dbHeaders, body: JSON.stringify(body) }, [status], predicate);
const ok = body => body?.ok === true && Object.keys(body).length === 1;
const error = body => body && !Array.isArray(body) && typeof body.code === 'string' && typeof body.message === 'string';
try {
  if (phase === 'preflight') {
    await web('wrong origin denied', payload, 403, { ...webHeaders, Origin: 'https://example.invalid' });
    await web('non-JSON content type denied', payload, 415, { ...webHeaders, 'Content-Type': 'text/plain' });
    await web('malformed JSON denied', '{', 400);
    await web('short purpose denied', { ...payload, use_case: 'short' }, 400);
    await web('long purpose denied', { ...payload, use_case: 'x'.repeat(1001) }, 400);
    await web('oversized body denied', { ...payload, use_case: 'x'.repeat(4100) }, 413);
    await web('web honeypot generic success', { ...payload, website: 'synthetic-bot-trap' }, 202, webHeaders, ok);
    await rpc('direct-RPC honeypot generic success', { ...rpcPayload, honeypot: 'synthetic-bot-trap' }, 200, ok);
    await rpc('direct-RPC validation enforced', { ...rpcPayload, request_use_case: 'short' }, 400,
      body => body?.code === 'P0001' && body?.message === 'INVALID_REQUEST');
    await check('private request table not exposed through public schema', `${database}/rest/v1/access_requests?select=id&limit=1`,
      { headers: dbHeaders }, [401, 403, 404], error);
    await check('private schema request read denied', `${database}/rest/v1/access_requests?select=id&limit=1`,
      { headers: { ...dbHeaders, 'Accept-Profile': 'private' } }, [401, 403, 404, 406], error);
    await check('anonymous inbox RPC unavailable or denied', `${database}/rest/v1/rpc/list_access_requests`,
      { method: 'POST', headers: dbHeaders, body: '{}' }, [401, 403, 404], error);
  } else {
    await web('valid web submission accepted', payload, 202, webHeaders, ok);
    await web('normalized web duplicate accepted', { ...payload, email: ` ${email.toUpperCase()} `, use_case: 'Synthetic duplicate must not replace the original request.' }, 202, webHeaders, ok);
    await rpc('normalized direct-RPC duplicate accepted', { ...rpcPayload, request_email: ` ${email.toUpperCase()} `, request_use_case: 'Synthetic direct duplicate must not replace the original request.' }, 200, ok);
  }
  console.log(JSON.stringify({ result: 'HTTP checks passed; operator SQL readback still required', phase, email,
    checks: checks.length, operator_sql: sql }, null, 2));
} catch (failure) {
  console.error(JSON.stringify({ result: 'failed', phase, email, checks: checks.length,
    error: failure instanceof Error ? failure.message : 'Request failed', operator_sql: sql }, null, 2));
  process.exitCode = 1;
}
