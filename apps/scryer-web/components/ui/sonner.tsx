
import {
  CircleCheckIcon,
  InfoIcon,
  Loader2Icon,
  OctagonXIcon,
  TriangleAlertIcon,
} from "lucide-react"
import { useTheme } from "next-themes"
import { Toaster as Sonner, type ToasterProps } from "sonner"

const Toaster = ({ ...props }: ToasterProps) => {
  const { resolvedTheme } = useTheme()
  const isDark = resolvedTheme === "dark"

  return (
    <Sonner
      theme={isDark ? "dark" : "light"}
      richColors
      className="toaster group"
      toastOptions={{
        className: "bg-background shadow-sm shadow-black/35",
        classNames: {
          toast: "border border-border/30",
          success: isDark ? "border-emerald-500 bg-emerald-950" : "border-emerald-500 bg-emerald-50",
          error: isDark ? "border-red-500 bg-red-950" : "border-red-500 bg-red-50",
          warning: "border-amber-400/55",
          info: "border-sky-400/55",
        },
      }}
      icons={{
        success: <CircleCheckIcon className="size-4" />,
        info: <InfoIcon className="size-4" />,
        warning: <TriangleAlertIcon className="size-4" />,
        error: <OctagonXIcon className="size-4" />,
        loading: <Loader2Icon className="size-4 animate-spin" />,
      }}
      style={
        isDark
          ? {
              "--normal-bg": "var(--popover)",
              "--normal-text": "var(--popover-foreground)",
              "--normal-border": "var(--border-color)",
              "--normal-bg-hover": "var(--card)",
              "--normal-border-hover": "var(--border-color)",
              "--success-bg": "rgb(2, 23, 18)",
              "--success-border": "rgb(16, 185, 129)",
              "--success-text": "rgb(209, 250, 229)",
              "--error-bg": "rgb(28, 7, 9)",
              "--error-border": "rgb(239, 68, 68)",
              "--error-text": "rgb(254, 226, 226)",
              "--warning-bg": "rgb(36, 29, 6)",
              "--warning-border": "rgb(251, 191, 36)",
              "--warning-text": "rgb(254, 243, 199)",
              "--info-bg": "rgb(7, 22, 42)",
              "--info-border": "rgb(96, 165, 250)",
              "--info-text": "rgb(219, 234, 254)",
              "--border-radius": "var(--radius)",
            } as React.CSSProperties
          : {
              "--normal-bg": "var(--popover)",
              "--normal-text": "var(--popover-foreground)",
              "--normal-border": "var(--border-color)",
              "--normal-bg-hover": "var(--card)",
              "--normal-border-hover": "var(--border-color)",
              "--success-bg": "rgb(240, 253, 244)",
              "--success-border": "rgb(16, 185, 129)",
              "--success-text": "rgb(5, 46, 22)",
              "--error-bg": "rgb(254, 242, 242)",
              "--error-border": "rgb(239, 68, 68)",
              "--error-text": "rgb(69, 10, 10)",
              "--warning-bg": "rgb(254, 252, 232)",
              "--warning-border": "rgb(251, 191, 36)",
              "--warning-text": "rgb(66, 32, 6)",
              "--info-bg": "rgb(239, 246, 255)",
              "--info-border": "rgb(96, 165, 250)",
              "--info-text": "rgb(30, 58, 138)",
              "--border-radius": "var(--radius)",
            } as React.CSSProperties
      }
      {...props}
    />
  )
}

export { Toaster }
