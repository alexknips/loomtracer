import { mkdirSync } from "node:fs";
import { expect, test as base, type Page } from "@playwright/test";

// Runs against the scratch `loomtracer serve` that playwright.config.ts starts: a fictional pricing study
// (e2e/support/seed-ledger.mjs). Set LOOMTRACER_SCREENSHOTS=<dir> to also write a screenshot of each page.
const SEED_TOKEN = "e2e-scratch-token";
const screenshots = process.env.LOOMTRACER_SCREENSHOTS;
async function shoot(page: Page, name: string) {
  if (!screenshots) return;
  mkdirSync(screenshots, { recursive: true });
  await page.screenshot({ path: `${screenshots}/${name}.png`, fullPage: true, caret: "initial" });
}

// Every test in this file also checks, as it ends, that the browser talked to the UI only: no request to the
// Loomtracer server, none carrying an Authorization header, and the bearer token in no response.
const test = base.extend<{ leaks: string[] }>({
  leaks: [async ({ page }, use) => {
    const leaks: string[] = [];
    page.on("request", request => { if (request.url().includes(`:${process.env.LOOMTRACER_E2E_PORT ?? 18736}`) || request.headers().authorization) leaks.push(request.url()); });
    page.on("response", async response => { if (response.url().startsWith("http://127.0.0.1:3107") && (await response.text().catch(() => "")).includes(SEED_TOKEN)) leaks.push(`token in ${response.url()}`); });
    await use(leaks);
    expect(leaks).toEqual([]);
  }, { auto: true }],
});

test("the graph shows the ledger's decisions with their statuses, and replays them in order", async ({ page }) => {
  const runtimeErrors: string[] = [];
  page.on("pageerror", error => runtimeErrors.push(error.message));
  await page.goto("/graph?project=pricing");
  await expect(page.getByTestId("workspace-identity")).toHaveText("local · service:api");
  await expect(page.getByLabel("Reasoning project")).toHaveValue("pricing");
  await expect(page.getByLabel("Reasoning project").locator("option")).toHaveText(["claude agents' personal project (2)", "Pricing study (3)", "Pricing study / Pricing experiments (1)"]);

  const timeline = page.getByTestId("timeline");
  await expect(timeline.getByTestId("timeline-entry").filter({ hasText: "Price per seat" }).first()).toBeVisible();
  await expect(timeline).toContainText("Test a team plan with included usage");
  await expect(timeline).toContainText("Show the price before the invite is sent");
  // Status is a word on every decision, and contested is its own.
  await expect(timeline.locator(".badge.status-superseded")).toHaveCount(1);
  await expect(timeline.locator(".badge.status-accepted")).toHaveCount(1);
  await expect(timeline.locator(".badge.status-contested")).toHaveText("Status: contested");
  // What was weighed is in the record: the options of a decision are entries of their own.
  await expect(timeline).toContainText("Pure usage");
  await expect(page.getByTestId("ledger-position")).toContainText("Ledger position");
  await expect(page.getByTestId("read-from")).toContainText("Loomtracer ledger, event");
  await expect(page.getByText("read from the Loomtracer ledger", { exact: false }).first()).toBeAttached();
  await shoot(page, "graph-timeline");

  // Inspecting a record shows who recorded it, and where it sits in the ledger.
  await timeline.getByTestId("timeline-entry").filter({ hasText: "Test a team plan with included usage" }).getByRole("button").first().click();
  const inspector = page.getByTestId("inspector");
  await expect(inspector.getByTestId("reported-provenance")).toContainText("agent:claude:analyst");
  await expect(inspector.getByTestId("server-capture")).toContainText("Loomtracer ledger");
  await expect(inspector.getByTestId("source-highlight").first()).toContainText("Test a team plan with included usage");
  await expect(inspector).toContainText("Read from the Loomtracer ledger");
  await expect(inspector).toContainText("Recorded as written");
  await expect(inspector).toContainText("Loomtracer record");
  await expect(inspector).not.toContainText("Legacy");
  await expect(inspector).not.toContainText("Preserved record record");
  // Its connections sit beside it in the timeline, worded as the graph words them.
  await expect(timeline.getByTestId("timeline-entry").filter({ hasText: "Test a team plan with included usage" }).first().locator(".connection-chip").filter({ hasText: "supersedes" })).toHaveCount(1);
  await shoot(page, "graph-inspector");

  // Moving the cutoff replays the project's record in the order it was made.
  const total = await timeline.getByTestId("timeline-entry").count();
  await page.getByTestId("cutoff-slider").fill("1");
  await expect(timeline.getByTestId("timeline-entry")).toHaveCount(1);
  await expect(timeline.getByTestId("timeline-entry").first()).toContainText("Price per seat");
  await page.getByTestId("cutoff-slider").fill(await page.getByTestId("cutoff-slider").getAttribute("max") ?? "99");
  await expect(timeline.getByTestId("timeline-entry")).toHaveCount(total);

  // The trace map draws the same record, and a project's edge into another project stays a pointer.
  await page.getByTestId("graph-tab").click();
  await expect(page.locator("[data-testid^='graph-node-']").first()).toBeVisible();
  await shoot(page, "graph-trace-map");
  await page.getByLabel("Reasoning project").selectOption("pricing-experiments");
  await expect(page).toHaveURL(/\/graph\?project=pricing-experiments/);
  await expect(page.getByRole("heading", { name: "Pricing experiments", level: 1 })).toBeVisible();
  await page.getByTestId("timeline-tab").click();
  await expect(page.getByTestId("timeline")).toContainText("Run the allowance experiment on the invite flow");
  await expect(page.getByTestId("timeline").locator(".badge.status-proposed")).toHaveText("Status: proposed");
  await page.getByTestId("graph-tab").click();
  await expect(page.getByText("1 preserved cross-project links")).toBeVisible();
  await shoot(page, "graph-cross-project");
  expect(runtimeErrors).toEqual([]);
});

test("search finds decisions by their words and opens each with its reasons", async ({ page }) => {
  await page.goto("/search");
  await page.getByLabel("Search decisions").fill("seat");
  await page.getByLabel("Search decisions").press("Enter");
  await expect(page.getByTestId("search-count")).toContainText("matching “seat”");
  const results = page.getByTestId("search-result");
  await expect(results.filter({ hasText: "Price per seat" })).toBeVisible();
  await expect(results.filter({ hasText: "Test a team plan with included usage" })).toBeVisible();
  await shoot(page, "search");

  await results.filter({ hasText: "Price per seat" }).getByRole("link", { name: "Price per seat" }).click();
  await expect(page).toHaveURL(/\/decisions\/decision-/);
  const detail = page.getByTestId("decision-detail");
  await expect(detail.getByRole("heading", { name: "Price per seat", level: 1 })).toBeVisible();
  await expect(detail.locator(".hm-status")).toHaveText("Superseded");
  await expect(detail.getByTestId("decision-rationale")).toContainText("easy for teams to budget for");
  await expect(detail.getByTestId("decision-options")).toContainText("Usage based");
  await expect(detail.getByTestId("decision-options").locator(".hm-chosen")).toHaveText("Per seat");
  await expect(detail.getByTestId("decision-grounds")).toContainText("Teams prefer a bill they can predict");
  await expect(detail.getByTestId("decision-supersession")).toContainText("Superseded by Test a team plan with included usage");
  await expect(detail.getByTestId("decision-timeline")).toContainText("Recorded by human:alice");
  await shoot(page, "decision-superseded");

  // Following the replacement reaches the decision that took its place, with the evidence it was based on.
  await detail.getByRole("link", { name: "Test a team plan with included usage" }).click();
  await expect(page.getByTestId("decision-grounds")).toContainText("Seat study: occasional collaborators were invited less often");
  await expect(page.getByTestId("decision-supersession")).toContainText("Supersedes Price per seat");

  await page.goto("/search?status=contested");
  await expect(page.getByTestId("search-result")).toHaveCount(1);
  await page.getByTestId("search-result").getByRole("link").click();
  await expect(page.locator(".hm-status")).toHaveText("Contested");
  await expect(page.getByTestId("decision-contest")).toContainText("human:bob");
  await expect(page.getByTestId("decision-contest")).toContainText("human:carol");
  await expect(page.getByTestId("decision-timeline")).toContainText("Showing the price first may suppress invites");
  await shoot(page, "decision-contested");

  await page.goto("/search?q=zzzz-no-such-words");
  await expect(page.getByTestId("search-count")).toContainText("0 decisions");
  await expect(page.getByText("Nothing in the ledger matches.")).toBeVisible();
});

test("search can be asked from one project, and says whose decisions it found", async ({ page }) => {
  await page.goto("/search");
  await expect(page.getByLabel("Project").locator("option")).toHaveText(["Any project", "claude agents' personal project", "Pricing study", "Pricing study / Pricing experiments"]);
  await page.getByLabel("Project").selectOption("pricing-experiments");
  await page.getByLabel("Search decisions").fill("price");
  await page.getByLabel("Search decisions").press("Enter");
  await expect(page).toHaveURL(/project=pricing-experiments/);
  // Asked from the experiments project, the answer holds the decisions of the project it is part of, and says so.
  await expect(page.getByTestId("search-count")).toContainText("asked from Pricing experiments");
  await expect(page.getByTestId("search-scope")).toContainText("Pricing experiments (own project)");
  await expect(page.getByTestId("search-result")).toHaveCount(2);
  await expect(page.getByTestId("search-result").filter({ hasText: "Show the price before the invite is sent" })).toContainText("Pricing study; Pricing experiments is part of it");
  await expect(page.getByTestId("search-result-scope")).toHaveCount(2);
  await shoot(page, "search-project");
  // A handle nobody registered is not found, so a typo never reads as an empty project.
  await page.goto("/search?project=no-such-project");
  await expect(page.getByTestId("loomtracer-problem")).toContainText("project not registered");
});

test("a classifier draft, whose id has colons in it, opens from a search result", async ({ page }) => {
  await page.goto("/search?q=weekly+release+train");
  const result = page.getByTestId("search-result");
  await expect(result).toHaveCount(1);
  const link = result.getByRole("link", { name: "Chose a weekly release train over releasing on demand" });
  await expect(link).toHaveAttribute("href", /^\/decisions\/capture%3A\d+%3A\d+$/);
  const href = await link.getAttribute("href");
  await link.click();
  await expect(page).toHaveURL(new RegExp(`${href}$`));
  const detail = page.getByTestId("decision-detail");
  await expect(detail.getByRole("heading", { name: "Chose a weekly release train over releasing on demand", level: 1 })).toBeVisible();
  await expect(detail.getByTestId("decision-rationale")).toContainText("a late fix waits at most six days");
  await expect(detail.getByTestId("decision-options").locator(".hm-chosen")).toHaveText("Weekly release train");
  await expect(detail.getByTestId("decision-timeline")).toContainText("Recorded");
  await shoot(page, "decision-draft");
  // The id written out with its colons, as a person would paste it, reaches the same page.
  const response = await page.goto(decodeURIComponent(href!));
  expect(response?.status()).toBe(200);
  await expect(page.getByTestId("decision-detail")).toContainText("Weekly release train");
});

test("a classifier draft that chose words it never weighed names its choice", async ({ page }) => {
  await page.goto("/search?q=hotfix+branch+rebase");
  const link = page.getByTestId("search-result").getByRole("link", { name: "Rebase the hotfix branch onto main before the fast-forward merge" });
  await link.click();
  const detail = page.getByTestId("decision-detail");
  await expect(detail.getByRole("heading", { name: "Rebase the hotfix branch onto main before the fast-forward merge", level: 1 })).toBeVisible();
  // The options it weighed stay open: the choice is not one of them, so none is marked, and the choice is named apart.
  await expect(detail.getByTestId("decision-options").locator("li")).toHaveText(["Merge commit", "Squash merge"]);
  await expect(detail.getByTestId("decision-options").locator(".hm-chosen")).toHaveCount(0);
  await expect(detail.getByTestId("decision-chose")).toHaveText("Rebase in a scratch worktree");
  await shoot(page, "decision-draft-unweighed-choice");
  // A draft whose choice was one of its options marks it there and names no second choice.
  await page.goto("/search?q=weekly+release+train");
  await page.getByTestId("search-result").getByRole("link", { name: "Chose a weekly release train over releasing on demand" }).click();
  await expect(page.getByTestId("decision-detail").getByTestId("decision-options").locator(".hm-chosen")).toHaveText("Weekly release train");
  await expect(page.getByTestId("decision-chose")).toHaveCount(0);
});

test("a decision that is not in the ledger says so", async ({ page }) => {
  const response = await page.goto("/decisions/decision-does-not-exist");
  expect(response?.status()).toBe(404);
});

test("review lists what needs a person: proposed, contested and unanswered", async ({ page }) => {
  await page.goto("/review");
  await expect(page.getByTestId("review-proposed")).toContainText("Run the allowance experiment on the invite flow");
  await expect(page.getByTestId("review-contested")).toContainText("Show the price before the invite is sent");
  await expect(page.getByTestId("review-contested")).toContainText("Accepted by human:bob");
  await expect(page.getByTestId("review-contested")).toContainText("Rejected by human:carol");
  await expect(page.getByTestId("review-waiting")).toContainText("Should the team plan include a free tier?");
  await shoot(page, "review");
  await page.getByTestId("review-proposed").getByRole("link", { name: "Run the allowance experiment on the invite flow" }).click();
  await expect(page).toHaveURL(/\/decisions\/decision-/);
  await expect(page.locator(".page-heading .hm-status")).toHaveText("Proposed");
  await expect(page.getByTestId("decision-grounds")).toContainText("Invite behaviour is a fair proxy");
  // It rests on a decision of the parent project, and says so.
  await expect(page.getByTestId("decision-grounds")).toContainText("Test a team plan with included usage");
});

test("projects lists each project with its parent and its decisions by status", async ({ page }) => {
  await page.goto("/projects");
  const projects = page.getByTestId("project");
  await expect(projects).toHaveCount(3);
  // The personal project holds the classifier drafts; it comes first, then the study and the project under it.
  await expect(projects.nth(0)).toContainText("claude agents' personal project");
  await expect(projects.nth(0)).toContainText("2 decisions");
  await expect(projects.nth(1)).toContainText("Pricing study");
  await expect(projects.nth(1)).toContainText("3 decisions");
  await expect(projects.nth(2)).toHaveAttribute("data-depth", "1");
  await expect(projects.nth(2)).toContainText("part of Pricing study");
  await expect(projects.nth(2).locator(".hm-status-proposed")).toBeVisible();
  await shoot(page, "projects");
  await projects.nth(1).getByRole("link", { name: "Pricing study" }).click();
  await expect(page).toHaveURL(/\/graph\?project=pricing$/);
});

test("the API serves the project and its bundle privately, and a missing project is a 404", async ({ request }) => {
  const health = await request.get("/api/health");
  expect(health.status()).toBe(200);
  expect(await health.json()).toEqual({ ok: true, tenant: "local" });
  const projects = await request.get("/api/reasoning");
  expect(projects.headers()["cache-control"]).toContain("no-store");
  expect((await projects.json()).projects).toEqual([{ ref: "personal:agent:claude", name: "claude agents' personal project", decisions: 2 }, { ref: "pricing", name: "Pricing study", decisions: 3 }, { ref: "pricing-experiments", name: "Pricing study / Pricing experiments", decisions: 1 }]);
  const bundle = await (await request.get("/api/reasoning?project_ref=pricing")).json();
  expect(Object.keys(bundle).sort()).toEqual(["bundle", "decisions", "head"]);
  expect(bundle.decisions).toEqual({ shown: 3, total: 3, truncated: false });
  expect(bundle.bundle.namespace).toEqual({ datasetId: "loomtracer:local:pricing", workspaceId: "local", projectId: "pricing" });
  expect(JSON.stringify(bundle)).not.toContain(SEED_TOKEN);
  // A short page says so: how many decisions it draws, of how many the project holds.
  const short = await (await request.get("/api/reasoning?project_ref=pricing&limit=2")).json();
  expect(short.decisions).toEqual({ shown: 2, total: 3, truncated: true });
  expect(short.bundle.runs[0].uncertainties.join(" ")).toContain("Only the newest decisions of this project are drawn in full; it holds 3");
  expect((await request.get("/api/reasoning?project_ref=pricing&limit=0")).status()).toBe(400);
  const missing = await request.get("/api/reasoning?project_ref=nope");
  expect(missing.status()).toBe(404);
  expect(await request.post("/api/reasoning", { data: {} }).then(response => response.status())).toBe(405);
});
