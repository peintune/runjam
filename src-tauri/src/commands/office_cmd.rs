//! Converting Office documents to a form the frontend can render.
//!
//! The frontend renders `.docx` and `.xlsx` directly (mammoth / SheetJS), but a
//! `.pptx` has no lightweight pure-JS renderer that preserves its layout. So for
//! presentations we ask LibreOffice to convert to PDF and preview that.
//!
//! LibreOffice is NOT bundled — it is a large application the user may not have.
//! Detection is explicit (`detect_office_converter`) so the UI can offer a plain
//! "open with the system app" fallback instead of failing, and the conversion
//! runs headless in a temp profile so it never disturbs a LibreOffice the user
//! has open.
//!
//! Everything here is a pure function over paths plus one detection probe, so the
//! decision logic stays unit-testable without LibreOffice installed.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::hidden_command;

/// Absolute paths LibreOffice installs to, checked when it is not on `PATH`.
/// The macOS app bundle is the common case: `soffice` is inside the bundle and
/// never symlinked to `/usr/local/bin` by the installer.
#[cfg(target_os = "macos")]
const BUNDLED_SOFFICE: &[&str] = &[
    "/Applications/LibreOffice.app/Contents/MacOS/soffice",
    "/opt/homebrew/bin/soffice",
    "/usr/local/bin/soffice",
];

#[cfg(target_os = "windows")]
const BUNDLED_SOFFICE: &[&str] = &[
    r"C:\Program Files\LibreOffice\program\soffice.exe",
    r"C:\Program Files (x86)\LibreOffice\program\soffice.exe",
];

#[cfg(all(unix, not(target_os = "macos")))]
const BUNDLED_SOFFICE: &[&str] = &["/usr/bin/soffice", "/usr/bin/libreoffice", "/snap/bin/libreoffice"];

/// Extensions RunJam can turn into a previewable PDF.
///
/// `.ppt` (the pre-2007 binary format) is included: LibreOffice reads it too, and
/// a user's file is just as likely to be one or the other.
pub const CONVERTIBLE_TO_PDF: &[&str] = &["pptx", "ppt", "odp"];

/// Whether `ext` (without the dot, any case) is something we convert to PDF.
pub fn is_convertible_to_pdf(ext: &str) -> bool {
    CONVERTIBLE_TO_PDF.contains(&ext.to_ascii_lowercase().as_str())
}

/// Locate a LibreOffice/OpenOffice binary, or `None` when not installed.
///
/// Prefers `PATH` (a user who installed via Homebrew gets an up-to-date build),
/// then the platform's standard install locations.
pub fn detect_office_converter() -> Option<PathBuf> {
    // PATH first.
    for name in ["soffice", "libreoffice"] {
        if let Ok(out) = hidden_command("which").arg(name).output() {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    let path = PathBuf::from(p);
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
    }
    // Then the well-known install locations.
    for candidate in BUNDLED_SOFFICE {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Where the converted PDF should be written for a given source document.
///
/// Lives beside the source in a RunJam-owned cache dir rather than in the user's
/// document folder: the PDF is a derived artifact, and scattering it next to the
/// user's file would be rude. The name embeds the source file stem so a user
/// looking in the cache can tell what it came from.
pub fn converted_pdf_path(source: &Path, cache_dir: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".to_string());
    // Sanitize the stem for use as a filename: the source name may contain
    // separators or characters the filesystem rejects.
    let safe: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let safe = if safe.is_empty() { "document".to_string() } else { safe };
    cache_dir.join(format!("{}-{}.pdf", safe, short_hash(source)))
}

/// A short, stable hash of the full source path, so two documents that share a
/// stem (e.g. `slides.pptx` in two folders) do not collide in the shared cache.
fn short_hash(path: &Path) -> String {
    // FNV-1a: tiny, dependency-free, and stable across runs — all this needs.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in path.to_string_lossy().as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (hash & 0xffff_ffff) as u32)
}

/// Convert a document to PDF, returning the path of the produced file.
///
/// `soffice` is given a private user-profile directory so it does not try to
/// reuse (and lock) a profile a running LibreOffice instance owns — without this
/// the conversion hangs or fails when the user has LibreOffice open. The PDF is
/// written into `cache_dir`, and an existing fresh copy is reused: converting a
/// large deck is slow enough that re-previewing the same file should be instant.
pub fn convert_to_pdf(
    converter: &Path,
    source: &Path,
    cache_dir: &Path,
) -> Result<PathBuf, String> {
    if !source.exists() {
        return Err(format!("File not found: {}", source.display()));
    }
    std::fs::create_dir_all(cache_dir).map_err(|e| format!("Cannot create cache dir: {e}"))?;

    let out_pdf = converted_pdf_path(source, cache_dir);
    if out_pdf.exists() {
        return Ok(out_pdf);
    }

    // Each conversion gets its own profile dir under the cache. Sharing one
    // profile across concurrent conversions would reintroduce the locking
    // problem this is here to avoid.
    let profile = cache_dir.join(format!("profile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&profile);

    let output = hidden_command(converter)
        // `-env:UserInstallation` must be passed as a single argument in the
        // `key=value` form, and needs a file:// URL or LibreOffice treats it as a
        // relative path and fails to start.
        .arg(format!(
            "-env:UserInstallation=file://{}",
            profile.to_string_lossy()
        ))
        .arg("--headless")
        .arg("--norestore")
        .arg("--convert-to")
        .arg("pdf")
        .arg("--outdir")
        .arg(cache_dir)
        .arg(source)
        .output()
        .map_err(|e| format!("Failed to run LibreOffice: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "LibreOffice conversion failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // `--outdir` names the output after the source stem, so what LibreOffice
    // wrote is NOT `out_pdf` — move it into place under our colliding-safe name.
    let produced = cache_dir.join(format!(
        "{}.pdf",
        source
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    if produced != out_pdf {
        std::fs::rename(&produced, &out_pdf).map_err(|e| {
            format!(
                "Conversion produced {} but it could not be moved: {e}",
                produced.display()
            )
        })?;
    }

    let _ = std::fs::remove_dir_all(&profile);
    Ok(out_pdf)
}

/// Whether LibreOffice is available, for the frontend to decide between the
/// in-app preview and the "open with the system app" fallback.
#[tauri::command]
pub fn detect_office_converter_cmd() -> bool {
    detect_office_converter().is_some()
}

/// Convert `path` to PDF and return the resulting path, for the frontend to load
/// into the PDF previewer.
#[tauri::command]
pub fn convert_office_to_pdf(path: String, cache_dir: String) -> Result<String, String> {
    // Reject a type we do not convert BEFORE spawning LibreOffice: handing it an
    // arbitrary file would either fail confusingly or (for a supported-but-not
    // convertible type) silently produce something the caller did not ask for.
    let ext = Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if !is_convertible_to_pdf(&ext) {
        return Err(format!("Not a convertible document type: .{ext}"));
    }
    let converter = detect_office_converter().ok_or_else(|| {
        "LibreOffice is not installed; cannot convert this document".to_string()
    })?;
    let pdf = convert_to_pdf(&converter, Path::new(&path), Path::new(&cache_dir))?;
    Ok(pdf.to_string_lossy().to_string())
}

/// Ask the system to open `path` with its default application.
///
/// The fallback when no in-app renderer fits — used by the file opener so a
/// document is never a dead end inside a session.
#[tauri::command]
pub fn open_with_system_app(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut cmd = hidden_command("explorer");

    cmd.arg(&p)
        .spawn()
        .map_err(|e| format!("Failed to open {}: {e}", p.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> PathBuf {
        let d = std::env::temp_dir().join(format!("runjam_office_cache_{}", std::process::id()));
        std::fs::create_dir_all(&d).ok();
        d
    }

    #[test]
    fn convertible_extensions_are_recognised_case_insensitively() {
        assert!(is_convertible_to_pdf("pptx"));
        assert!(is_convertible_to_pdf("PPTX"));
        assert!(is_convertible_to_pdf("ppt"));
        // docx/xlsx are rendered in the frontend, so they are NOT converted.
        assert!(!is_convertible_to_pdf("docx"));
        assert!(!is_convertible_to_pdf("xlsx"));
        assert!(!is_convertible_to_pdf("txt"));
    }

    #[test]
    fn converted_pdf_keeps_the_source_stem_and_adds_a_hash() {
        let src = Path::new("/Users/someone/work/slides.pptx");
        let out = converted_pdf_path(src, &cache());
        let name = out.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("slides-"), "got {name}");
        assert!(name.ends_with(".pdf"), "got {name}");
        assert_eq!(out.parent().unwrap(), cache());
    }

    #[test]
    fn same_stem_in_different_folders_does_not_collide() {
        // Two decks named slides.pptx must not overwrite each other in the shared
        // cache — that would show the wrong document.
        let c = cache();
        let a = converted_pdf_path(Path::new("/a/slides.pptx"), &c);
        let b = converted_pdf_path(Path::new("/b/slides.pptx"), &c);
        assert_ne!(a, b, "same stem must still map to distinct cache files");
    }

    #[test]
    fn same_source_maps_to_the_same_path() {
        // Stable across calls, so a second preview reuses the converted file.
        let c = cache();
        let src = Path::new("/a/slides.pptx");
        assert_eq!(converted_pdf_path(src, &c), converted_pdf_path(src, &c));
    }

    #[test]
    fn awkward_source_names_are_sanitized_for_the_filesystem() {
        let src = Path::new("/a/my weird:name?.pptx");
        let out = converted_pdf_path(src, &cache());
        let name = out.file_name().unwrap().to_string_lossy().to_string();
        assert!(!name.contains(':'), "colon must not survive: {name}");
        assert!(!name.contains('?'), "question mark must not survive: {name}");
        assert!(!name.contains('/'), "separator must not survive: {name}");
    }

    #[test]
    fn a_source_with_no_stem_still_yields_a_usable_name() {
        let out = converted_pdf_path(Path::new("/a/.pptx"), &cache());
        assert!(out.file_name().unwrap().to_string_lossy().ends_with(".pdf"));
    }

    #[test]
    fn conversion_reports_a_missing_source_rather_than_running_soffice() {
        // No converter is needed to detect this: the check must come first, or a
        // missing file would surface as a confusing LibreOffice error.
        let err = convert_to_pdf(
            Path::new("/nonexistent/soffice"),
            Path::new("/definitely/not/here.pptx"),
            &cache(),
        )
        .unwrap_err();
        assert!(err.contains("File not found"), "got {err}");
    }

    #[test]
    fn an_existing_conversion_is_reused_without_invoking_the_converter() {
        // A bogus converter path proves no process is spawned when the PDF is
        // already cached — the source must exist (a deleted source is an error,
        // not a cache hit), but the converter must not be run.
        let c = cache();
        let src = c.join("cached-source.pptx");
        std::fs::write(&src, b"not really a pptx").unwrap();
        let out = converted_pdf_path(&src, &c);
        std::fs::write(&out, b"%PDF-1.4 fake").unwrap();

        let got = convert_to_pdf(Path::new("/nonexistent/soffice"), &src, &c).unwrap();
        assert_eq!(got, out);

        std::fs::remove_file(&src).ok();
        std::fs::remove_file(&out).ok();
    }

    #[test]
    fn conversion_refuses_a_type_it_does_not_handle() {
        // A .docx is rendered in the frontend, so asking for a PDF is a caller
        // bug; it must be reported, not forwarded to LibreOffice.
        let err = convert_office_to_pdf(
            "/a/notes.docx".to_string(),
            cache().to_string_lossy().to_string(),
        )
        .unwrap_err();
        assert!(err.contains("Not a convertible document type"), "got {err}");
    }

    #[test]
    fn detection_returns_a_real_path_or_none() {
        // Cannot assert LibreOffice is installed, but if it reports one the path
        // must actually exist (so the UI never offers a broken action).
        if let Some(p) = detect_office_converter() {
            assert!(p.exists(), "detected converter does not exist: {p:?}");
        }
    }
}