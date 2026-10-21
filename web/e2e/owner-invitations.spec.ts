import { expect, test } from "@playwright/test";

test("owner invitations explain separate workspaces and keep demo administration read-only", async ({ page, request }) => {
  await page.goto("/settings/invitations");
  await expect(page.getByRole("heading", { name: "Invite a team owner", exact: true })).toBeVisible();
  await expect(page.getByLabel("Workspace name", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("Google email", { exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Create invitation link", exact: true })).toBeDisabled();
  await expect(page.getByText("Accepting creates a new, empty workspace", { exact: false })).toBeVisible();
  await expect(page.getByRole("status")).toContainText("invitation and account changes are disabled");
  await expect(page.getByRole("heading", { name: "Owner invitations", exact: true })).toBeVisible();
  await expect(page.getByLabel("Current workspace: Personal")).toHaveText("Personal · Owner");
  const response = await request.post("/api/invitations", {
    headers: { origin: "http://127.0.0.1:3107" },
    data: { email: "invited@example.test", workspaceName: "Separate pilot" },
  });
  expect(response.status()).toBe(403);
  expect(await response.json()).not.toHaveProperty("link");
});

test("workspace identity and owner invitation fields fit on mobile", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/settings/invitations");
  await expect(page.getByLabel("Current workspace: Personal")).toBeVisible();
  await expect(page.getByLabel("Workspace name", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Sign out of owner@neurofilament.local" })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("owner-invitations-mobile.png"), fullPage: true });
});

test("access request administration identifies owner invitations and disables demo changes", async ({ page }) => {
  await page.goto("/settings/access-requests");
  await expect(page.getByRole("heading", { name: "Access requests", exact: true })).toBeVisible();
  await expect(page.getByText("Prepare an owner invitation with a name", { exact: false })).toBeVisible();
  await expect(page.getByRole("status")).toContainText("access-request changes are disabled");
});
