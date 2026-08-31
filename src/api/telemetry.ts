import { invoke } from "@tauri-apps/api/core";

export interface TelemetryStatus {
  installationId: string;
  enabled: boolean;
}

export async function getTelemetryStatus(): Promise<TelemetryStatus> {
  return invoke<TelemetryStatus>("get_telemetry_status");
}

export async function setTelemetryEnabled(enabled: boolean): Promise<TelemetryStatus> {
  return invoke<TelemetryStatus>("set_telemetry_enabled", { enabled });
}

export async function trackEvent(
  eventName: string,
  eventProps?: Record<string, unknown>,
): Promise<void> {
  return invoke<void>("track_event", { eventName, eventProps });
}

/**
 * Fire-and-forget telemetry event. Best-effort only: swallows errors and
 * never throws / blocks, so telemetry can never disturb the product UX.
 */
export function track(eventName: string, eventProps?: Record<string, unknown>): void {
  trackEvent(eventName, eventProps).catch(() => {});
}

export type ErrorLevel = "error" | "warn" | "info";

export async function reportErrorEvent(
  level: ErrorLevel,
  category: string,
  message: string,
  stack?: string,
  context?: Record<string, unknown>,
): Promise<void> {
  return invoke<void>("report_error", { level, category, message, stack, context });
}

/**
 * Fire-and-forget error report. The backend sanitizes the message (home paths,
 * API keys) and length-caps it before queueing, and respects the user's
 * telemetry opt-out. Never throws / blocks, same contract as `track`.
 */
export function reportError(
  level: ErrorLevel,
  category: string,
  message: string,
  stack?: string,
  context?: Record<string, unknown>,
): void {
  reportErrorEvent(level, category, message, stack, context).catch(() => {});
}

export async function getProxyConfig(): Promise<string> {
  return invoke<string>("get_proxy_config");
}

export async function setProxyConfig(proxy: string): Promise<void> {
  return invoke<void>("set_proxy_config", { proxy });
}

export async function testProxy(proxy: string): Promise<void> {
  return invoke<void>("test_proxy", { proxy });
}

export async function submitFeedback(
  feedbackType: string,
  content: string,
  email?: string,
  screenshotUrl?: string,
): Promise<void> {
  return invoke<void>("submit_feedback", {
    feedbackType,
    content,
    email,
    screenshotUrl,
  });
}

export interface UpdateInfo {
  updateAvailable: boolean;
  latestVersion?: string | null;
  publishedAt?: string | null;
  notes?: string | null;
  downloadUrl?: string | null;
  downloadUrls?: Record<string, string> | null;
}

export async function checkForUpdates(current: string): Promise<UpdateInfo> {
  return invoke<UpdateInfo>("check_for_updates", { current });
}

export interface UpdateCheckResult {
  updateAvailable: boolean;
  action: "install" | "open_download";
  latestVersion?: string | null;
  notes?: string | null;
  downloadUrl?: string | null;
  /** 备用下载源，如 { github: "...", cn: "..." } */
  downloadUrls?: Record<string, string> | null;
}

export async function checkUpdateUi(current: string): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>("check_update_ui", { current });
}

export async function installUpdate(): Promise<void> {
  return invoke<void>("install_update");
}

export interface Announcement {
  id: string;
  title: string;
  body: string;
  level: "info" | "important";
  createdAt?: string | null;
}

export async function getAnnouncements(): Promise<Announcement[]> {
  return invoke<Announcement[]>("get_announcements");
}

export async function markAnnouncementRead(id: string): Promise<void> {
  return invoke<void>("mark_announcement_read", { id });
}
