import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { isAbsolute, join } from 'node:path';
import { StringDecoder } from 'node:string_decoder';

// Subscription execution is for trusted synthetic fixtures, not hostile inputs.
// No model call occurs unless invokeSubscription receives execute: true.
export const PIN = Object.freeze({ cli: '0.163.0-alpha.10.2', model: 'gpt-6.1-sol', reasoning: 'medium', processing: 'standard', auth: 'chatgpt' });
export const DEFAULT_CODEX = '/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex';

const disabled = ['shell_tool', 'unified_exec', 'shell_snapshot', 'shell_snapshot_v2', 'code_mode', 'code_mode_host',
  'apps', 'plugins', 'hooks', 'browser_use', 'browser_use_external', 'computer_use', 'in_app_browser',
  'image_generation', 'view_image', 'memories', 'external_agent_memory_import', 'multi_agent', 'multi_agent_v2',
  'agent_message_board', 'fast_mode', 'ultrafast_mode', 'skill_search', 'skill_mcp_dependency_install',
  'sleep_tool', 'goals', 'artifact', 'workspace_dependencies', 'tool_suggest', 'recommended_plugins',
  'remote_plugin', 'in_app_local_automation', 'realtime_conversation', 'daemon_auto_start',
  'context_management', 'stable_environment_tools', 'default_mode_request_user_input', 'current_time_reminder',
  'unbounded_connection_retries'];

export const CONFIG = Object.freeze([
  'model_provider="openai"', `model_reasoning_effort="${PIN.reasoning}"`, 'forced_login_method="chatgpt"',
  'approval_policy="never"', 'web_search="disabled"', 'project_doc_max_bytes=0',
  'include_environment_context=false', 'mcp_servers={}', 'analytics.enabled=false',
  ...disabled.map(feature => `features.${feature}=false`),
  'features.skip_host_skill_discovery=true',
  'developer_instructions="This is a trusted synthetic memory-chain experiment. Perform the requested capture, reconstruction, or isolation check using only the user-provided data and instructions. Do not call tools, access files, execute code, browse, delegate, or consult other conversations or memories. Return only the requested output."',
]);

const hash = value => createHash('sha256').update(value).digest('hex');
const validCount = value => Number.isSafeInteger(value) && value >= 0;

// Keep saved subscription authentication available without reading its contents.
// API credentials, proxy routes, provider overrides and app injection are excluded.
export function subscriptionEnvironment(environment = process.env) {
  const env = {};
  for (const key of ['HOME', 'PATH', 'USER', 'LOGNAME', 'TMPDIR', 'LANG', 'LC_ALL', 'CODEX_HOME']) {
    if (environment[key] !== undefined) env[key] = environment[key];
  }
  env.NO_COLOR = '1';
  return env;
}

export function subscriptionCommand({ cwd, executable = DEFAULT_CODEX, environment = process.env }) {
  if (!isAbsolute(cwd ?? '') || !isAbsolute(executable)) throw new Error('Absolute empty workspace and executable paths required');
  const args = ['exec', '--ignore-user-config', '--ignore-rules', '--strict-config', '--ephemeral', '--json',
    '--color', 'never', '--skip-git-repo-check', '--sandbox', 'read-only', '-C', cwd, '-m', PIN.model,
    ...CONFIG.flatMap(value => ['-c', value]), '-'];
  return { executable, args, env: subscriptionEnvironment(environment), shell: false,
    command_fingerprint: hash(JSON.stringify({ executable,
      args: args.map(value => value === cwd ? '<empty-temporary-workspace>' : value), pin: PIN })) };
}

export function normalizeSessionUsage(raw) {
  if (!raw || typeof raw !== 'object') return null;
  const input = raw.input_tokens, output = raw.output_tokens;
  if (!validCount(input) || !validCount(output) || !validCount(input + output)) return null;
  const cached = raw.cached_input_tokens ?? raw.input_tokens_details?.cached_tokens ?? null;
  const reasoning = raw.reasoning_output_tokens ?? raw.output_tokens_details?.reasoning_tokens ?? null;
  if (cached !== null && (!validCount(cached) || cached > input)) return null;
  if (reasoning !== null && (!validCount(reasoning) || reasoning > output)) return null;
  return { input_tokens: input, output_tokens: output, total_tokens: input + output,
    cached_input_tokens: cached, reasoning_output_tokens: reasoning };
}

// These exact messages are CLI configuration diagnostics, not tool invocations.
// Changed wording or additional errors must be investigated instead of suppressed.
export function knownConfigurationDiagnostic(event) {
  if (event?.type !== 'item.completed' || event.item?.type !== 'error') return false;
  const message = event.item.message;
  return typeof message === 'string' && (
    message === 'Code Mode is unavailable because code-mode host is disabled. Code mode will fail closed; enable `features.code_mode_host` and install `codex-code-mode-host`.' ||
    /^Under-development features enabled: skip_host_skill_discovery\. Under-development features are incomplete and may behave unpredictably\. To suppress this warning, set `suppress_unstable_features_warning = true` in \/[^\r\n]+\/config\.toml\.$/.test(message));
}

export function classifyEvent(event) {
  if (knownConfigurationDiagnostic(event)) return null;
  if (event?.item && !['agent_message', 'reasoning'].includes(event.item.type)) return 'unexpected-tool-or-item';
  if (['model.rerouted', 'model_reroute', 'model_verification'].includes(event?.type) && event.model !== PIN.model) {
    return 'model-reroute-or-unverified-change';
  }
  return null;
}

export function parseSubscriptionEvents(stdout) {
  const events = [], parseErrors = [];
  for (const line of stdout.split(/\r?\n/)) {
    if (!line.trim()) continue;
    try {
      const event = JSON.parse(line);
      if (!event || typeof event !== 'object' || Array.isArray(event) || typeof event.type !== 'string') parseErrors.push(line);
      else events.push(event);
    } catch { parseErrors.push(line); }
  }
  const completions = events.filter(event => event.type === 'turn.completed');
  const usageEvents = events.filter(event => ['turn.completed', 'turn.failed'].includes(event.type) && event.usage !== undefined);
  // Usage is cumulative per turn. Never sum duplicate completions or silently
  // discard failed-turn usage. Raw events remain available when totals are unknown.
  const usage = usageEvents.length === 1 && completions.length <= 1 ? normalizeSessionUsage(usageEvents[0].usage) : null;
  const models = [...new Set(completions.flatMap(event => {
    const model = event.model ?? event.response?.model;
    return typeof model === 'string' && model ? [model] : [];
  }))];
  return { events, parse_errors: parseErrors,
    answer: events.filter(event => event.type === 'item.completed' && event.item?.type === 'agent_message').at(-1)?.item.text ?? null,
    usage, returned_model: models.length === 1 ? models[0] : null,
    failures: events.filter(event => event.type === 'error' || event.type === 'turn.failed' ||
      (event.item?.type === 'error' && !knownConfigurationDiagnostic(event))),
    diagnostics: events.filter(knownConfigurationDiagnostic),
    violations: events.flatMap(event => { const reason = classifyEvent(event); return reason ? [reason] : []; }),
    completed_turns: completions.length };
}

function runLocal(executable, args, { env, cwd, timeoutMs = 10000 }) {
  return new Promise(resolve => {
    const child = spawn(executable, args, { env, cwd, shell: false, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '', spawnError = null, timedOut = false;
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGKILL'); }, timeoutMs);
    child.stdout.on('data', bytes => { stdout += bytes; });
    child.stderr.on('data', bytes => { stderr += bytes; });
    child.once('error', error => { spawnError = error.message; });
    child.once('close', (code, signal) => { clearTimeout(timer); resolve({ code, signal, stdout, stderr, spawn_error: spawnError, timed_out: timedOut }); });
  });
}

export async function preflightSubscription({ executable = DEFAULT_CODEX, environment = process.env, cwd = tmpdir() } = {}) {
  const env = subscriptionEnvironment(environment);
  const evidence = {};
  const featureExceptions = [];
  function requireCheck(condition, message) {
    if (!condition) { const error = new Error(message); error.evidence = evidence; throw error; }
  }
  evidence.version = await runLocal(executable, ['--version'], { env, cwd });
  requireCheck(evidence.version.code === 0 && evidence.version.stdout.trim() === `codex-cli ${PIN.cli}`,
    `Pinned Codex CLI ${PIN.cli} is unavailable`);
  // This command parses overrides and lists feature state without a model request.
  // features does not support --ignore-user-config or --strict-config. Both are
  // enforced by exec; the required live canary proves strict exec acceptance.
  evidence.configuration = await runLocal(executable, [...CONFIG.flatMap(value => ['-c', value]), 'features', 'list'], { env, cwd });
  requireCheck(evidence.configuration.code === 0, 'Pinned subscription configuration was rejected');
  for (const feature of disabled) {
    if (feature === 'unified_exec' && /^unified_exec\s+.+\s+true\s*$/m.test(evidence.configuration.stdout)) {
      // This pinned CLI retains the internal executor feature despite the false
      // override. shell_tool=false removes the shell surface. Keep the fact
      // visible and invalidate any actual tool event during the canary or run.
      featureExceptions.push({ feature, observed: true, requested: false,
        limitation: 'Pinned CLI retains its internal executor; shell_tool is disabled and tool events invalidate the run.' });
      continue;
    }
    requireCheck(new RegExp(`^${feature}\\s+.+\\s+false\\s*$`, 'm').test(evidence.configuration.stdout),
      `Isolation feature ${feature} was not confirmed disabled`);
  }
  requireCheck(/^skip_host_skill_discovery\s+.+\s+true\s*$/m.test(evidence.configuration.stdout), 'Host skill discovery was not disabled');
  // login status reports the auth mechanism without exposing credentials.
  evidence.authentication = await runLocal(executable, ['login', 'status'], { env, cwd });
  const status = `${evidence.authentication.stdout}\n${evidence.authentication.stderr}`;
  requireCheck(evidence.authentication.code === 0 && /Logged in using ChatGPT/.test(status) && !/API key/i.test(status),
    'ChatGPT subscription login required; API fallback disabled');
  return { cli_version: PIN.cli, auth_mode: 'subscription', credentials_read_by_runner: false,
    configuration_verified: true, strict_exec_configuration_verified: false,
    feature_exceptions: featureExceptions, evidence };
}

export async function invokeSubscription({ prompt, execute = false, timeoutMs = 120000,
  executable = DEFAULT_CODEX, environment = process.env, cwd, onEvent } = {}) {
  if (execute !== true) throw new Error('Execution disabled; caller must explicitly opt in');
  if (typeof prompt !== 'string' || !prompt.trim() || Buffer.byteLength(prompt) > 500000) throw new Error('Bounded synthetic prompt required');
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1000 || timeoutMs > 120000) throw new Error('Timeout must be between 1 and 120 seconds');
  const created = cwd === undefined;
  const workspace = cwd ?? await mkdtemp(join(tmpdir(), 'loomtracer-v3-'));
  try {
    if ((await readdir(workspace)).length !== 0) throw new Error('Subscription workspace must be empty');
    const command = subscriptionCommand({ executable, cwd: workspace, environment });
    const started = new Date().toISOString(), startClock = performance.now();
    let preflight, result;
    try { preflight = await preflightSubscription({ executable, environment, cwd: workspace }); }
    catch (error) {
      result = { stdout: '', stderr: '', exit_code: null, signal: null,
        stop_reason: 'preflight-failed', spawn_error: null, preflight_error: error.message,
        preflight_evidence: error.evidence ?? null };
    }
    if (!result) result = await new Promise(resolve => {
      const child = spawn(executable, command.args, { cwd: workspace, env: command.env, shell: false,
        detached: true, stdio: ['pipe', 'pipe', 'pipe'] });
      const decoder = new StringDecoder('utf8');
      let stdout = '', stderr = '', pending = '', stopReason = null, spawnError = null;
      const stop = reason => {
        if (stopReason) return;
        stopReason = reason;
        try { process.kill(-child.pid, 'SIGKILL'); } catch { child.kill('SIGKILL'); }
      };
      const timer = setTimeout(() => stop('wall-time-limit'), timeoutMs);
      const inspect = line => {
        if (!line.trim()) return;
        let event;
        try { event = JSON.parse(line); } catch { stop('non-json-output'); return; }
        if (!event || typeof event !== 'object' || Array.isArray(event) || typeof event.type !== 'string') { stop('non-event-output'); return; }
        const violation = classifyEvent(event);
        if (violation) stop(violation);
        if (onEvent) { try { onEvent(event); } catch { stop('event-consumer-failed'); } }
      };
      child.stdout.on('data', bytes => {
        const text = decoder.write(bytes); stdout += text; pending += text;
        if (Buffer.byteLength(stdout) > 8 * 1024 * 1024) { stop('output-size-limit'); return; }
        let newline;
        while ((newline = pending.indexOf('\n')) !== -1) { inspect(pending.slice(0, newline)); pending = pending.slice(newline + 1); }
      });
      child.stderr.on('data', bytes => { stderr += bytes.toString('utf8'); if (Buffer.byteLength(stderr) > 1024 * 1024) stop('stderr-size-limit'); });
      child.stdin.on('error', () => {});
      child.once('error', error => { spawnError = error.message; });
      child.once('close', (code, signal) => {
        clearTimeout(timer); const tail = decoder.end(); stdout += tail; pending += tail;
        if (pending) inspect(pending);
        resolve({ stdout, stderr, exit_code: code, signal, stop_reason: stopReason, spawn_error: spawnError });
      });
      child.stdin.end(prompt);
    });
    const parsed = parseSubscriptionEvents(result.stdout);
    const valid = result.exit_code === 0 && !result.stop_reason && !result.spawn_error && !parsed.failures.length &&
      !parsed.violations.length && !parsed.parse_errors.length && parsed.completed_turns === 1 &&
      typeof parsed.answer === 'string' && parsed.answer.length > 0 &&
      (parsed.returned_model === null || parsed.returned_model === PIN.model);
    return { ...result, ...parsed, status: valid ? 'completed' : 'failed', started_at: started,
      finished_at: new Date().toISOString(), duration_ms: Math.round(performance.now() - startClock),
      requested_model: PIN.model, reasoning: PIN.reasoning, processing: PIN.processing,
      cli_version: preflight?.cli_version ?? null, preflight: preflight ?? null,
      command_fingerprint: command.command_fingerprint, prompt_sha256: hash(prompt),
      usage_scope: 'aggregate-codex-exec-turn', billing_dollars: null,
      limitations: ['Isolation is CLI-configuration and tool-event checked; the two-call canary is also required.',
        'Saved subscription authentication is shared; this is not a credential-isolated execution environment.',
        'CLI internal service retries are not fully controlled; the harness never retries an invocation.',
        'No hard provider token ceiling is available. Unknown usage must stop subsequent dispatch.',
        'Missing returned model identity makes subscription comparisons exploratory.'] };
  } finally {
    if (created) await rm(workspace, { recursive: true, force: true });
  }
}
