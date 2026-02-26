import { Link } from "react-router-dom";

export default function NotFound() {
  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-8">
      <div className="w-full max-w-lg rounded-lg border border-border bg-card/70 p-6 text-center">
        <h1 className="mb-2 text-xl font-semibold text-foreground">Page not found</h1>
        <p className="mb-4 text-sm text-muted-foreground">The page you requested does not exist.</p>
        <Link
          to="/"
          className="inline-flex h-10 items-center rounded-md border border-border bg-muted px-4 py-2 text-sm font-medium text-foreground transition-colors hover:bg-accent"
        >
          Go to Home
        </Link>
      </div>
    </main>
  );
}
