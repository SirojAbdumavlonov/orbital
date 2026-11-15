import { Toaster } from "sonner";
import { useResolvedTheme } from "../lib/settings";

/** Notifications, drawn by Sonner and styled to match the app (see .toast in styles.css). */
export function Toasts() {
  const theme = useResolvedTheme();
  return (
    <Toaster
      theme={theme}
      position="bottom-right"
      closeButton
      visibleToasts={4}
      toastOptions={{ unstyled: true, classNames: { toast: "toast", title: "toast-title", description: "toast-body", actionButton: "btn primary toast-action", closeButton: "toast-close", success: "success", error: "error", icon: "toast-icon" } }}
    />
  );
}
