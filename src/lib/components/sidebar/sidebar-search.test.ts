import { test, expect } from 'vitest';
import { objectMatches, schemaMatches, dbMatches } from './sidebar-search';

test('objectMatches is case-insensitive and empty-query matches all', () => {
  expect(objectMatches('Users', '')).toBe(true);
  expect(objectMatches('Users', 'user')).toBe(true);
  expect(objectMatches('orders', 'user')).toBe(false);
});

test('schemaMatches by schema name or contained object', () => {
  const objects = [{ name: 'users' }, { name: 'orders' }];
  expect(schemaMatches({ name: 'public' }, objects, 'user')).toBe(true); // object match
  expect(schemaMatches({ name: 'auth' }, objects, 'auth')).toBe(true); // name match
  expect(schemaMatches({ name: 'public' }, objects, 'zzz')).toBe(false); // no match
  expect(schemaMatches({ name: 'public' }, undefined, 'user')).toBe(false); // not loaded
});

test('dbMatches: db retained when a contained table matches, not just db name', () => {
  const schemas = [{ name: 'public' }];
  const objectsBySchema = { public: [{ name: 'users' }] };
  // Searching a TABLE name keeps the database visible (the reported bug).
  expect(dbMatches('appdb', schemas, objectsBySchema, 'user')).toBe(true);
  // Searching the db name still works.
  expect(dbMatches('appdb', schemas, objectsBySchema, 'appdb')).toBe(true);
  // No match anywhere hides the db.
  expect(dbMatches('appdb', schemas, objectsBySchema, 'zzz')).toBe(false);
  // Empty query always matches.
  expect(dbMatches('appdb', schemas, objectsBySchema, '')).toBe(true);
});
