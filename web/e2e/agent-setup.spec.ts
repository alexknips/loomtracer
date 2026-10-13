import { expect, test } from "@playwright/test";

test("selected project and client produce the corresponding persistent setup", async ({ page }) => {
  await page.goto("/settings/connections");
  await page.getByLabel("Project for this repository").selectOption("project:neurofilament");
  await page.getByLabel("Agent client").selectOption("claude");
  await expect(page.locator(".setup-code").first()).toContainText("claude mcp add --transport http loomtracer-local --scope user");
  await expect(page.getByText("Authorization covers every project in this workspace.", { exact: false })).toBeVisible();
  await page.getByRole("button", { name: "Copy CLAUDE.md import" }).click();
  await expect(page.getByRole("status")).toHaveText("CLAUDE.md import copied.");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("@AGENTS.md");
  await page.getByRole("button", { name: "Copy AGENTS.md block" }).click();
  const instructions = await page.evaluate(() => navigator.clipboard.readText());
  expect(instructions).toContain('project_ref: "project:neurofilament"');
  expect(instructions).toContain("previous_fingerprint");
  expect(instructions).toContain("get_reasoning_brief");
  expect(instructions).toContain("capture_reasoning");
  expect(instructions).toContain("revise_extraction");
  expect(instructions).toContain("Standing capture authorization is unratified.");
  expect(instructions).toContain("active human-owner grant to this named agent for this exact project");
  expect(instructions).not.toMatch(/get_task_brief|check_plan_against_decisions/);
  await page.getByLabel("Agent client").selectOption("codex");
  await expect(page.locator(".setup-code").first()).toContainText("codex mcp login loomtracer-local --scopes openid");
  await expect(page.getByRole("button", { name: "Copy CLAUDE.md import" })).toHaveCount(0);
  await page.getByRole("button", { name: "Copy verification prompt" }).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toContain("Do not create records");
  await expect(page.getByText("No agents connected yet.")).toBeVisible();
});

test("agent setup fits a narrow viewport", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/settings/connections");
  await expect(page.getByRole("heading", { name: "Agent connections", exact: true })).toBeVisible();
  await page.getByLabel("Agent client").selectOption("claude");
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("agent-setup-mobile.png"), fullPage: true });
});
