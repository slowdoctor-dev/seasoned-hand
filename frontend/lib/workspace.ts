const BASE_URL =
  typeof window === "undefined"
    ? ""
    : process.env.NEXT_PUBLIC_API_URL ?? `http://${window.location.hostname}:3000`;

export type WorkspaceEntry = {
  name: string;
  type: "file" | "dir";
  size?: number;
};

export type DirListing = { type: "dir"; entries: WorkspaceEntry[] };

export async function listDir(
  sessionId: string,
  path: string,
): Promise<DirListing> {
  const tail = path === "" ? "" : encodeURI(path).replace(/^\/+/, "");
  const url = `${BASE_URL}/v1/workspace/${encodeURIComponent(sessionId)}/${tail}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`listDir ${res.status}`);
  return (await res.json()) as DirListing;
}

export async function readFile(
  sessionId: string,
  path: string,
): Promise<string> {
  const tail = encodeURI(path).replace(/^\/+/, "");
  const url = `${BASE_URL}/v1/workspace/${encodeURIComponent(sessionId)}/${tail}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`readFile ${res.status}`);
  return await res.text();
}

const EXT_TO_LANG: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  json: "json",
  md: "markdown",
  py: "python",
  rs: "rust",
  go: "go",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  sql: "sql",
  sh: "shell",
  html: "html",
  css: "css",
};

export function languageForPath(path: string): string {
  const dot = path.lastIndexOf(".");
  if (dot < 0) return "plaintext";
  const ext = path.slice(dot + 1).toLowerCase();
  return EXT_TO_LANG[ext] ?? "plaintext";
}
