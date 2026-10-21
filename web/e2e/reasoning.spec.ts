import { test, expect, type Page } from '@playwright/test';
import { makeDemoBundle } from '../components/reasoning/fixture';

const imported = () => { const bundle = makeDemoBundle(); return { ...bundle, schemaVersion: '0.2.0', namespace: undefined, authorityGrants: undefined, commitmentReceipts: undefined, externalEntryRefs: undefined }; };
async function importFile(page: Page) {
  await page.getByTestId('bundle-input').setInputFiles({ name: 'private-review.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(imported())) });
}

test('private imports remain local, retain exact passages, and export review notes', async ({ page }) => {
  const writes: string[] = [];
  page.on('request', req => { if (req.method() !== 'GET' && req.url().includes('/api/')) writes.push(req.url()); });
  await page.goto('/inspect'); await importFile(page);
  await expect(page.getByRole('heading', { name: 'Synthetic walkthrough', exact: true })).toBeVisible();
  await page.getByTestId('entry-card-E-recommendation:v1').click();
  await expect(page.getByTestId('source-highlight').first()).toContainText('I recommend option A');
  await page.getByRole('button', { name: '+ Add note' }).click();
  await page.getByRole('textbox', { name: 'Reviewer name' }).fill('Alex');
  await page.getByTestId('review-comment').fill('Confirm the original workload assumption.');
  await page.getByTestId('review-save').click();
  await expect(page.getByTestId('review-unsaved')).toBeVisible();
  const download = page.waitForEvent('download'); await page.getByTestId('export-bundle').click();
  expect((await download).suggestedFilename()).toBe('synthetic-walkthrough.json');
  expect(writes).toEqual([]);
});

test('graph selection and cutoff preserve position and viewport with minimap and curves', async ({ page }) => {
  await page.goto('/inspect'); await importFile(page); await page.getByTestId('graph-tab').click();
  const node = page.getByTestId('graph-node-E-question:v1'); await expect(node).toBeVisible();
  const viewport = page.locator('.reasoning-surface .react-flow__viewport');
  await expect(viewport).toHaveAttribute('style', /scale\(0\.85\)/);
  const before = await viewport.getAttribute('style');
  await node.click(); await expect(node).toHaveAttribute('aria-pressed', 'true');
  expect(await viewport.getAttribute('style')).toBe(before);
  const nodeBefore = await page.locator('[data-id="E-question:v1"]').getAttribute('style');
  await page.getByTestId('cutoff-slider').fill('3');
  expect(await page.locator('[data-id="E-question:v1"]').getAttribute('style')).toBe(nodeBefore);
  expect(await viewport.getAttribute('style')).toBe(before);
  await expect(page.locator('.react-flow__minimap')).toBeVisible();
  expect(await page.locator('.react-flow__edge-path').first().getAttribute('d')).toContain('C');
  await page.screenshot({ path: test.info().outputPath('reasoning-inspector.png'), fullPage: true });
});

test('legacy database sources are labeled as preserved records with unknown original authorship', async ({ page }) => {
  const bundle = makeDemoBundle(); bundle.title = 'Legacy source fixture';
  bundle.contexts = bundle.contexts.map(context => ({ ...context, kind: 'legacy_record' }));
  bundle.entries = bundle.entries.map(entry => entry.id === 'E-recommendation:v1' ? { ...entry, provenance: { ...entry.provenance, expressedBy: null } } : entry);
  bundle.sources = bundle.sources.map(source => ({ ...source, role: 'record', provenance: { kind: 'legacy_record', originalRecord: { table: 'nodes', id: source.id, workspaceId: 'legacy-workspace' } } }));
  await page.goto('/inspect');
  await page.getByTestId('bundle-input').setInputFiles({ name: 'legacy.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(bundle)) });
  await expect(page.locator('.dataset-subtitle')).toContainText('4 source records');
  await page.getByTestId('entry-card-E-recommendation:v1').click();
  const inspector = page.getByTestId('inspector');
  await expect(inspector.getByText('Preserved source', { exact: true })).toBeVisible();
  await expect(inspector.getByText('Directly expressed', { exact: true })).toHaveCount(0);
  await expect(page.getByTestId('reported-provenance')).toContainText('Expressed byNot established');
  await inspector.getByRole('button', { name: 'Record 2' }).click();
  await expect(inspector).toContainText('Preserved source record');
  await expect(inspector.getByRole('heading', { name: 'Record 2', exact: true })).toBeVisible();
});
