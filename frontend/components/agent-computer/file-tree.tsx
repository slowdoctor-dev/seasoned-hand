"use client";

import { useCallback, useEffect, useState } from "react";
import { listDir, type WorkspaceEntry } from "@/lib/workspace";

type Props = {
  sessionId: string;
  selectedPath: string | null;
  onSelect: (path: string) => void;
};

type Node = WorkspaceEntry & {
  path: string;
  open?: boolean;
  children?: Node[];
};

export function FileTree({ sessionId, selectedPath, onSelect }: Props) {
  const [root, setRoot] = useState<Node[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadRoot = useCallback(async () => {
    try {
      const listing = await listDir(sessionId, "");
      setRoot(
        listing.entries.map((e) => ({
          ...e,
          path: e.name,
        })),
      );
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [sessionId]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void loadRoot();
  }, [loadRoot]);

  const toggleDir = useCallback(
    async (path: string) => {
      const expand = async (nodes: Node[]): Promise<Node[]> =>
        Promise.all(
          nodes.map(async (n) => {
            if (n.path !== path) {
              if (n.children) {
                return { ...n, children: await expand(n.children) };
              }
              return n;
            }
            if (n.type !== "dir") return n;
            if (n.open) return { ...n, open: false };
            if (!n.children) {
              try {
                const listing = await listDir(sessionId, n.path);
                return {
                  ...n,
                  open: true,
                  children: listing.entries.map((e) => ({
                    ...e,
                    path: `${n.path}/${e.name}`,
                  })),
                };
              } catch (e) {
                setError(e instanceof Error ? e.message : String(e));
                return n;
              }
            }
            return { ...n, open: true };
          }),
        );
      if (root === null) return;
      const next = await expand(root);
      setRoot(next);
    },
    [root, sessionId],
  );

  if (error) return <p className="text-xs text-red-600">{error}</p>;
  if (root === null) return <p className="text-xs text-gray-500">Loading…</p>;
  if (root.length === 0)
    return <p className="text-xs text-gray-500">Empty workspace.</p>;

  return (
    <ul className="overflow-auto text-xs">
      {root.map((n) => (
        <TreeNode
          key={n.path}
          node={n}
          depth={0}
          selectedPath={selectedPath}
          onToggle={toggleDir}
          onSelect={onSelect}
        />
      ))}
    </ul>
  );
}

function TreeNode({
  node,
  depth,
  selectedPath,
  onToggle,
  onSelect,
}: {
  node: Node;
  depth: number;
  selectedPath: string | null;
  onToggle: (path: string) => void;
  onSelect: (path: string) => void;
}) {
  const isSelected = node.type === "file" && node.path === selectedPath;
  return (
    <li>
      <button
        type="button"
        onClick={() =>
          node.type === "dir" ? onToggle(node.path) : onSelect(node.path)
        }
        className={`w-full truncate text-left hover:bg-gray-100 dark:hover:bg-gray-800 ${
          isSelected ? "bg-blue-100 font-medium dark:bg-blue-900/30" : ""
        }`}
        style={{ paddingLeft: 4 + depth * 12 }}
      >
        {node.type === "dir" ? (node.open ? "▾ " : "▸ ") : "  "}
        {node.name}
      </button>
      {node.type === "dir" && node.open && node.children && (
        <ul>
          {node.children.map((child) => (
            <TreeNode
              key={child.path}
              node={child}
              depth={depth + 1}
              selectedPath={selectedPath}
              onToggle={onToggle}
              onSelect={onSelect}
            />
          ))}
        </ul>
      )}
    </li>
  );
}
