import { expect, test, type Page } from "@playwright/test";
import { makeDemoBundle } from "../components/reasoning/fixture";

async function hostedFixture(page: Page) {
  const bundle = makeDemoBundle(); const reads: string[] = [];
  await page.route('**/api/reasoning**', async route => {
    const url = new URL(route.request().url());
    if (!url.searchParams.has('project_ref')) return route.fulfill({ json: { projects: [{ ref: 'project:one', name: 'One' }, { ref: 'project:two', name: 'Two' }], actor: { id: 'human:fixture-owner', name: 'Fixture owner', kind: 'human' }, isOwner: true } });
    const ref = url.searchParams.get('project_ref'); reads.push(ref!);
    const scoped = { ...bundle, title: ref === 'project:one' ? 'Project one history' : 'Project two history' };
    if (url.searchParams.has('dataset_id')) return route.fulfill({ json: { bundle: scoped, head: 'head-0', commitments: [], authorityGrants: [], captureMetadata: {} } });
    return route.fulfill({ json: { datasets: [{ id: bundle.id, title: scoped.title, head: 'head-0' }], grants: [], commitments: [], connections: [] } });
  });
  return { bundle, reads };
}

test("hosted reasoning exposes contributions, grouped connections, filters, and original source history", async ({ page }) => {
  const { bundle } = await hostedFixture(page); await page.goto("/graph?project=project:one");
  await expect(page.getByRole('heading', { name: 'Project one history' })).toBeVisible();
  await expect(page.locator('.masthead')).toHaveCount(0);
  await expect(page.getByTestId('bundle-input')).toHaveCount(0);
  const download = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Export history', exact: true }).click();
  expect((await download).suggestedFilename()).toBe('synthetic-walkthrough.json');
  await expect(page.getByTestId('timeline-entry')).toHaveCount(bundle.entries.length);
  await page.getByTestId('role-filter').selectOption('observation'); await expect(page.getByTestId('timeline-entry')).toHaveCount(3);
  await page.getByTestId('role-filter').selectOption(''); await page.getByTestId('entry-card-E-recommendation:v1').click();
  await expect(page.getByTestId('source-highlight')).toContainText('I recommend option A');
  await page.locator('[title="Inspect C-support:v1"]').first().click();
  await expect(page.getByTestId('inspector').locator('.participants button')).toHaveCount(4);
  await expect(page.getByTestId('inspector')).toContainText('Example recorder');
  await page.getByTestId('run-select').selectOption('R1'); await expect(page.getByTestId('timeline-entry')).toHaveCount(8);
  await expect(page.getByTestId('entry-card-E-new-test:v1')).toHaveCount(0);
});

test("legacy review bookmark opens append-only review without adopting the contribution", async ({ page }) => {
  await hostedFixture(page); const writes: string[] = []; page.on('request', req => { if (req.method() === 'POST') writes.push(req.url()); });
  await page.goto('/review'); await expect(page).toHaveURL(/\/graph$/);
  await page.getByTestId('entry-card-E-recommendation:v1').click(); await page.getByRole('button', { name: '+ Add note' }).click();
  await page.getByTestId('review-comment').fill('Check the workload coverage; this is not adoption.'); await page.getByTestId('review-save').click();
  await expect(page.getByTestId('review-unsaved')).toBeVisible(); await expect(page.getByTestId('inspector')).toContainText('Check the workload coverage');
  await expect(page.getByRole('button', { name: 'Ratify', exact: true })).toHaveCount(0);
  await page.getByText('Commitments and authority').click(); await expect(page.getByText('No current commitment is established for this history.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Record adopt', exact: true })).toBeDisabled(); expect(writes.filter(url => url.includes('/api/reasoning'))).toEqual([]);
});

test("agent setup replaces manual token creation and preserves private legacy export", async ({ page }, testInfo) => {
  await page.goto("/settings/tokens"); await expect(page.getByRole("heading", { name: "Agent connections", exact: true })).toBeVisible();
  await expect(page.locator(".setup-code").first()).toContainText("codex mcp login loomtracer"); await expect(page.getByText("No agents connected yet.")).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("agent-connections.png"), fullPage: true });
  expect((await page.request.post("/api/tokens", {data:{name:"retired"}})).status()).toBe(410);
  const exported = await page.request.get("/api/export?format=jsonl"); expect(exported.status()).toBe(200); expect(exported.headers()["cache-control"]).toContain("no-store");
});

test("private MCP route authenticates before discovery and rejects streaming methods", async ({ request }) => {
  const unauthorized = await request.post("/api/mcp", { data: { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "test", version: "1" } } } });
  expect(unauthorized.status()).toBe(401); expect(unauthorized.headers()['www-authenticate']).toContain('/.well-known/oauth-protected-resource/api/mcp');
  expect((await request.get("/api/mcp")).status()).toBe(405);
});

test("fresh stateless MCP discovery exposes native reasoning and rejects retired tools and incomplete writes", async ({ request }) => {
  const headers = { Authorization: 'Bearer demo-oauth-agent', Accept: "application/json, text/event-stream", "MCP-Protocol-Version": "2025-06-18" };
  const call = (id: number, name: string, args: Record<string, unknown>) => request.post("/api/mcp", { headers, data: { jsonrpc: "2.0", id, method: "tools/call", params: { name, arguments: args } } });
  const initialized = await request.post("/api/mcp", { headers, data: { jsonrpc: "2.0", id: 10, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "e2e", version: "1" } } } });
  expect(initialized.status()).toBe(200); expect(initialized.headers()["cache-control"]).toContain("no-store");
  const listed = await request.post("/api/mcp", { headers, data: { jsonrpc: "2.0", id: 11, method: "tools/list", params: {} } }); const tools = (await listed.json()).result.tools;
  expect(tools.map((tool: { name: string }) => tool.name).sort()).toEqual(['capture_reasoning', 'change_commitment', 'expand_reasoning', 'get_reasoning_brief', 'get_reconstruction_packet', 'list_projects', 'list_reasoning_datasets', 'revise_extraction', 'search_reasoning'].sort());
  expect(tools.find((tool: { name: string }) => tool.name === "get_reasoning_brief").annotations.readOnlyHint).toBe(true);
  expect(tools.find((tool: { name: string }) => tool.name === "change_commitment").annotations.readOnlyHint).toBe(false);
  const notification = await request.post("/api/mcp", { headers, data: { jsonrpc: "2.0", method: "notifications/initialized" } }); expect(notification.status()).toBe(202);
  const rejected: Array<[string, Record<string, unknown>]> = [
    ...['get_task_brief', 'get_relevant_context', 'expand_node', 'record_attempt', 'propose_decision', 'propose_assumption', 'check_plan_against_decisions', 'propose_superseding_decision'].map(name => [name, { project_ref: 'project:neurofilament' }] as [string, Record<string, unknown>]),
    ['get_reasoning_brief', { project_ref: 'unscoped', task: 'Review' }], ['capture_reasoning', { project_ref: 'project:neurofilament', bundle: makeDemoBundle() }], ['change_commitment', { project_ref: 'project:neurofilament', action: 'adopt' }],
  ];
  for (const [index, [name, args]] of rejected.entries()) { const response = await call(20 + index, name, args); expect(response.status(), name).toBe(200); expect((await response.json()).result.isError, name).toBe(true); }
});

test("project selection opens only the explicitly scoped saved history", async ({ page }) => {
  const { reads, bundle } = await hostedFixture(page); await page.goto('/graph?project=project:two');
  await expect(page.getByRole('combobox', { name: 'Reasoning project' })).toHaveValue('project:two'); await expect(page.getByRole('heading', { name: 'Project two history' })).toBeVisible();
  expect(reads.every(ref => ref === 'project:two')).toBe(true); await page.getByRole('combobox', { name: 'Reasoning project' }).selectOption('project:one');
  await page.getByRole('combobox', { name: 'Saved reasoning history' }).selectOption(bundle.id); await expect(page.getByRole('heading', { name: 'Project one history' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Project two history' })).toHaveCount(0); expect(reads.at(-1)).toBe('project:one');
});

test("project management previews a move and never offers a descendant as parent", async ({ page }) => {
  await page.goto("/projects");
  await page.getByRole("button", { name: "Edit Echeasea", exact: true }).click();
  await expect(page.getByLabel("Parent project").locator("option", { hasText: "Hush" })).toHaveCount(0);
  await page.getByRole("button", { name: "Edit Hush", exact: true }).click();
  await page.getByLabel("Parent project").selectOption("10000000-0000-4000-8000-000000000002");
  await page.getByRole("button", { name: "Preview changes" }).click();
  await expect(page.locator(".move-preview")).toContainText("1 lost");
  await expect(page.locator(".move-preview")).toContainText("Preserve player choice");
});

test("invitation handoff keeps its secret out of navigation URLs", async ({ page }) => {
  const invitation = "11111111-1111-4111-8111-111111111111." + "a".repeat(64);
  await page.route("**/auth/v1/authorize?**", route => route.fulfill({contentType:"text/html",body:"Google authorization handoff fixture"}));
  await page.goto(`/join#${invitation}`);
  await expect(page.getByRole("heading",{name:"Welcome to Loomtracer."})).toBeVisible();
  const prepared = page.waitForResponse("**/api/invitations/prepare");
  await page.getByRole("button",{name:"Accept invitation with Google"}).click();
  expect((await prepared).status()).toBe(200);
  await expect(page).toHaveURL(/auth\/v1\/authorize/);
  expect(page.url()).not.toContain("a".repeat(64));
  const cookies = await page.context().cookies("http://127.0.0.1:3107");
  expect(cookies.find(c=>c.name==="nf_invitation")?.httpOnly).toBe(true);
});

test("beta administration displays closed admission and MCP discovery has no private data", async ({page,request}, testInfo) => {
  await page.goto("/settings/invitations");
  await expect(page.getByRole("heading",{name:"Invitations",exact:true}).first()).toBeVisible();
  await expect(page.getByRole("status")).toContainText("Tester admission is closed");
  await page.screenshot({ path: testInfo.outputPath("invitations.png"), fullPage: true });
  const response = await request.get("/.well-known/oauth-protected-resource/api/mcp");
  expect(response.status()).toBe(200);
  const metadata=await response.json();
  expect(metadata.resource).toMatch(/\/api\/mcp$/);
  expect(metadata.authorization_servers).toHaveLength(1);
  expect(JSON.stringify(metadata)).not.toMatch(/workspace_id|user_id|OWNER_EMAIL|SECRET_KEY/);
});
