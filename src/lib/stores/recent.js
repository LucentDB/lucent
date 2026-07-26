const STORAGE_KEY = 'lucent-recent';

export function getRecentConnections() {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) || '[]');
  } catch {
    return [];
  }
}

export function addRecentConnection(config) {
  const recent = getRecentConnections().filter(
    (c) =>
      !(
        c.host === config.host &&
        c.port === config.port &&
        c.database === config.database
      ),
  );
  recent.unshift({
    host: config.host,
    port: config.port,
    user: config.user,
    database: config.database,
    lastConnected: Date.now(),
  });
  localStorage.setItem(STORAGE_KEY, JSON.stringify(recent.slice(0, 5)));
}
