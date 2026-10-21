import { expect, test, type Page } from "@playwright/test";

async function expectReadableHeroGraph(page: Page) {
  const graph = page.getByRole("figure", { name: "Fictional pricing study graph" });
  await expect(graph.locator('[data-hero-node="experiment"]')).toHaveCount(2);
  await expect(graph.locator('[data-hero-node="finding"]')).toHaveCount(2);
  await expect(graph.locator('[data-hero-node="decision"][data-hero-status="candidate"]')).toHaveCount(1);
  await expect(graph.getByText("Documented finding", { exact: true })).toHaveCount(2);
  await expect(graph.getByText("Awaiting human ratification", { exact: true })).toBeVisible();
  await expect(graph.locator('[aria-label="The experiment documents its findings"]')).toHaveCount(2);
  const layout = await graph.evaluate(element => {
    const frame = element.getBoundingClientRect();
    const nodes = Array.from(element.querySelectorAll<HTMLElement>("[data-hero-node]"));
    const bounds = nodes.map(node => ({ title: node.querySelector("h3")?.textContent, box: node.getBoundingClientRect() }));
    const clipped = bounds.filter(({ box }) => box.left < frame.left - 1 || box.right > frame.right + 1 || box.top < frame.top - 1 || box.bottom > frame.bottom + 1).map(({ title }) => title);
    const overlaps = bounds.flatMap((a, index) => bounds.slice(index + 1).filter(b => Math.min(a.box.right, b.box.right) - Math.max(a.box.left, b.box.left) > 1 && Math.min(a.box.bottom, b.box.bottom) - Math.max(a.box.top, b.box.top) > 1).map(b => [a.title, b.title]));
    const overflowingText = nodes.flatMap(node => Array.from(node.querySelectorAll<HTMLElement>("h3, p, footer")).filter(text => text.scrollWidth > text.clientWidth + 1).map(text => text.textContent));
    return { clipped, overlaps, overflowingText };
  });
  expect(layout).toEqual({ clipped: [], overlaps: [], overflowingText: [] });
}

async function expectReadablePricingWalkthrough(page: Page) {
  const section = page.locator(".public-pricing-section");
  await expect(section.locator(".public-pricing-disclosure")).toContainText("All experiments and findings in this example are fictional");
  const studies = section.locator(".public-pricing-study");
  await expect(studies).toHaveCount(2);
  for (const study of await studies.all()) {
    await expect(study.locator("dt")).toHaveText(["Question", "Experiment", "Documented finding", "Still open"]);
  }
  await expect(studies.filter({ hasText: "Explore per-seat pricing" }).locator(".public-pricing-observation")).toContainText("Predictable bills, fewer invitations.");
  await expect(studies.filter({ hasText: "Explore usage-based pricing" }).locator(".public-pricing-observation")).toContainText("Flexible usage, uncertain bills.");
  await expect(section.locator(".public-pricing-synthesis")).toContainText("predictable spending matters, and so does the freedom to invite collaborators");
  const proposal = section.locator(".public-pricing-proposal");
  await expect(proposal.getByRole("heading", { name: "Test a team plan with included usage", exact: true })).toBeVisible();
  await expect(proposal.locator(".public-pricing-badge")).toHaveText("Awaiting human ratification");
  await expect(proposal.getByRole("heading", { name: "Why this is worth testing", exact: true })).toBeVisible();
  await expect(proposal.getByRole("heading", { name: "What the next experiment checks", exact: true })).toBeVisible();
  await expect(proposal.getByRole("listitem")).toHaveCount(3);
  await expect(proposal.locator(".public-pricing-review-note")).toContainText("The final price remains an open question.");
  const layout = await section.evaluate(element => {
    const frame = element.getBoundingClientRect();
    const blocks = Array.from(element.querySelectorAll<HTMLElement>(".public-pricing-study, .public-pricing-synthesis, .public-pricing-proposal"));
    const bounds = blocks.map(block => ({ title: block.querySelector("h3")?.textContent, box: block.getBoundingClientRect() }));
    const clipped = bounds.filter(({ box }) => box.left < frame.left - 1 || box.right > frame.right + 1 || box.top < frame.top - 1 || box.bottom > frame.bottom + 1).map(({ title }) => title);
    const overlaps = bounds.flatMap((a, index) => bounds.slice(index + 1).filter(b => Math.min(a.box.right, b.box.right) - Math.max(a.box.left, b.box.left) > 1 && Math.min(a.box.bottom, b.box.bottom) - Math.max(a.box.top, b.box.top) > 1).map(b => [a.title, b.title]));
    const overflowingText = Array.from(element.querySelectorAll<HTMLElement>("h2, h3, h4, p, dt, dd, li")).filter(text => text.scrollWidth > text.clientWidth + 1 || text.scrollHeight > text.clientHeight + 1).map(text => text.textContent);
    return { clipped, overlaps, overflowingText };
  });
  expect(layout).toEqual({ clipped: [], overlaps: [], overflowingText: [] });
}

test("public landing and demo are anonymous, isolated and shareable", async ({ page, context }, testInfo) => {
  const privateRequests: string[] = [];
  const runtimeErrors: string[] = [];
  page.on("pageerror", error => runtimeErrors.push(error.message));
  page.on("console", message => { if (message.type() === "error") runtimeErrors.push(message.text()); });
  page.on("request", request => { if (/\/api\/(graph|search|review|projects|nodes|export|mcp|session|reasoning|health)(\/|\?|$)/.test(request.url())) privateRequests.push(request.url()); });
  await page.setViewportSize({ width: 1440, height: 1000 });
  const response = await page.goto("/");
  expect(response?.headers()["x-robots-tag"]).toBe("index, follow");
  await expect(page.getByRole("heading", { name: "Test. Iterate. Connect the dots.", exact: true, level: 1 })).toBeVisible();
  await expect(page.locator(".public-pricing-intro").getByRole("heading", { name: "One pricing question. Three possible paths.", exact: true, level: 2 })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Public navigation" }).getByRole("link", { name: "Explore demo", exact: true })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Public navigation" }).getByRole("link", { name: /^Open the workspace/ })).toBeVisible();
  await expect(page.locator('meta[name="robots"]')).toHaveAttribute("content", "index, follow");
  await expectReadableHeroGraph(page);
  await expectReadablePricingWalkthrough(page);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("landing-desktop.png"), fullPage: true, caret: "initial" });
  await page.getByRole("figure", { name: "Fictional pricing study graph" }).screenshot({ path: testInfo.outputPath("hero-desktop.png"), caret: "initial" });
  await page.locator(".public-pricing-section").screenshot({ path: testInfo.outputPath("pricing-desktop.png"), caret: "initial" });
  await page.getByRole("link", { name: "Explore the demo", exact: true }).click();
  await expect(page.getByRole("heading", { name: "See the connections.", exact: true, level: 1 })).toBeVisible();
  await expect(page.locator(".public-demo-notice")).toContainText(/fictional/i);
  await expect(page.locator(".public-demo-card")).toHaveCount(4);
  await expect(page.locator(".react-flow__edge")).toHaveCount(3);
  const details = page.getByRole("region", { name: "Selected record details" });
  await expect(details.getByRole("heading", { name: "Test a team plan with included usage", exact: true })).toBeVisible();
  await expect(details.locator(".public-demo-status")).toHaveText("Candidate");
  await expect(details).toContainText("candidate for human review and further testing");
  await expect(page.locator(".public-demo-status").filter({ hasText: /^Ratified$/ })).toHaveCount(0);
  await page.getByLabel("Record type").selectOption("Attempt");
  await expect(page.locator(".public-demo-card")).toHaveCount(2);
  await page.locator(".public-demo-card").filter({ hasText: "Explore per-seat pricing" }).click();
  await expect(details.locator(".public-demo-status")).toHaveText("Observed");
  await expect(details).toContainText("spending easier to forecast");
  await expect(details).toContainText("occasional collaborator creates an added charge");
  await page.locator(".public-demo-card").filter({ hasText: "Explore usage-based pricing" }).click();
  await expect(details.locator(".public-demo-status")).toHaveText("Observed");
  await expect(details).toContainText("without requiring a paid seat for every collaborator");
  await expect(details).toContainText("bill uncertain");
  await page.getByRole("button", { name: "Clear filters", exact: true }).click();
  await page.locator(".public-demo-card").filter({ hasText: "Connect the pricing findings" }).click();
  await expect(details.getByRole("heading", { name: "Connect the pricing findings", exact: true })).toBeVisible();
  const connectedProposal = details.getByRole("button", { name: "Test a team plan with included usage" });
  await connectedProposal.focus();
  await expect(connectedProposal).toBeFocused();
  await connectedProposal.press("Enter");
  await expect(details.getByRole("heading", { name: "Test a team plan with included usage", exact: true })).toBeVisible();
  await expect(details.locator(".public-demo-status")).toHaveText("Candidate");
  await page.getByLabel("Search the sample").fill("no-match-here");
  await expect(page.getByRole("heading", { name: "No matching records" })).toBeVisible();
  await page.getByRole("button", { name: "Clear filters", exact: true }).click();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("demo-desktop.png"), fullPage: true, caret: "initial" });
  expect(privateRequests).toEqual([]);
  expect((await context.cookies()).some(cookie => cookie.name === "nf_project")).toBe(false);
  const html = await page.content();
  expect(html).not.toMatch(/source_threads|D-c8a549c0|D-hosted-architecture|\/Users\/jeffreymiao/);
  expect(runtimeErrors).toEqual([]);
});

test("the public sample has a usable mobile list and visible record details", async ({ page }, testInfo) => {
  const runtimeErrors: string[] = [];
  page.on("pageerror", error => runtimeErrors.push(error.message));
  page.on("console", message => { if (message.type() === "error") runtimeErrors.push(message.text()); });
  for (const viewport of [
    { width: 720, height: 1000, screenshot: "720" },
    { width: 390, height: 844, screenshot: "mobile" },
  ]) {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Test. Iterate. Connect the dots.", exact: true, level: 1 })).toBeVisible();
    await expect(page.locator(".public-pricing-intro").getByRole("heading", { name: "One pricing question. Three possible paths.", exact: true, level: 2 })).toBeVisible();
    await expect(page.getByRole("navigation", { name: "Public navigation" }).getByRole("link", { name: "Explore demo", exact: true })).toBeVisible();
    await expect(page.getByRole("navigation", { name: "Public navigation" }).getByRole("link", { name: /^Open the workspace/ })).toBeVisible();
    await expectReadableHeroGraph(page);
    await expectReadablePricingWalkthrough(page);
    await page.screenshot({ path: testInfo.outputPath(`landing-${viewport.screenshot}.png`), fullPage: true, caret: "initial" });
    await page.getByRole("figure", { name: "Fictional pricing study graph" }).screenshot({ path: testInfo.outputPath(`hero-${viewport.screenshot}.png`), caret: "initial" });
    await page.locator(".public-pricing-section").screenshot({ path: testInfo.outputPath(`pricing-${viewport.screenshot}.png`), caret: "initial" });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await page.goto("/demo");
    await expect(page.getByRole("heading", { name: "See the connections.", exact: true, level: 1 })).toBeVisible();
    await expect(page.locator(".public-demo-notice")).toContainText(/fictional/i);
    await expect(page.locator(".public-demo-canvas")).toBeHidden();
    await expect(page.locator(".public-demo-list-item")).toHaveCount(4);
    await page.locator(".public-demo-list-item").filter({ hasText: "Explore per-seat pricing" }).click();
    const details = page.getByRole("region", { name: "Selected record details" });
    await expect(details.locator(".public-demo-status")).toHaveText("Observed");
    await expect(details.locator(".public-demo-source")).toContainText(/fictional/i);
    await expect(details.getByRole("heading", { name: "Explore per-seat pricing", exact: true })).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({ path: testInfo.outputPath(`demo-${viewport.screenshot}.png`), fullPage: true, caret: "initial" });
  }
  expect(runtimeErrors).toEqual([]);
});
