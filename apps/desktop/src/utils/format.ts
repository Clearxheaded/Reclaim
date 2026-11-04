// Formatting and Display Utilities

export function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  const val = bytes / Math.pow(k, i);
  return `${val.toFixed(i === 0 ? 0 : 2)} ${sizes[i]}`;
}

export function formatDate(dateStrOrTimestamp: string | number): string {
  try {
    let date: Date;
    if (typeof dateStrOrTimestamp === 'number') {
      date = new Date(dateStrOrTimestamp * 1000);
    } else {
      date = new Date(dateStrOrTimestamp);
    }
    return date.toLocaleString(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  } catch {
    return String(dateStrOrTimestamp);
  }
}

export function truncateHash(hash: string, len: number = 8): string {
  if (!hash || hash.length <= len * 2) return hash;
  return `${hash.substring(0, len)}...${hash.substring(hash.length - len)}`;
}

export function getBasename(path: string): string {
  const normalized = path.replace(/\\/g, '/');
  const parts = normalized.split('/');
  return parts[parts.length - 1] || path;
}

export function getParentDir(path: string): string {
  const normalized = path.replace(/\\/g, '/');
  const parts = normalized.split('/');
  if (parts.length <= 1) return '';
  parts.pop();
  return parts.join('/');
}

