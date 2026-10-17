import { openUrl } from "@tauri-apps/plugin-opener";

/** Handle links in rendered message content without navigating the webview. */
export function openExternalContentLink(event: MouseEvent): boolean {
  if (event.defaultPrevented || event.button > 1) return false;
  const target = event.target as Element | null;
  const element = target?.nodeType === 3 ? target.parentElement : target;
  const anchor = element?.closest?.("a[href]");
  if (!anchor) return false;

  const href = anchor.getAttribute("href")?.trim() ?? "";
  if (href.startsWith("#")) return false;

  // Cancel synchronously, including invalid URLs and opener failures. Never
  // fall back to window.open/location: those can replace the RunJam webview.
  event.preventDefault();
  let url: URL;
  try {
    url = new URL(href.startsWith("//") ? `https:${href}` : href);
  } catch {
    return true;
  }
  if (!["https:", "http:", "mailto:", "tel:"].includes(url.protocol)) return true;

  void openUrl(url.href).catch((error) => {
    console.error("Failed to open external content link:", error);
  });
  return true;
}
