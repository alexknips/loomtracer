import { readFile, readdir, lstat, readlink } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
export { saveImmutable, parse, exists, snapshotTree, verifyFiles, loadState, saveState } from '../functionality-v3/artifacts.mjs';
export const sha256 = value => createHash('sha256').update(typeof value === 'string' || Buffer.isBuffer(value) ? value : JSON.stringify(value)).digest('hex');

/** Verify the original inventory without treating newly created v5 files as mutations. */
export async function verifyPreservation(repo, inventory) {
  let files = 0, links = 0, bytes = 0;
  for (const [name, expected] of Object.entries(inventory)) {
    const path = resolve(repo, name);
    if (!path.startsWith(`${resolve(repo)}/`)) throw new Error('Inventory path outside repository');
    const info = await lstat(path);
    if (expected.symlink !== undefined) {
      if (!info.isSymbolicLink() || await readlink(path) !== expected.symlink) throw new Error(`Prior symlink changed: ${name}`);
      links++;
    } else {
      if (!info.isFile() || sha256(await readFile(path)) !== expected.sha256) throw new Error(`Prior artifact changed: ${name}`);
      files++; bytes += expected.bytes;
    }
  }
  return { verified: true, files, links, bytes, inventoryHash: sha256(inventory) };
}

export async function listFiles(root, prefix = '') {
  const result = [];
  for (const entry of (await readdir(join(root, prefix), { withFileTypes: true })).sort((a,b) => a.name.localeCompare(b.name))) {
    const name = join(prefix, entry.name);
    if (entry.isDirectory()) result.push(...await listFiles(root, name));
    else if (entry.isFile()) result.push(name);
    else throw new Error(`Unsupported snapshot entry: ${name}`);
  }
  return result;
}
