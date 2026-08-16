import { describe, it, expect } from 'vitest';
import { connectionSubtitle, connectionEndpoint } from './connection-format';

const pg = {
  driver: 'postgres',
  params: {
    host: '127.0.0.1',
    port: '5432',
    user: 'postgres',
    database: 'postgres',
  },
};
const pgDefaults = { driver: 'postgres', params: {} };
const duck = {
  driver: 'duckdb',
  params: { path: '/tmp/analytics.duckdb', read_only: 'false' },
};
const duckMemory = { driver: 'duckdb', params: { path: ':memory:' } };
const duckNoPath = { driver: 'duckdb', params: {} };

describe('connectionSubtitle', () => {
  it('formats postgres as user@host:port/database', () => {
    expect(connectionSubtitle(pg)).toBe('postgres@127.0.0.1:5432/postgres');
  });

  it('falls back to postgres defaults when params are absent', () => {
    expect(connectionSubtitle(pgDefaults)).toBe('postgres@:5432/');
  });

  it('shows the database path for duckdb', () => {
    expect(connectionSubtitle(duck)).toBe('/tmp/analytics.duckdb');
    expect(connectionSubtitle(duckMemory)).toBe(':memory:');
  });

  it('says in-memory when a duckdb profile has no path', () => {
    expect(connectionSubtitle(duckNoPath)).toBe('In-memory database');
  });
});

describe('connectionEndpoint', () => {
  it('formats postgres as host:port', () => {
    expect(connectionEndpoint(pg)).toBe('127.0.0.1:5432');
  });

  it('shows the path for duckdb', () => {
    expect(connectionEndpoint(duck)).toBe('/tmp/analytics.duckdb');
    expect(connectionEndpoint(duckNoPath)).toBe('In-memory');
  });
});
