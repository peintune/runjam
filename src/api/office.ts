import { invoke } from "@tauri-apps/api/core";

/** Whether LibreOffice is installed, so a `.pptx` can be converted for preview. */
export async function detectOfficeConverter(): Promise<boolean> {
  return invoke<boolean>("detect_office_converter_cmd");
}

/**
 * Convert an Office document (pptx/ppt/odp) to PDF and return the PDF's path.
 *
 * RunJam does not ship a presenter, so a presentation is shown by converting it
 * to PDF and reusing the PDF previewer. Fails when LibreOffice is absent — call
 * `detectOfficeConverter` first to offer the fallback instead of an error.
 */
export async function convertOfficeToPdf(path: string, cacheDir: string): Promise<string> {
  return invoke<string>("convert_office_to_pdf", { path, cacheDir });
}

/** Open a file with the OS default application (the universal fallback). */
export async function openWithSystemApp(path: string): Promise<void> {
  return invoke<void>("open_with_system_app", { path });
}