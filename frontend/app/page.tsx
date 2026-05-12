export default function Home() {
  return (
    <main className="min-h-screen flex flex-col items-center justify-center gap-4 p-8">
      <h1 className="text-3xl font-semibold">Seasoned Hand — Phase 0</h1>
      <p className="text-gray-600 dark:text-gray-400">
        UI scaffolding lands in story 0.19 (3-panel layout).
      </p>
      <p className="text-sm text-gray-400">
        Build: {process.env.NEXT_PUBLIC_BUILD ?? "dev"}
      </p>
    </main>
  );
}
