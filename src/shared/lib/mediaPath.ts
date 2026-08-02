export function normalizeMediaPath(path: string): string {
  return path
    .trim()
    .replace(/\//g, "\\")
    .replace(/\\+$/, "")
    .toLocaleLowerCase("en-US");
}
