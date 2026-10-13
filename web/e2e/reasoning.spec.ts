import { test, expect, type Page } from '@playwright/test';
import { makeDemoBundle } from '../components/reasoning/fixture';

const imported = () => { const bundle = makeDemoBundle(); return { ...bundle, schemaVersion: '0.2.0', namespace: undefined, authorityGrants: undefined, commitmentReceipts: undefined, externalEntryRefs: undefined }; };
async function importFile(page: Page) {
  await page.getByTestId('bundle-input').setInputFiles({ name: 'private-review.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(imported())) });
}
const actor = { id: 'human:reviewer', name: 'Reviewing owner', kind: 'human' };

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

test('saving a local file is explicit and hosted review persistence keeps authenticated attribution', async ({ page }) => {
  let saved = makeDemoBundle(); let writes = 0; const bodies: Array<Record<string, unknown>> = [];
  await page.route('**/api/reasoning**', async route => {
    const req = route.request(), url = new URL(req.url());
    if (req.method() === 'POST') { const body = req.postDataJSON(); bodies.push(body); writes++; saved = body.bundle; await route.fulfill({ json: { bundle: saved, head: `head-${writes}`, commitments: [], authorityGrants: [] } }); return; }
    if (!url.searchParams.has('project_ref')) await route.fulfill({ json: { projects: [{ ref: 'project:one', name: 'One' }], actor, isOwner: true } });
    else await route.fulfill({ json: { datasets: writes ? [{ id: saved.id, title: saved.title, head: `head-${writes}` }] : [], grants: [], commitments: [], connections: [] } });
  });
  await page.goto('/inspect'); await importFile(page); expect(writes).toBe(0);
  await page.getByRole('button', { name: 'Connect to a project to save' }).click();
  await page.getByRole('combobox', { name: 'Reasoning project' }).selectOption('project:one'); expect(writes).toBe(0);
  await page.getByTestId('save-project').click(); await expect(page.getByTestId('save-project')).toHaveText('Save changes');
  await page.getByTestId('entry-card-E-recommendation:v1').click(); await page.getByRole('button', { name: '+ Add note' }).click();
  await expect(page.getByRole('textbox', { name: 'Reviewer name' })).toHaveValue('Reviewing owner');
  await page.getByTestId('review-comment').fill('Source checked. No endorsement implied.'); await page.getByTestId('review-save').click();
  await page.getByTestId('save-project').click(); await expect(page.getByTestId('review-unsaved')).toHaveCount(0);
  expect(writes).toBe(2); expect(saved.reviewNotes.at(-1)?.reviewer).toBe(actor.id);
  expect(bodies[1].expected_head).toBe('head-1'); expect(typeof bodies[1].operation_id).toBe('string');
});

test('quick capture adds a source and a new extraction revision without changing originals', async ({ page }) => {
  let saved = makeDemoBundle(); const initialSource = JSON.stringify(saved.sources); let writes = 0;
  await page.route('**/api/reasoning**', async route => {
    const req = route.request(), url = new URL(req.url());
    if (req.method() === 'POST') { saved = req.postDataJSON().bundle; writes++; await route.fulfill({ json: { bundle: saved, head: `head-${writes}`, commitments: [], authorityGrants: [] } }); return; }
    if (!url.searchParams.has('project_ref')) await route.fulfill({ json: { projects: [{ ref: 'project:one', name: 'One' }], actor, isOwner: true } });
    else if (url.searchParams.has('dataset_id')) await route.fulfill({ json: { bundle: saved, head: 'head-0', commitments: [], authorityGrants: [] } });
    else await route.fulfill({ json: { datasets: [{ id: saved.id, title: saved.title, head: 'head-0' }], grants: [], commitments: [], connections: [] } });
  });
  await page.goto('/graph'); await expect(page.getByRole('heading', { name: 'Synthetic walkthrough', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '+ Capture contribution' }).click();
  await page.getByRole('textbox', { name: 'New contribution' }).fill('What assumptions still need to be tested?');
  await page.getByRole('combobox', { name: 'Connection target' }).selectOption('E-recommendation:v1');
  await page.getByRole('button', { name: 'Record contribution' }).click();
  await expect(page.getByTestId('timeline-entry')).toHaveCount(11);
  expect(writes).toBe(1); expect(JSON.stringify(saved.sources.slice(0, 4))).toBe(initialSource);
  expect(saved.runs).toHaveLength(3); expect(saved.sources.at(-1)?.content).toBe('What assumptions still need to be tested?');
  expect(saved.connections.at(-1)?.participants[1]).toEqual({ entryId: 'E-recommendation:v1', role: 'target' });
});

test('authority uses server receipts and explicit actions retain the original human contribution', async ({ page }) => {
  const saved = makeDemoBundle(); saved.contexts = saved.contexts.map(context => ({ ...context, kind: 'discussion' }));
  saved.commitmentReceipts = [{ id: 'untrusted-import', action: 'adopt', entryId: 'E-commitment:v1', priorReceiptId: null, actorId: 'fictional-human', grantId: null, recordedAt: '2026-10-12T18:00:00Z', rationale: 'An imported claim, not server authority.', connectionId: null, basis: 'legacy_approval', sourceSpans: saved.entries.find(entry => entry.id === 'E-commitment:v1')!.provenance.sourceSpans }];
  saved.actors.push({ id: actor.id, name: 'Forged portable owner', kind: 'human' });
  const original = JSON.stringify(saved.entries); const actions: Record<string, unknown>[] = [];
  const receipts: typeof saved.commitmentReceipts = [];
  const response = () => ({ bundle: saved, head: `head-${actions.length}`, commitments: receipts, authorityGrants: [] });
  await page.route('**/api/reasoning**', async route => {
    const req = route.request(), url = new URL(req.url());
    if (url.pathname.endsWith('/commitments')) {
      const body = req.postDataJSON(); actions.push(body);
      receipts.push({ id: `receipt-${actions.length}`, action: body.action, entryId: body.entry_id, priorReceiptId: body.prior_receipt_id, actorId: actor.id, grantId: null, recordedAt: '2026-10-12T20:00:00Z', rationale: body.rationale, connectionId: null, basis: 'authorized_action', sourceSpans: [] });
      await route.fulfill({ json: response() }); return;
    }
    if (!url.searchParams.has('project_ref')) await route.fulfill({ json: { projects: [{ ref: 'project:one', name: 'One' }], actor, isOwner: true } });
    else if (url.searchParams.has('dataset_id')) await route.fulfill({ json: response() });
    else await route.fulfill({ json: { datasets: [{ id: saved.id, title: saved.title, head: 'head-0' }], grants: [], commitments: [], connections: [] } });
  });
  await page.goto('/graph');
  await page.getByText('Commitments and authority').click();
  await expect(page.getByText('No current commitment is established for this history.')).toBeVisible();
  await page.getByTestId('entry-card-E-commitment:v1').click();
  await page.getByRole('textbox', { name: 'Commitment rationale' }).fill('This is the agreed direction.');
  await page.getByRole('button', { name: 'Record adopt', exact: true }).click();
  await expect(page.getByText('1 current commitments')).toBeVisible();
  await expect(page.locator('.commitment-card')).toContainText('By Reviewing owner');
  await expect(page.locator('.commitment-card')).not.toContainText('Forged portable owner');
  await page.getByTestId('entry-card-E-new-test:v1').click();
  await page.getByRole('combobox', { name: 'Commitment action' }).selectOption('revise');
  await page.getByRole('combobox', { name: 'Prior commitment' }).selectOption('receipt-1');
  await page.getByRole('textbox', { name: 'Commitment rationale' }).fill('The new evidence changes the direction.');
  await page.getByRole('button', { name: 'Record revise', exact: true }).click();
  await expect(page.getByText('All 2 commitment receipts')).toBeVisible();
  expect(actions[1].prior_receipt_id).toBe('receipt-1'); expect(actions[1].expected_head).toBe('head-1');
  expect(JSON.stringify(saved.entries)).toBe(original);
  await page.getByTestId('cutoff-slider').fill('3');
  await expect(page.getByText('Commitments and authority')).toHaveCount(0);
});

test('owner grants a named agent project authority and revokes future actions', async ({ page }) => {
  const grants: Record<string, unknown>[] = []; const actions: { method: string; body: Record<string, unknown> }[] = [];
  const agentId = '00000000-0000-0000-0000-000000000005';
  await page.route('**/api/reasoning**', async route => {
    const req = route.request(), url = new URL(req.url());
    if (url.pathname.endsWith('/grants')) {
      const body = req.postDataJSON(); actions.push({ method: req.method(), body });
      grants.push({ id: `grant-${actions.length}`, action: req.method() === 'POST' ? 'grant' : 'revoke', agentActorId: 'agent:five', ownerActorId: actor.id, projectId: 'project-id', recordedAt: '2026-10-12T20:00:00Z', priorGrantId: req.method() === 'DELETE' ? 'grant-1' : null });
      await route.fulfill({ json: { grant: grants.at(-1) } }); return;
    }
    if (!url.searchParams.has('project_ref')) await route.fulfill({ json: { projects: [{ ref: 'project:one', name: 'One' }], actor, isOwner: true } });
    else await route.fulfill({ json: { datasets: [], grants, commitments: [], connections: [{ id: agentId, actor_id: 'agent:five', display_name: 'Alex research agent', revoked_at: null }] } });
  });
  await page.goto('/graph'); await page.getByText('Agent commitment grants').click();
  await page.getByRole('combobox', { name: 'Agent connection for grant' }).selectOption(agentId);
  await page.getByRole('button', { name: 'Grant project authority' }).click();
  await expect(page.getByText('1 active', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Revoke future authority' }).click();
  await expect(page.getByText('0 active', { exact: true })).toBeVisible();
  expect(actions).toEqual([{ method: 'POST', body: { project_ref: 'project:one', connection_id: agentId } }, { method: 'DELETE', body: { project_ref: 'project:one', grant_id: 'grant-1' } }]);
});

test('admitted human members can capture and commit while agent grants remain owner-only', async ({ page }) => {
  const bundle = makeDemoBundle(); bundle.contexts = bundle.contexts.map(context => ({ ...context, kind: 'discussion' }));
  await page.route('**/api/reasoning**', async route => {
    const url = new URL(route.request().url());
    if (!url.searchParams.has('project_ref')) await route.fulfill({ json: { projects: [{ ref: 'project:one', name: 'One' }], actor, isOwner: false } });
    else if (url.searchParams.has('dataset_id')) await route.fulfill({ json: { bundle, head: 'head-0', commitments: [], authorityGrants: [] } });
    else await route.fulfill({ json: { datasets: [{ id: bundle.id, title: bundle.title, head: 'head-0' }], grants: [], commitments: [], connections: [] } });
  });
  await page.goto('/graph');
  await expect(page.getByTestId('save-project')).toBeEnabled();
  await expect(page.getByRole('button', { name: '+ Capture contribution' })).toBeEnabled();
  await expect(page.getByText('Agent commitment grants')).toHaveCount(0);
  await page.getByTestId('entry-card-E-commitment:v1').click();
  await page.getByText('Commitments and authority').click();
  await page.getByRole('textbox', { name: 'Commitment rationale' }).fill('Human members retain peer decision rights.');
  await expect(page.getByRole('button', { name: 'Record adopt', exact: true })).toBeEnabled();
  await expect(page.getByRole('combobox', { name: 'Commitment action' }).locator('option[value=withdraw]')).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'Commitment action' }).locator('option[value=revise]')).toHaveCount(1);
});


test('server capture is separate from reported authorship and imported recorder claims', async ({ page }) => {
  const bundle = makeDemoBundle();
  bundle.actors.push({ id: 'agent:authenticated-recorder', name: 'Forged portable identity', kind: 'agent' });
  const captured = { actorId: 'agent:authenticated-recorder', recordedAt: '2026-10-12T20:00:00Z' };
  await page.route('**/api/reasoning**', async route => {
    const url = new URL(route.request().url());
    if (!url.searchParams.has('project_ref')) await route.fulfill({ json: { projects: [{ ref: 'project:one', name: 'One' }], actor, isOwner: true } });
    else if (url.searchParams.has('dataset_id')) await route.fulfill({ json: { bundle, head: 'head-0', commitments: [], authorityGrants: [], captureMetadata: { 'E-recommendation:v1': captured, S2: captured } } });
    else await route.fulfill({ json: { datasets: [{ id: bundle.id, title: bundle.title, head: 'head-0' }], grants: [], commitments: [], connections: [] } });
  });
  await page.goto('/graph'); await page.getByTestId('entry-card-E-recommendation:v1').click();
  const reported = page.getByTestId('reported-provenance'), capture = page.getByTestId('server-capture');
  await expect(reported).toContainText('Expressed byAgent (illustrative)');
  await expect(reported).toContainText('Reported recorderExample recorder');
  await expect(capture).toContainText('Captured byagent:authenticated-recorder');
  await expect(capture).not.toContainText('Forged portable identity');
  await expect(capture.locator('time')).toHaveAttribute('datetime', captured.recordedAt);
  await page.screenshot({ path: test.info().outputPath('capture-attribution.png'), fullPage: true });
  await page.getByTestId('inspector').getByRole('button', { name: 'Message 2' }).click();
  await expect(page.getByTestId('inspector')).toContainText('SpeakerAgent (illustrative)');
  await expect(capture).toContainText('agent:authenticated-recorder');
  await page.goto('/inspect'); await importFile(page); await page.getByTestId('entry-card-E-recommendation:v1').click();
  await expect(capture).toHaveCount(0);
  await expect(page.getByTestId('imported-attribution')).toContainText('has not been verified');
  await expect(reported).toContainText('Reported recorderExample recorder');
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
