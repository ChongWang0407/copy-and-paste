/**
 * Stores a value in a cache kept to `limit` entries: the least recently set
 * ones go first. Caches keyed by file path or entry ID otherwise grow with
 * every file and entry ever shown.
 */
export const setBounded = <K, V>(cache: Map<K, V>, key: K, value: V, limit: number) => {
  cache.delete(key);
  cache.set(key, value);
  for (const oldest of cache.keys()) {
    if (cache.size <= limit) break;
    cache.delete(oldest);
  }
};
