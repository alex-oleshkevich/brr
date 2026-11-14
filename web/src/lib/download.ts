export function saveBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  link.click();
  // Revoking at once can cancel a large download before the browser has read the blob.
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
