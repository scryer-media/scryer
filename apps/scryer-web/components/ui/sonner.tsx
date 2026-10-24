import {
  CircleCheckIcon,
  InfoIcon,
  OctagonXIcon,
  TriangleAlertIcon,
} from "lucide-react"
import { useTheme } from "next-themes"
import { Toaster as Sonner, toast, type ToasterProps } from "sonner"
import { isDarkTheme } from "@/lib/theme"
import { LoadingMark } from "@/components/common/loading-mark";

const Toaster = ({ style, toastOptions, ...props }: ToasterProps) => {
  const { resolvedTheme } = useTheme()
  const isDark = isDarkTheme(resolvedTheme)

  const themeStyle = (
    isDark
      ? {
          "--normal-bg": "var(--background)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border-color)",
          "--normal-bg-hover": "var(--background)",
          "--normal-border-hover": "var(--border-color)",
          "--success-bg": "var(--scry-success-bg)",
          "--success-border": "var(--scry-success-border)",
          "--success-text": "var(--scry-success-text)",
          "--error-bg": "var(--scry-danger-bg)",
          "--error-border": "var(--scry-danger-border)",
          "--error-text": "var(--scry-danger-text)",
          "--warning-bg": "var(--scry-warning-bg)",
          "--warning-border": "var(--scry-warning-border)",
          "--warning-text": "var(--scry-warning-text)",
          "--info-bg": "var(--scry-info-bg)",
          "--info-border": "var(--scry-info-border)",
          "--info-text": "var(--scry-info-text)",
          "--border-radius": "var(--radius)",
        }
      : {
          "--normal-bg": "var(--background)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border-color)",
          "--normal-bg-hover": "var(--background)",
          "--normal-border-hover": "var(--border-color)",
          "--success-bg": "var(--scry-success-bg)",
          "--success-border": "var(--scry-success-border)",
          "--success-text": "var(--scry-success-text)",
          "--error-bg": "var(--scry-danger-bg)",
          "--error-border": "var(--scry-danger-border)",
          "--error-text": "var(--scry-danger-text)",
          "--warning-bg": "var(--scry-warning-bg)",
          "--warning-border": "var(--scry-warning-border)",
          "--warning-text": "var(--scry-warning-text)",
          "--info-bg": "var(--scry-info-bg)",
          "--info-border": "var(--scry-info-border)",
          "--info-text": "var(--scry-info-text)",
          "--border-radius": "var(--radius)",
        }
  ) as React.CSSProperties

  return (
    <Sonner
      theme={isDark ? "dark" : "light"}
      richColors
      className="toaster group"
      toastOptions={{
        className: "shadow-[0_18px_48px_rgba(0,0,0,0.34)]",
        classNames: {
          toast:
            "rounded-[12px] border border-[var(--scry-border2)] !bg-[var(--background)]",
          success:
            "border-[var(--scry-success-border)] !bg-[var(--background)] text-[var(--scry-success-text)]",
          error:
            "border-[var(--scry-danger-border)] !bg-[var(--background)] text-[var(--scry-danger-text)]",
          // Every variant paints the same opaque surface. The earlier
          // `bg-[linear-gradient(...),var(--scry-bg)]` compiled to
          // `background-image`, where a bare colour is not a valid layer, so the
          // whole declaration was dropped and Sonner's translucent rich colour
          // showed through. `!` is what beats rich colours.
          warning:
            "border-[var(--scry-warning-border)] !bg-[var(--background)] text-[var(--scry-warning-text)]",
          info:
            "border-[var(--scry-info-border)] !bg-[var(--background)] text-[var(--scry-info-text)]",
        },
        ...toastOptions,
      }}
      icons={{
        success: <CircleCheckIcon className="size-4" />,
        info: <InfoIcon className="size-4" />,
        warning: <TriangleAlertIcon className="size-4" />,
        error: <OctagonXIcon className="size-4" />,
        loading: <LoadingMark className="size-4" />,
      }}
      style={{ ...themeStyle, ...style } as React.CSSProperties}
      {...props}
    />
  )
}

export { Toaster, toast }
