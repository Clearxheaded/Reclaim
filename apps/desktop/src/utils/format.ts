// Formatting and Display Utilities

export function formatBytes(bytes?: number | null): string {
  if (bytes === undefined || bytes === null || isNaN(bytes) || bytes <= 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1);
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

export function getFileExtension(path: string): string {
  const basename = getBasename(path);
  const dotIndex = basename.lastIndexOf('.');
  if (dotIndex === -1 || dotIndex === 0) return '';
  return basename.substring(dotIndex + 1).toLowerCase();
}

export function formatNumber(num?: number | null): string {
  if (num === undefined || num === null || isNaN(num)) return '0';
  return new Intl.NumberFormat().format(num);
}

export function compareTimestamps(timeA?: number, timeB?: number): string {
  if (timeA === undefined || timeB === undefined) return 'Unknown timestamp';
  const diffSec = timeA - timeB;
  if (Math.abs(diffSec) < 2) return 'Identical modification timestamp';
  if (diffSec > 0) {
    if (diffSec < 60) return `${diffSec}s newer`;
    if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m newer`;
    if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h newer`;
    return `${Math.floor(diffSec / 86400)}d newer`;
  } else {
    const absDiff = Math.abs(diffSec);
    if (absDiff < 60) return `${absDiff}s older`;
    if (absDiff < 3600) return `${Math.floor(absDiff / 60)}m older`;
    if (absDiff < 86400) return `${Math.floor(absDiff / 3600)}h older`;
    return `${Math.floor(absDiff / 86400)}d older`;
  }
}
