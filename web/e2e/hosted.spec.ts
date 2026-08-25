import { expect, test } from "@playwright/test";

test("renders the bounded decision graph with filters and detail history", async ({ page }) => {
  await page.goto("/graph");
  await expect(page.getByRole("heading", { name: "Decision flow" })).toBeVisible();
  await expect(page.locator(".graph-card")).toHaveCount(4);
  await page.getByRole("combobox", { name: "Node type" }).selectOption("Evidence");
  await expect(page.locator(".graph-card")).toHaveCount(1);
  await page.getByRole("combobox", { name: "Node type" }).selectOption("all");
  await page.getByText("Use a hosted decision graph", { exact: true }).first().click();
  await expect(page.getByRole("heading", { name: "History" })).toBeVisible();
});

test("review queue enforces reconsider conditions and rejection reasons", async ({ page }) => {
  await page.goto("/review");
  await page.getByRole("button", { name: "Ratify" }).click();
  await page.getByLabel(/Reconsider conditions/).fill("");
  await page.getByRole("button", { name: "Confirm" }).click();
  await expect(page.locator(".form-error")).toContainText("needs at least one reconsider condition");
  await page.getByLabel(/Reconsider conditions/).fill("A redaction policy is validated");
  await page.getByRole("button", { name: "Confirm" }).click();
  await expect(page.getByText("Queue clear")).toBeVisible();
  await page.reload();
  await page.getByRole("button", { name: "Reject" }).click();
  await page.getByRole("button", { name: "Confirm" }).click();
  await expect(page.locator(".form-error")).toContainText("rejection reason is required");
  await page.getByLabel("Why reject this entry? (required)").fill("The retention model is unresolved");
  await page.getByRole("button", { name: "Confirm" }).click();
  await expect(page.getByText("Queue clear")).toBeVisible();
});

test("token creation reveals the secret once and requires copy acknowledgement", async ({ page }) => {
  await page.goto("/settings/tokens");
  await page.getByLabel("Name / agent identity").fill("Playwright canary");
  await page.getByRole("button", { name: "Create token" }).click();
  await expect(page.getByRole("heading", { name: "Save this token now" })).toBeVisible();
  const close = page.getByRole("button", { name: "Close" });
  await expect(close).toBeDisabled();
  await page.getByRole("button", { name: "Copy token" }).click();
  await page.getByLabel("I have stored this token securely.").check();
  await expect(close).toBeEnabled();
  await close.click();
  await expect(page.getByRole("heading", { name: "Save this token now" })).toBeHidden();
  const exported = await page.request.get("/api/export?format=jsonl");
  expect(exported.status()).toBe(200);
  expect(exported.headers()["cache-control"]).toContain("no-store");
  expect((await exported.text()).split("\n")[0]).toContain('"record_type":"node"');
});

test("private MCP route authenticates before discovery and rejects streaming methods", async ({ request }) => {
  const unauthorized = await request.post("/api/mcp", { data: { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "test", version: "1" } } } });
  expect(unauthorized.status()).toBe(401);
  const unsupported = await request.get("/api/mcp");
  expect(unsupported.status()).toBe(405);
});

test("stateless MCP preserves all seven local tool interfaces", async ({ request }) => {
  const token = `nf_pat_00000000-0000-4000-8000-000000000002.${"A".repeat(43)}`;
  const headers = { Authorization: `Bearer ${token}`, Accept: "application/json, text/event-stream", "MCP-Protocol-Version": "2025-06-18" };
  const call = (id: number, name: string, args: Record<string, unknown>) => request.post("/api/mcp", {
    headers,
    data: { jsonrpc: "2.0", id, method: "tools/call", params: { name, arguments: args } },
  });
  const initialized = await request.post("/api/mcp", { headers, data: { jsonrpc: "2.0", id: 10, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "e2e", version: "1" } } } });
  expect(initialized.status()).toBe(200);
  expect(initialized.headers()["cache-control"]).toContain("no-store");
  const listed = await request.post("/api/mcp", { headers, data: { jsonrpc: "2.0", id: 11, method: "tools/list", params: {} } });
  expect((await listed.json()).result.tools).toHaveLength(7);
  const notification = await request.post("/api/mcp", { headers, data: { jsonrpc: "2.0", method: "notifications/initialized" } });
  expect(notification.status()).toBe(202);

  const cases: Array<[string, Record<string, unknown>]> = [
    ["get_relevant_context", { task: "hosted supabase", files: [], technologies: ["Supabase"] }],
    ["expand_node", { id: "D-hosted" }],
    ["record_attempt", { title: "MCP attempt", actor: "impersonated-owner", outcome: "Worked" }],
    ["propose_decision", { title: "MCP candidate", reconsider_conditions: ["Evidence changes"], actor: "impersonated-owner" }],
    ["propose_assumption", { title: "MCP assumption" }],
    ["check_plan_against_decisions", { task: "Choose persistence", plan: "Use bidirectional sync" }],
    ["propose_superseding_decision", { title: "MCP replacement", supersedes_id: "D-hosted", reason: "New evidence", reconsider_conditions: ["Evidence reverses"] }],
  ];
  for (const [index, [name, args]] of cases.entries()) {
    const response = await call(20 + index, name, args);
    expect(response.status(), name).toBe(200);
    const result = (await response.json()).result;
    expect(result.isError ?? false, name).toBe(false);
    expect(result.structuredContent, name).toBeTruthy();
    if (name === "propose_decision") expect(result.structuredContent.node.created_by).toBe("Demo agent");
  }
});
