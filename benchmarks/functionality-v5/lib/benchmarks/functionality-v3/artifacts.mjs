import { mkdir, open, readFile, readdir, stat, lstat, readlink } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { hash } from './common.mjs';
export const encode = value => `${JSON.stringify(value,null,2)}\n`;
export const parse = async path => JSON.parse(await readFile(path,'utf8'));
export const exists = async path => {try {await stat(path);return true;}catch(e){if(e.code==='ENOENT')return false;throw e;}};
export async function saveImmutable(root,name,value) {
  const target=resolve(root,name);
  if(!target.startsWith(`${resolve(root)}/`))throw new Error('Artifact path escapes root');
  const bytes=typeof value==='string'||Buffer.isBuffer(value)?value:encode(value);
  await mkdir(dirname(target),{recursive:true,mode:0o700});
  if(await exists(target)){if(hash(await readFile(target))!==hash(bytes))throw new Error(`Immutable artifact changed: ${name}`);return hash(bytes);}
  const file=await open(target,'wx',0o600);
  try{await file.writeFile(bytes);await file.sync();}finally{await file.close();}
  return hash(bytes);
}
export async function loadState(root) {
  if(!await exists(join(root,'states')))return {sequence:0,head:null,value:{maintenance:{},tasks:{},groups:{},errors:[],firstGroupAssessment:null}};
  const names=(await readdir(join(root,'states'))).filter(n=>/^state-\d+\.json$/.test(n)).sort();
  let previous={sequence:0,head:null,value:{maintenance:{},tasks:{},groups:{},errors:[],firstGroupAssessment:null}};
  for(const name of names){const e=await parse(join(root,'states',name));if(e.sequence!==previous.sequence+1||e.parentHash!==previous.head||hash(e.value)!==e.valueHash)throw new Error('Checkpoint chain integrity failure');previous={sequence:e.sequence,head:hash(e),value:e.value};}
  return previous;
}
export async function saveState(root,previous,value) {
  const e={sequence:previous.sequence+1,parentHash:previous.head,valueHash:hash(value),value:structuredClone(value)};
  await saveImmutable(root,`states/state-${String(e.sequence).padStart(6,'0')}.json`,e);
  return {sequence:e.sequence,head:hash(e),value:structuredClone(value)};
}
export async function snapshotTree({source,root,target,exclude=()=>false}) {
  const files={};
  async function walk(src,name){if(exclude(name))return;const info=await lstat(src);if(info.isSymbolicLink())throw new Error(`Snapshot symlink requires explicit dependency handling: ${name}`);if(info.isDirectory()){for(const file of (await readdir(src)).sort())await walk(join(src,file),join(name,file));}else if(info.isFile()){const b=await readFile(src);await saveImmutable(root,join(target,name),b);files[join(target,name)]=hash(b);}}
  await walk(source,'');return files;
}
export async function verifyFiles(root,files){for(const [name,digest] of Object.entries(files))if(hash(await readFile(join(root,name)))!==digest)throw new Error(`Frozen source changed: ${name}`);return true;}
