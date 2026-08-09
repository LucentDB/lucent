import { invoke as tauriInvoke } from '@tauri-apps/api/core';

function parseError(e) {
  if (typeof e === 'string') {
    try {
      return JSON.parse(e);
    } catch {
      return { kind: 'Error', message: e };
    }
  }
  if (e && typeof e.message === 'string') {
    try {
      return JSON.parse(e.message);
    } catch {
      return { kind: 'Error', message: e.message };
    }
  }
  return { kind: 'Error', message: String(e) };
}

async function invoke(cmd, args) {
  try {
    return await tauriInvoke(cmd, args);
  } catch (e) {
    throw parseError(e);
  }
}

export async function connect(config) {
  return invoke('connect', { config });
}

export async function executeQuery(
  sql,
  { limit, offset, sort = null, filters = [] },
) {
  return invoke('execute_query', { sql, limit, offset, sort, filters });
}

export function cancelQuery() {
  return invoke('cancel_query');
}

export async function getDatabases() {
  return invoke('get_databases');
}

export async function getSchemas() {
  return invoke('get_schemas');
}

export async function getSchemaObjects(schema) {
  return invoke('get_schema_objects', { schema });
}

export async function disconnect() {
  return invoke('disconnect');
}

export async function getFunctionSource(schema, name) {
  return invoke('get_function_source', { schema, name });
}

export async function getViewSource(schema, name, kind = 'view') {
  return invoke('get_view_source', { schema, name, kind });
}

export async function getSequenceInfo(schema, name) {
  return invoke('get_sequence_info', { schema, name });
}

export async function browseTable(
  schema,
  name,
  { limit, offset, sort = null, filters = [] },
) {
  return invoke('browse_table', { schema, name, limit, offset, sort, filters });
}

export async function countAllRows(sql, filters = []) {
  return invoke('count_all_rows', { sql, filters });
}

export async function describeFilters(filters = []) {
  return invoke('describe_filters', { filters });
}

// ─── Connection Profile IPC ─────────────────────────────────────────────

export async function listConnections() {
  return invoke('list_connections');
}

export async function getConnection(id) {
  return invoke('get_connection', { id });
}

export async function saveConnection(profile, password = null) {
  return invoke('save_connection', { profile, password });
}

export async function deleteConnection(id) {
  return invoke('delete_connection', { id });
}

export async function duplicateConnection(id) {
  return invoke('duplicate_connection', { id });
}

export async function testConnection(id) {
  return invoke('test_connection', { id });
}

export async function listDrivers() {
  return invoke('list_drivers');
}

export async function connectionCapabilities() {
  return invoke('connection_capabilities');
}

// ─── SSH Config IPC ───────────────────────────────────────────────────

/** @param {any} config @param {string | null} [secret] */
export async function saveSshConfig(config, secret = null) {
  return invoke('save_ssh_config', { config, secret });
}

export async function listSshConfigs() {
  return invoke('list_ssh_configs');
}

export async function deleteSshConfig(id) {
  return invoke('delete_ssh_config', { id });
}

// ─── Query History IPC ─────────────────────────────────────────────────

export async function listHistory(params = {}) {
  return invoke('list_history', params);
}

export async function toggleHistoryFavorite(id) {
  return invoke('toggle_history_favorite', { id });
}

export async function deleteHistoryEntry(id) {
  return invoke('delete_history_entry', { id });
}

export async function clearHistory() {
  return invoke('clear_history');
}

// ─── Logs Drawer IPC ──────────────────────────────────────────────────

/** @param {number} [after] index to tail from — pass the count of lines already held */
export async function getLogs(after = 0) {
  return invoke('get_logs', { after });
}
