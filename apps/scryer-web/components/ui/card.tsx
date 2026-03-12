import * as React from "react";
import { cn } from "@/lib/utils";

function Card({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <section
      data-slot="card"
      className={cn(
        "rounded-xl border border-border bg-card text-card-foreground p-4 shadow-sm",
        className,
      )}
      {...props}
    />
  );
}

function CardHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div data-slot="card-header" className={cn("mb-3", className)} {...props} />;
}

function CardTitle({ className, ...props }: React.HTMLAttributes<HTMLHeadingElement>) {
  return <h2 data-slot="card-title" className={cn("text-lg font-semibold leading-none", className)} {...props} />;
}

function CardContent({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div data-slot="card-content" className={cn("space-y-3", className)} {...props} />;
}

export { Card, CardHeader, CardTitle, CardContent };
