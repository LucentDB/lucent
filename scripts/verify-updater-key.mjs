#!/usr/bin/env node
// Verifies that a signature produced by the release signing key is accepted by
// the public key the app actually ships.
//
//   npx tauri signer sign -f ~/.lucent-updater.key -p "$PW" /tmp/payload.bin
//   node scripts/verify-updater-key.mjs --payload /tmp/payload.bin \
//        --sig /tmp/payload.bin.sig
//
// Why this exists: the public key is committed in tauri.conf.json, but the
// private key lives only in CI. If they are not the same pair, every release
// still publishes cleanly — the artifacts, latest.json and the GitHub Release
// all look right — and then every installed app rejects the update. Nothing
// fails until users are stuck, so the check has to happen before the first tag,
// not after.
//
// The release workflow runs this before building, with the real key from its
// secrets, so a mismatched pair fails the job instead of shipping.
import { readFileSync } from 'node:fs';
import { createHash, createPublicKey, verify as cryptoVerify } from 'node:crypto';

const args = process.argv.slice(2);
function arg(name, { required = true } = {}) {
  const i = args.indexOf(`--${name}`);
  const value = i === -1 ? undefined : args[i + 1];
  if (required && !value) {
    console.error(`usage: node scripts/verify-updater-key.mjs --payload <file> --sig <file> [--pubkey <base64>]`);
    process.exit(2);
  }
  return value;
}

const payloadPath = arg('payload');
const sigPath = arg('sig');
// Default to the key the app ships, so the common case cannot be run with the
// wrong one by accident.
const pubkeyB64 =
  arg('pubkey', { required: false }) ??
  JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).plugins.updater.pubkey;

// A minisign public key: an "untrusted comment" line, then base64 of
// alg[2] || key_id[8] || key[32].
function parsePublicKey(b64) {
  const text = Buffer.from(b64, 'base64').toString('utf8');
  const blobLine = text.split('\n').filter((l) => l.trim())[1];
  if (!blobLine) throw new Error('public key: no key material line');
  const blob = Buffer.from(blobLine, 'base64');
  if (blob.length !== 42) throw new Error(`public key: expected 42 bytes, got ${blob.length}`);
  const [algA, algB] = [String.fromCharCode(blob[0]), String.fromCharCode(blob[1])];
  if (algA !== 'E' || algB !== 'd') throw new Error(`public key: unexpected algorithm ${algA}${algB}`);
  return { keyId: blob.subarray(2, 10), raw: blob.subarray(10, 42) };
}

// A Tauri .sig file holds base64 of a minisign signature file:
//   untrusted comment
//   base64(alg[2] || key_id[8] || signature[64])
//   trusted comment
//   base64(global signature)
function parseSignature(b64) {
  const text = Buffer.from(b64.trim(), 'base64').toString('utf8');
  const lines = text.split('\n').filter((l) => l.trim());
  const blob = Buffer.from(lines[1], 'base64');
  if (blob.length !== 74) throw new Error(`signature: expected 74 bytes, got ${blob.length}`);
  return {
    alg: String.fromCharCode(blob[0]) + String.fromCharCode(blob[1]),
    keyId: blob.subarray(2, 10),
    signature: blob.subarray(10, 74),
  };
}

// Ed25519 in an SPKI DER envelope: fixed 12-byte prefix + the raw 32-byte key.
function publicKeyObject(raw) {
  const der = Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), raw]);
  return createPublicKey({ key: der, format: 'der', type: 'spki' });
}

const pub = parsePublicKey(pubkeyB64);
const sig = parseSignature(readFileSync(sigPath, 'utf8'));
const payload = readFileSync(payloadPath);

const keyIdMatches = pub.keyId.equals(sig.keyId);
console.log(`public key id : ${pub.keyId.toString('hex')}`);
console.log(`signed with   : ${sig.keyId.toString('hex')} (alg ${sig.alg})`);

if (!keyIdMatches) {
  console.error('\nMISMATCH: the signature was made by a different key than the one the app ships.');
  console.error('Updates produced with this private key would be rejected by every install.');
  process.exit(1);
}

// `ED` is minisign's prehashed variant: Ed25519 over a BLAKE2b-512 digest of
// the message. `Ed` is the legacy form over the message itself.
const message = sig.alg === 'ED' ? createHash('blake2b512').update(payload).digest() : payload;
const ok = cryptoVerify(null, message, publicKeyObject(pub.raw), sig.signature);

if (!ok) {
  console.error(`\nMISMATCH: signature does not verify against the shipped public key (alg ${sig.alg}).`);
  process.exit(1);
}

console.log('\nOK: the shipped public key verifies signatures from this private key.');
