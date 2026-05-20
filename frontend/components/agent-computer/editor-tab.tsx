"use client";

import dynamic from "next/dynamic";
import { useEffect, useState } from "react";
import { FileTree } from "@/components/agent-computer/file-tree";
import { getSession } from "@/lib/api";
import { languageForPath, readFile } from "@/lib/workspace";

const MonacoEditor = dynamic(() => import("@monaco-editor/react"), {
  ssr: false,
  loading: () => <p className="text-xs text-gray-500">Loading editor…</p>,
});

type Props = {
  sessionId: string | null;
};

export function EditorTab({ sessionId }: Props) {
  const [hasSandbox, setHasSandbox] = useState<boolean | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [content, setContent] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (sessionId === null) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setHasSandbox(null);
      return;
    }
    (async () => {
      try {
        const detail = await getSession(sessionId);
         
        setHasSandbox(detail.sandbox !== null);
        setError(null);
      } catch (e) {
         
        setError(e instanceof Error ? e.message : String(e));
        setHasSandbox(false);
      }
    })();
  }, [sessionId]);

  useEffect(() => {
    if (sessionId === null || selectedPath === null) return;
    (async () => {
      try {
        const text = await readFile(sessionId, selectedPath);
         
        setContent(text);
      } catch (e) {
         
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
  }, [sessionId, selectedPath]);

  if (sessionId === null) {
    return (
      <p className="text-sm text-gray-500">
        Select a task to view its workspace.
      </p>
    );
  }
  if (hasSandbox === null) return <p className="text-sm text-gray-500">Loading…</p>;
  if (hasSandbox === false) {
    return (
      <p className="text-sm text-gray-500">
        {error ?? "No sandbox running for this session yet."}
      </p>
    );
  }

  return (
    <div className="flex h-full gap-2">
      <div className="w-1/3 min-w-[160px] overflow-auto border-r pr-2">
        <FileTree
          sessionId={sessionId}
          selectedPath={selectedPath}
          onSelect={setSelectedPath}
        />
      </div>
      <div className="flex-1 overflow-hidden">
        {selectedPath === null ? (
          <p className="text-xs text-gray-500">Pick a file to view.</p>
        ) : (
          <MonacoEditor
            height="100%"
            language={languageForPath(selectedPath)}
            value={content}
            options={{
              readOnly: true,
              minimap: { enabled: false },
              fontSize: 12,
            }}
          />
        )}
      </div>
    </div>
  );
}
