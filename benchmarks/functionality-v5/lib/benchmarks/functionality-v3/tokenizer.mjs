import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { Tiktoken } from './vendor/node_modules/js-tiktoken/dist/lite.js';
import o200k from './vendor/node_modules/js-tiktoken/dist/ranks/o200k_base.js';

const encoding = new Tiktoken(o200k);
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
const packageJson = JSON.parse(readFileSync(new URL('./vendor/node_modules/js-tiktoken/package.json', import.meta.url), 'utf8'));
if (packageJson.version !== '1.0.21') throw new Error(`Unfrozen tokenizer package ${packageJson.version}`);

/** Corpus sizing only. This is not CLI-reported usage or verified gpt-6 tokenization. */
export function countTokens(text) {
  if (typeof text !== 'string') throw new TypeError('countTokens requires a string');
  return encoding.encode(text, [], []).length;
}

export function encodeTokens(text) { return encoding.encode(text, [], []); }
export function decodeTokens(tokens) { return encoding.decode(tokens); }

export function tokenizerManifest() {
  const implementationFiles = ['js-tiktoken/dist/lite.js', 'js-tiktoken/dist/chunk-VL2OQCWN.js', 'base64-js/index.js'];
  const implementationFilesSha256 = Object.fromEntries(implementationFiles.map(path => [path, digest(readFileSync(new URL('./vendor/node_modules/' + path, import.meta.url)))]));
  return {
    id: 'js-tiktoken@1.0.21/o200k_base',
    package: 'js-tiktoken', version: packageJson.version, encoding: 'o200k_base',
    ranksSha256: digest(readFileSync(new URL('./vendor/node_modules/js-tiktoken/dist/ranks/o200k_base.js', import.meta.url))),
    implementationSha256: digest(JSON.stringify(implementationFilesSha256)),
    implementationFilesSha256,
    lockSha256: digest(readFileSync(new URL('./vendor/package-lock.json', import.meta.url))),
    mappingStatus: 'experimental-sizing-proxy',
    providerExact: false,
    requestedModelMapping: null,
    note: 'Named BPE sizing proxy. No verified mapping to the requested gpt-6.1-sol model. Usage accounting must use CLI-reported tokens.',
  };
}

/** Stable public source representation, including provenance and chronological metadata. */
export function serializeSourceMessage(message) {
  return JSON.stringify({
    id: message.id,
    actorId: message.actorId,
    role: message.role,
    order: message.order,
    conversationId: message.conversationId,
    occurredAt: message.occurredAt,
    content: message.content,
  });
}

export function serializeSourceMessages(messages) {
  return messages.map(serializeSourceMessage).join('\n');
}

export function countSourceTokens(messages) { return countTokens(serializeSourceMessages(messages)); }
