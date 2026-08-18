use serde::Serialize;
use calamine::Reader;
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
    pub extension: String,
}

#[derive(Debug, Serialize)]
pub struct FileSearchResult {
    pub path: String,
    pub name: String,
    pub relative_path: String,
    pub match_type: String,
    pub line_number: Option<usize>,
    pub line_content: Option<String>,
}

/// Directories to skip when searching.
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", ".next", ".cache",
    ".vscode", ".idea", "__pycache__", ".pytest_cache", "venv",
    ".venv", "env", ".env", ".mypy_cache", ".tox", "build",
    ".angular", ".svelte-kit", ".nuxt", ".gradle", ".terraform",
    // Large vendored/binary dirs that bloat traversal and never contain
    // user-editable source files (node headers, ACP packages, prebuilt bins)
    "nodejs", "acp", "binaries",
];

/// Text file extensions eligible for content search.
const TEXT_EXTS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte",
    "rs", "go", "py", "java", "c", "cpp", "h", "hpp", "cc", "cxx",
    "rb", "php", "swift", "kt", "scala", "sh", "bash", "zsh", "fish",
    "json", "yaml", "yml", "toml", "xml", "ini", "cfg", "conf",
    "md", "mdx", "txt", "log", "csv", "tsv",
    "css", "scss", "sass", "less", "styl",
    "html", "htm", "svg", "vue",
    "sql", "graphql", "gql", "prisma", "proto",
    "dockerfile", "makefile", "cmake",
    "gitignore", "env", "editorconfig",
    "lua", "r", "dart", "ex", "exs", "erl", "clj", "cljs", "hs",
];

/// Max file size for content search (1 MB).
const MAX_CONTENT_FILE_SIZE: u64 = 1024 * 1024;

fn is_text_file(ext: &str) -> bool {
    let ext_lower = ext.to_lowercase();
    TEXT_EXTS.contains(&ext_lower.as_str())
}

fn should_skip(name: &str) -> bool {
    if name.starts_with('.') && name != ".env" && name != ".gitignore" && name != ".editorconfig" {
        return true;
    }
    SKIP_DIRS.contains(&name)
}

// ── .gitignore support for the @ mention picker ────────────────────
// The mention picker reads the project root .gitignore and dynamically
// excludes listed dirs/files from both browsing and searching. This keeps
// generated/vendored content out of results without hard-coding every
// project-specific path. Negation rules (!) are ignored for simplicity.

/// Parsed .gitignore: exact names (case-insensitive) + simple glob patterns.
#[derive(Default)]
struct GitignoreSkip {
    names: HashSet<String>, // e.g. "node_modules", "logs", "dist-landing"
    globs: Vec<String>,    // e.g. "*.log", "*.local"
}

/// Parse .gitignore at `root` into skip names + glob patterns.
/// Only the last path component is used (e.g. "src-tauri/nodejs/" → "nodejs")
/// so patterns apply anywhere in the tree, matching `should_skip` semantics.
fn parse_gitignore(root: &Path) -> GitignoreSkip {
    let mut gi = GitignoreSkip::default();
    let path = root.join(".gitignore");
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return gi, // no .gitignore or unreadable — nothing to add
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
            continue;
        }
        // Strip leading/trailing slashes; take last path component.
        let cleaned = trimmed.trim_matches('/');
        if cleaned.is_empty() {
            continue;
        }
        let name = cleaned.rsplit('/').next().unwrap_or(cleaned);
        let name_lower = name.to_lowercase();
        if name.contains('*') || name.contains('?') {
            // Skip pure-wildcard patterns (e.g. ".vscode/*" → "*") — they'd
            // match every file. The parent dir is already skipped by the
            // hidden-dir rule, so this scoped rule adds nothing useful.
            let stripped: String = name.chars().filter(|c| *c != '*' && *c != '?').collect();
            if !stripped.is_empty() {
                gi.globs.push(name_lower);
            }
        } else {
            gi.names.insert(name_lower);
        }
    }
    gi
}

/// Match a name against a simple glob pattern (supports `*.ext`, `prefix*`).
fn glob_match(name_lower: &str, pattern_lower: &str) -> bool {
    if let Some(suffix) = pattern_lower.strip_prefix('*') {
        return name_lower.ends_with(suffix);
    }
    if let Some(prefix) = pattern_lower.strip_suffix('*') {
        return name_lower.starts_with(prefix);
    }
    name_lower == pattern_lower
}

/// Skip check for the @ mention picker: built-in list + .gitignore patterns.
fn should_skip_mention(name: &str, gi: &GitignoreSkip) -> bool {
    // Hidden files/dirs (except common tracked config files)
    if name.starts_with('.') && name != ".gitignore" && name != ".editorconfig" {
        return true;
    }
    if SKIP_DIRS.contains(&name) {
        return true;
    }
    let name_lower = name.to_lowercase();
    if gi.names.contains(&name_lower) {
        return true;
    }
    gi.globs.iter().any(|g| glob_match(&name_lower, g))
}

fn walk_dir(root: &Path, dir: &Path, query_lower: &str, results: &mut Vec<FileSearchResult>, limit: usize) {
    if results.len() >= limit {
        return;
    }
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in read_dir {
        if results.len() >= limit {
            return;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_name = entry.file_name().to_string_lossy().to_string();
        if should_skip(&file_name) {
            continue;
        }
        let path_buf = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        if metadata.is_dir() {
            walk_dir(root, &path_buf, query_lower, results, limit);
        } else {
            let name_lower = file_name.to_lowercase();
            let relative = path_buf
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| file_name.clone());

            // Filename match
            if name_lower.contains(query_lower) {
                results.push(FileSearchResult {
                    path: path_buf.to_string_lossy().to_string(),
                    name: file_name.clone(),
                    relative_path: relative.clone(),
                    match_type: "filename".to_string(),
                    line_number: None,
                    line_content: None,
                });
                if results.len() >= limit {
                    return;
                }
            }

            // Content match
            let ext = path_buf
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();
            // Also match files without extension (Makefile, Dockerfile, etc.)
            let no_ext_match = TEXT_EXTS.contains(&file_name.to_lowercase().as_str());
            if (is_text_file(&ext) || no_ext_match) && metadata.len() <= MAX_CONTENT_FILE_SIZE {
                if let Ok(content) = fs::read_to_string(&path_buf) {
                    for (i, line) in content.lines().enumerate() {
                        if line.to_lowercase().contains(query_lower) {
                            results.push(FileSearchResult {
                                path: path_buf.to_string_lossy().to_string(),
                                name: file_name.clone(),
                                relative_path: relative.clone(),
                                match_type: "content".to_string(),
                                line_number: Some(i + 1),
                                line_content: Some(line.trim().chars().take(200).collect()),
                            });
                            if results.len() >= limit {
                                return;
                            }
                            break; // Only first match per file
                        }
                    }
                }
            }
        }
    }
}

#[tauri::command]
pub fn search_files(root_path: String, query: String, limit: Option<usize>) -> Result<Vec<FileSearchResult>, String> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }
    let root = Path::new(&root_path);
    if !root.exists() || !root.is_dir() {
        return Err(format!("Invalid directory: {}", root_path));
    }
    let max_results = limit.unwrap_or(100);
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    walk_dir(root, root, &query_lower, &mut results, max_results);

    // Sort: filename matches first, then content matches
    results.sort_by(|a, b| {
        let a_is_filename = a.match_type == "filename";
        let b_is_filename = b.match_type == "filename";
        b_is_filename
            .cmp(&a_is_filename)
            .then(a.relative_path.cmp(&b.relative_path))
    });

    Ok(results)
}

#[tauri::command]
pub fn list_dir(path: String) -> Result<Vec<FileEntry>, String> {
    let dir = Path::new(&path);
    if !dir.exists() {
        return Err(format!("Directory not found: {}", path));
    }
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", path));
    }

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(dir).map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in read_dir {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let file_name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files and node_modules
        if file_name.starts_with('.') || file_name == "node_modules" || file_name == "target" {
            continue;
        }

        let path_buf = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|e| format!("Failed to read metadata: {}", e))?;

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            })
            .unwrap_or_default();

        let extension = path_buf
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();

        entries.push(FileEntry {
            name: file_name,
            path: path_buf.to_string_lossy().to_string(),
            is_dir: metadata.is_dir(),
            size: metadata.len(),
            modified,
            extension,
        });
    }

    // Sort: directories first, then alphabetically (case-insensitive)
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

#[tauri::command]
pub fn read_file_text(path: String) -> Result<String, String> {
    let metadata =
        fs::metadata(&path).map_err(|e| format!("Failed to read file metadata: {}", e))?;
    if metadata.len() > 100 * 1024 * 1024 {
        return Err("File too large (>100MB). Please open with external editor.".to_string());
    }

    fs::read_to_string(&path).map_err(|e| format!("Failed to read file: {}", e))
}

#[tauri::command]
pub fn write_file(path: String, content: String) -> Result<(), String> {
    fs::write(&path, &content).map_err(|e| format!("Failed to write file: {}", e))
}

#[tauri::command]
pub fn read_file_bytes(path: String) -> Result<Vec<u8>, String> {
    let metadata =
        fs::metadata(&path).map_err(|e| format!("Failed to read file metadata: {}", e))?;
    if metadata.len() > 50 * 1024 * 1024 {
        return Err("File too large (>50MB).".to_string());
    }
    fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))
}

#[tauri::command]
pub fn get_file_size(path: String) -> Result<u64, String> {
    let metadata =
        fs::metadata(&path).map_err(|e| format!("Failed to read file metadata: {}", e))?;
    Ok(metadata.len())
}

// ── Mention picker ──────────────────────────────────────────

/// Combined response for the @ mention picker: recent files + root-level entries.
#[derive(Debug, Serialize)]
pub struct MentionEntries {
    pub recent: Vec<FileEntry>,
    pub root: Vec<FileEntry>,
}

/// Build a FileEntry from a std::fs::DirEntry.
fn dir_entry_to_file_entry(entry: &std::fs::DirEntry) -> Option<FileEntry> {
    let file_name = entry.file_name().to_string_lossy().to_string();
    let path_buf = entry.path();
    let metadata = entry.metadata().ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| {
            chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        })
        .unwrap_or_default();
    let extension = path_buf
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    Some(FileEntry {
        name: file_name,
        path: path_buf.to_string_lossy().to_string(),
        is_dir: metadata.is_dir(),
        size: metadata.len(),
        modified,
        extension,
    })
}

/// Collect entries in a single directory (non-recursive, root-level listing).
fn collect_dir_entries(dir: &Path, gi: &GitignoreSkip) -> Vec<FileEntry> {
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return vec![],
    };
    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_name = entry.file_name().to_string_lossy().to_string();
        if should_skip_mention(&file_name, gi) {
            continue;
        }
        if let Some(fe) = dir_entry_to_file_entry(&entry) {
            entries.push(fe);
        }
    }
    // Sort: directories first, then alphabetically (case-insensitive)
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// Recursively collect files only (skip directories from output, but still
/// recurse into them). Used to find the most recently modified files.
fn walk_collect_files(dir: &Path, gi: &GitignoreSkip, out: &mut Vec<FileEntry>) {
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_name = entry.file_name().to_string_lossy().to_string();
        if should_skip_mention(&file_name, gi) {
            continue;
        }
        let path_buf = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            walk_collect_files(&path_buf, gi, out);
        } else if let Some(fe) = dir_entry_to_file_entry(&entry) {
            out.push(fe);
        }
    }
}

/// Recursively search for entries whose name or path matches the query.
/// Used for on-demand @ mention search — no upfront cap, stops at `limit`.
fn walk_search(dir: &Path, gi: &GitignoreSkip, query_lower: &str, out: &mut Vec<FileEntry>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in read_dir {
        if out.len() >= limit {
            return;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_name = entry.file_name().to_string_lossy().to_string();
        if should_skip_mention(&file_name, gi) {
            continue;
        }
        let path_buf = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Match by filename or full path (case-insensitive)
        if file_name.to_lowercase().contains(query_lower)
            || path_buf.to_string_lossy().to_lowercase().contains(query_lower)
        {
            if let Some(fe) = dir_entry_to_file_entry(&entry) {
                out.push(fe);
            }
        }
        if metadata.is_dir() {
            walk_search(&path_buf, gi, query_lower, out, limit);
        }
    }
}

/// Returns recent modified files + root-level entries for the @ mention picker.
/// Browsing is instant (single dir read); searching is handled separately by
/// `search_mention_files` to avoid the 2000-entry cap issue on large projects.
#[tauri::command]
pub fn list_mention_entries(
    root_path: String,
    recent_limit: Option<usize>,
) -> Result<MentionEntries, String> {
    let root = Path::new(&root_path);
    if !root.exists() || !root.is_dir() {
        return Err(format!("Invalid directory: {}", root_path));
    }
    let limit = recent_limit.unwrap_or(5);

    // Parse .gitignore once so browsing + recent-file walk both honor it.
    let gi = parse_gitignore(root);

    // Root-level entries (fast, non-recursive)
    let root_entries = collect_dir_entries(root, &gi);

    // Recently modified files (recursive walk, files only)
    let mut all_files: Vec<FileEntry> = Vec::new();
    walk_collect_files(root, &gi, &mut all_files);
    all_files.sort_by(|a, b| b.modified.cmp(&a.modified));
    let recent: Vec<FileEntry> = all_files.into_iter().take(limit).collect();

    Ok(MentionEntries {
        recent,
        root: root_entries,
    })
}

/// On-demand search for the @ mention picker. Walks the tree recursively and
/// returns entries whose name or path matches the query (case-insensitive).
#[tauri::command]
pub fn search_mention_files(
    root_path: String,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<FileEntry>, String> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }
    let root = Path::new(&root_path);
    if !root.exists() || !root.is_dir() {
        return Err(format!("Invalid directory: {}", root_path));
    }
    let query_lower = query.to_lowercase();
    let max_results = limit.unwrap_or(100);
    let gi = parse_gitignore(root);
    let mut results = Vec::new();
    walk_search(root, &gi, &query_lower, &mut results, max_results);
    // Sort: directories first, then alphabetically (case-insensitive)
    results.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(results)
}

// ── File Tree Mutations ──────────────────────────────────────────
//
// create_dir / create_file / rename_path are the write-side counterparts to
// list_dir. They power the file tree's right-click menu and "+" toolbar in
// FileTree.vue.
//
// All three accept an absolute `path` for the *target* (so the caller picks
// the exact destination, including the new name). rename_path is also used
// for cross-directory moves — fs::rename handles same-volume moves atomically;
// cross-volume moves fall back to copy+remove via std::fs::rename's documented
// behavior. The caller is responsible for picking a name that doesn't collide.

/// Validate `path` against `root` for safety: both must exist, `path` must
/// resolve to a location inside `root` after canonicalization. Returns the
/// canonicalized path on success.
///
/// This guards against:
///   - `..` traversal in caller-supplied paths
///   - symlinks pointing outside the project root
///   - typos that would land outside the workspace
fn validate_inside_root(path: &Path, root: &Path) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let canonical_root = fs::canonicalize(root)
        .map_err(|e| format!("Invalid root: {}", e))?;
    let canonical_path = fs::canonicalize(path)
        .map_err(|e| format!("Invalid path: {}", e))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(format!(
            "Path '{}' is outside the workspace root",
            canonical_path.display()
        ));
    }
    Ok((canonical_root, canonical_path))
}

/// Create a directory at `path`. Parent directories must already exist
/// (callers resolve the target parent themselves, so this stays predictable).
/// Errors if the path already exists.
#[tauri::command]
pub fn create_dir(path: String, root: String) -> Result<(), String> {
    let p = Path::new(&path);
    if p.exists() {
        return Err(format!("Path already exists: {}", path));
    }
    // Safety: the new dir doesn't exist yet, so validate its parent stays
    // inside the workspace root. This blocks `..` traversal and symlink-escape.
    validate_inside_root(p.parent().unwrap_or(p), Path::new(&root))?;
    fs::create_dir(p).map_err(|e| format!("Failed to create directory: {}", e))
}

/// Create an empty file at `path`. Errors if the path already exists, so the
/// caller can decide whether to overwrite (currently we don't — that should
/// be an explicit user action).
#[tauri::command]
pub fn create_file(path: String, root: String) -> Result<(), String> {
    let p = Path::new(&path);
    if p.exists() {
        return Err(format!("Path already exists: {}", path));
    }
    // Safety: the new file doesn't exist yet, so validate its parent stays
    // inside the workspace root.
    validate_inside_root(p.parent().unwrap_or(p), Path::new(&root))?;
    // touch() semantics: create empty file. fs::OpenOptions::create_new returns
    // an error if the file exists, which matches our "no overwrite" policy.
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)
        .map_err(|e| format!("Failed to create file: {}", e))?;
    // No content to write, but we need to keep `f` alive past the open. A no-op
    // flush makes intent clear without writing any bytes.
    f.flush().map_err(|e| format!("Failed to finalize file: {}", e))?;
    Ok(())
}

/// Rename or move a file/directory from `old_path` to `new_path`. Both paths
/// must live inside the same workspace root (the `root` argument). Atomic
/// within a filesystem via fs::rename.
///
/// Refuses to move a directory into one of its own descendants (would create
/// a cycle). The caller is expected to pre-validate the user's selection, but
/// we re-check here as a safety net.
#[tauri::command]
pub fn rename_path(
    old_path: String,
    new_path: String,
    root: String,
) -> Result<(), String> {
    let old = Path::new(&old_path);
    let new = Path::new(&new_path);
    let root_path = Path::new(&root);

    if !old.exists() {
        return Err(format!("Source does not exist: {}", old_path));
    }
    if new.exists() {
        return Err(format!("Destination already exists: {}", new_path));
    }

    // Path safety: both endpoints must live under the workspace root.
    let (_, canonical_old) = validate_inside_root(old, root_path)?;
    let (_, canonical_new) = validate_inside_root(new.parent().unwrap_or(new), root_path)?;
    // `new` itself doesn't exist yet, so canonicalize the parent. We've
    // already confirmed the parent is inside the root.

    // Cycle check: moving a directory into itself or a descendant would make
    // it unreachable. canonicalize the new path's intended parent + the new
    // name, then check the resulting prefix.
    if canonical_old.is_dir() {
        // The new path will be `canonical_new_parent/new_file_name`.
        // If canonical_old is an ancestor of that, it's a cycle.
        if canonical_new.starts_with(&canonical_old) {
            return Err("Cannot move a directory into itself or its descendant".to_string());
        }
    }

    fs::rename(&canonical_old, new).map_err(|e| format!("Failed to rename: {}", e))
}

/// Permanently delete a file or directory (recursively). The path must live
/// inside the workspace `root`. Deleting is irreversible — the frontend shows
/// a confirmation dialog before calling this.
///
/// Never deletes the workspace root itself, even if asked (guards against a
/// UI bug or a malicious caller wiping the project).
#[tauri::command]
pub fn delete_path(path: String, root: String) -> Result<(), String> {
    let p = Path::new(&path);
    let root_path = Path::new(&root);

    if !p.exists() && !p.symlink_metadata().is_ok() {
        return Err(format!("Path does not exist: {}", path));
    }

    // Detect symlinks with symlink_metadata (does NOT follow the link). A
    // symlink is a small file pointing elsewhere — deleting it must remove the
    // LINK, never the target's contents. So we treat symlinks as files and
    // validate the link's own location (via its parent), not its target.
    let meta = p.symlink_metadata().map_err(|e| format!("Failed to read metadata: {}", e))?;
    let is_symlink = meta.file_type().is_symlink();

    if is_symlink {
        // Validate the symlink's parent dir is inside the root, then remove
        // the link itself with remove_file (which on symlinks removes the link,
        // not the target).
        validate_inside_root(p.parent().unwrap_or(p), root_path)?;
        return fs::remove_file(p).map_err(|e| format!("Failed to delete: {}", e));
    }

    // Regular file or directory: canonicalize (resolves any remaining symlink
    // components in the path) and confirm it stays inside the root.
    let (canonical_root, canonical_path) = validate_inside_root(p, root_path)?;

    // Refuse to delete the workspace root or anything at/above it.
    if canonical_path == canonical_root {
        return Err("Cannot delete the workspace root".to_string());
    }

    if canonical_path.is_dir() {
        fs::remove_dir_all(&canonical_path)
            .map_err(|e| format!("Failed to delete directory: {}", e))
    } else {
        fs::remove_file(&canonical_path)
            .map_err(|e| format!("Failed to delete file: {}", e))
    }
}

// ── File Attachment Parser ──────────────────────────────────────

/// Max file size for parsing (10 MB).
const MAX_PARSE_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Max parsed text length (50,000 chars).
const MAX_PARSE_TEXT_LEN: usize = 50_000;

/// Result of parsing an attached file for the LLM.
#[derive(Debug, Serialize, Clone)]
pub struct ParsedFile {
    pub name: String,
    pub path: String,
    pub content: String,
    pub size: u64,
    pub truncated: bool,
    pub error: Option<String>,
}

/// Parse a file into plain text for LLM context.
/// Supports: txt, md, json, csv, log, yaml, yml, xml, py, js, ts, java, rs, go,
///           html, css, sh, toml, docx, xlsx, xls, pptx, pdf
#[tauri::command]
pub fn parse_file(path: String) -> Result<ParsedFile, String> {
    let file_path = Path::new(&path);
    if !file_path.exists() {
        return Err(format!("File not found: {}", path));
    }
    if !file_path.is_file() {
        return Err("Path is not a file".to_string());
    }

    let name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let metadata = fs::metadata(&path).map_err(|e| format!("Failed to read metadata: {}", e))?;
    let size = metadata.len();

    let ext = file_path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default()
        .to_lowercase();

    // Reject oversized binary formats outright (they'd blow up memory).
    // Plain text is capped below; docx/xlsx/pptx/pdf > 10MB are not worth parsing.
    if size > MAX_PARSE_FILE_SIZE {
        return Ok(ParsedFile {
            name,
            path,
            content: String::new(),
            size,
            truncated: false,
            error: Some(format!(
                "File too large ({:.1} MB, max {} MB) — attach a smaller file",
                size as f64 / (1024.0 * 1024.0),
                MAX_PARSE_FILE_SIZE / (1024 * 1024)
            )),
        });
    }

    let result = match ext.as_str() {
        // ── Plain text files ──────────────────────────────
        "txt" | "md" | "json" | "csv" | "log" | "yaml" | "yml" | "xml"
        | "py" | "js" | "jsx" | "ts" | "tsx" | "java" | "rs" | "go"
        | "html" | "css" | "scss" | "less" | "sh" | "bash" | "toml"
        | "ini" | "cfg" | "conf" | "sql" | "vue" | "rb" | "php"
        | "swift" | "kt" | "scala" | "c" | "cpp" | "h" | "hpp" => {
            parse_plain_text(&path, size)
        }

        // ── Office documents ─────────────────────────────
        "docx" => parse_docx(&path, size),
        "xlsx" | "xls" => parse_xlsx(&path, size),
        "pptx" => parse_pptx(&path, size),

        // ── PDF ──────────────────────────────────────────
        "pdf" => parse_pdf(&path, size),

        // ── Unsupported ──────────────────────────────────
        _ => Err(format!("Unsupported file type: .{}", ext)),
    };

    match result {
        Ok((content, truncated)) => Ok(ParsedFile {
            name,
            path,
            content,
            size,
            truncated,
            error: None,
        }),
        Err(err) => Ok(ParsedFile {
            name,
            path,
            content: String::new(),
            size,
            truncated: false,
            error: Some(err),
        }),
    }
}

/// Truncate text to MAX_PARSE_TEXT_LEN with a note.
fn truncate_text(text: &str) -> (String, bool) {
    if text.chars().count() > MAX_PARSE_TEXT_LEN {
        let truncated: String = text.chars().take(MAX_PARSE_TEXT_LEN).collect();
        (format!("{}\n\n[Content truncated — showing first {} characters]", truncated, MAX_PARSE_TEXT_LEN), true)
    } else {
        (text.to_string(), false)
    }
}

/// Parse plain text files (UTF-8).
fn parse_plain_text(path: &str, _size: u64) -> Result<(String, bool), String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Failed to read file: {}", e))?;
    Ok(truncate_text(&text))
}

/// Parse .docx (Office Open XML) — extract text from word/document.xml.
fn parse_docx(path: &str, _size: u64) -> Result<(String, bool), String> {
    let file = fs::File::open(path).map_err(|e| format!("Failed to open: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Failed to read docx: {}", e))?;

    let mut doc = archive
        .by_name("word/document.xml")
        .map_err(|_| "Not a valid .docx file (missing word/document.xml)".to_string())?;

    let mut xml = String::new();
    doc.read_to_string(&mut xml).map_err(|e| format!("Failed to read document.xml: {}", e))?;

    let text = extract_xml_text(&xml);
    Ok(truncate_text(&text))
}

/// Parse .xlsx / .xls using calamine.
fn parse_xlsx(path: &str, _size: u64) -> Result<(String, bool), String> {
    let mut workbook = calamine::open_workbook_auto(path)
        .map_err(|e| format!("Failed to open spreadsheet: {}", e))?;

    let mut output = String::new();
    let sheet_names = workbook.sheet_names().to_vec();

    for sheet_name in &sheet_names {
        if let Ok(range) = workbook.worksheet_range(sheet_name) {
            output.push_str(&format!("--- Sheet: {} ---\n", sheet_name));
            let mut rows_iter = range.rows();
            let max_rows = 500;
            let mut row_count = 0;

            for row in rows_iter.by_ref() {
                if row_count >= max_rows {
                    output.push_str(&format!("\n[Truncated — showing first {} rows per sheet]\n", max_rows));
                    break;
                }
                let cells: Vec<String> = row.iter().map(|c| c.to_string()).collect();
                output.push_str(&cells.join("\t"));
                output.push('\n');
                row_count += 1;
            }
            output.push('\n');
        }
    }

    if output.is_empty() {
        output = "[Empty spreadsheet]".to_string();
    }

    Ok(truncate_text(&output))
}

/// Parse .pptx — extract text from all slides.
fn parse_pptx(path: &str, _size: u64) -> Result<(String, bool), String> {
    let file = fs::File::open(path).map_err(|e| format!("Failed to open: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Failed to read pptx: {}", e))?;

    let mut output = String::new();
    let mut slide_num = 1;

    loop {
        let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
        match archive.by_name(&slide_path) {
            Ok(mut slide) => {
                let mut xml = String::new();
                slide.read_to_string(&mut xml).map_err(|e| format!("Failed to read slide {}: {}", slide_num, e))?;
                let text = extract_xml_text(&xml);
                if !text.trim().is_empty() {
                    output.push_str(&format!("--- Slide {} ---\n{}\n\n", slide_num, text.trim()));
                }
                slide_num += 1;
            }
            Err(_) => break, // No more slides
        }
    }

    if output.is_empty() {
        output = "[No text content found in slides]".to_string();
    }

    Ok(truncate_text(&output))
}

/// Parse PDF using pdf-extract.
fn parse_pdf(path: &str, _size: u64) -> Result<(String, bool), String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read PDF: {}", e))?;
    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| format!("Failed to extract PDF text: {}", e))?;

    if text.trim().is_empty() {
        return Ok(("[No extractable text in PDF — may be scanned/image-based]".to_string(), false));
    }

    Ok(truncate_text(&text))
}

/// Extract visible text from Office Open XML (docx/pptx) using quick-xml.
/// Collects only the text inside <w:t>/<a:t> elements (local name "t"),
/// inserts a newline at paragraph (<w:p>/<a:p>) and break (<w:br/>/<a:br/>)
/// boundaries so paragraphs stay readable.
fn extract_xml_text(xml: &str) -> String {
    let mut reader = XmlReader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut text = String::new();
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"t" => in_text = true,
                b"p" => {
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == b"br" {
                    text.push('\n');
                }
            }
            Ok(Event::End(_)) => in_text = false,
            Ok(Event::Text(e)) => {
                if in_text {
                    if let Ok(unescaped) = e.unescape() {
                        text.push_str(&unescaped);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break, // malformed XML → return whatever was gathered
            _ => {}
        }
    }

    // Collapse runs of spaces/tabs (keep newlines), then trim.
    let mut result = String::new();
    let mut last_was_space = false;
    for ch in text.chars() {
        if (ch == ' ' || ch == '\t' || ch == '\r') && ch != '\n' {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = ch == ' ';
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_xml_text_docx() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Hello, World!</w:t></w:r></w:p>
    <w:p><w:r><w:t>Second &amp; paragraph with </w:t></w:r><w:r><w:t>two runs</w:t></w:r></w:p>
    <w:p><w:r><w:t xml:space="preserve">Trailing </w:t></w:r><w:r><w:br/><w:t>line break</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let text = extract_xml_text(xml);
        assert!(text.contains("Hello, World!"), "missing first paragraph: {text}");
        assert!(text.contains("Second & paragraph with two runs"), "entity/run concat failed: {text}");
        assert!(text.contains('\n'), "no newlines inserted: {text}");
    }

    #[test]
    fn test_extract_xml_text_pptx() {
        let xml = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cSld><p:spTree>
  <p:sp><p:txBody>
    <a:p><a:r><a:t>Slide Title</a:t></a:r></a:p>
    <a:p><a:r><a:t>Bullet 1</a:t></a:r><a:br/><a:r><a:t>Bullet 2</a:t></a:r></a:p>
  </p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#;
        let text = extract_xml_text(xml);
        assert!(text.contains("Slide Title"), "missing slide title: {text}");
        assert!(text.contains("Bullet 1"), "missing bullet: {text}");
        assert!(text.contains('\n'), "no newlines inserted: {text}");
    }

    #[test]
    fn test_parse_pptx_real_file() {
        let path = "/Users/guizhan/.runjam/session/msmzr14dbcxvp8/outputs/project-management-best-practices.pptx";
        if !std::path::Path::new(path).exists() {
            return; // skip if file unavailable on this machine
        }
        let result = parse_pptx(path, 0);
        assert!(result.is_ok(), "parse_pptx failed: {:?}", result.err());
        let (text, _truncated) = result.unwrap();
        assert!(!text.trim().is_empty(), "extracted text is empty");
        let preview: String = text.chars().take(2000).collect();
        println!("=== PPTX EXTRACTED TEXT ===\n{}", preview);
    }

    // ── File tree mutation tests ────────────────────────────────
    //
    // Use a unique temp directory per test so they can run in parallel and
    // never touch the real workspace. The dir is created at test start and
    // best-effort cleaned up at the end (we don't panic on cleanup failure).

    use std::path::PathBuf;

    /// Build a fresh temp dir for a test. The returned path is guaranteed to
    /// exist and be empty.
    fn make_tmpdir(label: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "runjam-fs-cmd-test-{}-{}",
            label,
            std::process::id()
        ));
        // Each test gets its own subdir so parallel test runs don't collide.
        let unique = base.join(format!(
            "{}-{}",
            label,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&unique).expect("create temp dir");
        unique
    }

    fn best_effort_remove(p: &Path) {
        if p.is_dir() {
            let _ = fs::remove_dir_all(p);
        } else if p.exists() {
            let _ = fs::remove_file(p);
        }
    }

    #[test]
    fn test_create_dir_happy_path() {
        let root = make_tmpdir("create_dir_ok");
        let target = root.join("new_folder");
        let result = create_dir(
            target.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_ok(), "create_dir failed: {:?}", result.err());
        assert!(target.is_dir(), "directory was not actually created");
        best_effort_remove(&root);
    }

    #[test]
    fn test_create_dir_already_exists() {
        let root = make_tmpdir("create_dup");
        let target = root.join("dup");
        fs::create_dir(&target).unwrap();
        let result = create_dir(
            target.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_err(), "expected error when path already exists");
        assert!(
            result.as_ref().err().unwrap().contains("already exists"),
            "error message should mention 'already exists', got: {:?}",
            result
        );
        best_effort_remove(&root);
    }

    #[test]
    fn test_create_file_happy_path() {
        let root = make_tmpdir("create_file_ok");
        let target = root.join("note.txt");
        let result = create_file(
            target.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_ok(), "create_file failed: {:?}", result.err());
        assert!(target.is_file(), "file was not actually created");
        let size = fs::metadata(&target).unwrap().len();
        assert_eq!(size, 0, "newly created file should be empty");
        best_effort_remove(&root);
    }

    #[test]
    fn test_create_file_already_exists() {
        let root = make_tmpdir("create_file_dup");
        let target = root.join("dup.txt");
        fs::write(&target, "existing content").unwrap();
        let result = create_file(
            target.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_err(), "expected error when file already exists");
        // The original content must be preserved — we don't silently overwrite.
        let preserved = fs::read_to_string(&target).unwrap();
        assert_eq!(preserved, "existing content");
        best_effort_remove(&root);
    }

    #[test]
    fn test_create_rejects_escape() {
        let root = make_tmpdir("create_escape");
        // Attempt to create a file OUTSIDE the root. Its parent canonicalizes
        // outside root, so create_file must refuse.
        let outside = std::env::temp_dir().join(format!(
            "runjam-create-escape-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let escape_path = outside.join("evil.txt");

        let result = create_file(
            escape_path.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_err(), "expected escape attempt to be rejected");
        assert!(!escape_path.exists(), "outside file must not be created");

        best_effort_remove(&outside);
        best_effort_remove(&root);
    }

    #[test]
    fn test_rename_simple() {
        let root = make_tmpdir("rename_simple");
        let src = root.join("a.txt");
        let dst = root.join("b.txt");
        fs::write(&src, "hello").unwrap();

        let result = rename_path(
            src.to_string_lossy().to_string(),
            dst.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_ok(), "rename failed: {:?}", result.err());
        assert!(!src.exists(), "source should be gone after rename");
        assert!(dst.is_file(), "destination should exist");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
        best_effort_remove(&root);
    }

    #[test]
    fn test_rename_across_directories() {
        let root = make_tmpdir("rename_xdir");
        let sub_a = root.join("a");
        let sub_b = root.join("b");
        fs::create_dir(&sub_a).unwrap();
        fs::create_dir(&sub_b).unwrap();
        let src = sub_a.join("file.txt");
        let dst = sub_b.join("moved.txt");
        fs::write(&src, "x").unwrap();

        let result = rename_path(
            src.to_string_lossy().to_string(),
            dst.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_ok(), "cross-dir rename failed: {:?}", result.err());
        assert!(!src.exists());
        assert!(dst.is_file());
        best_effort_remove(&root);
    }

    #[test]
    fn test_rename_rejects_cycle() {
        let root = make_tmpdir("rename_cycle");
        let parent = root.join("parent");
        let child = parent.join("child");
        fs::create_dir(&parent).unwrap();
        fs::create_dir(&child).unwrap();

        // Attempt to move `parent` into `child` — that would make parent
        // unreachable (it'd be inside itself).
        let bad_dst = child.join("parent");
        let result = rename_path(
            parent.to_string_lossy().to_string(),
            bad_dst.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_err(), "expected cycle to be rejected");
        assert!(
            result.as_ref().err().unwrap().contains("itself or its descendant"),
            "error should explain the cycle, got: {:?}",
            result
        );
        best_effort_remove(&root);
    }

    #[test]
    fn test_rename_rejects_escape() {
        let root = make_tmpdir("rename_escape");
        // A file *outside* the root — attempt to move it into the root.
        let outside = std::env::temp_dir().join(format!(
            "runjam-outside-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::write(&outside, "outside").unwrap();

        let inside_target = root.join("sneaky.txt");
        let result = rename_path(
            outside.to_string_lossy().to_string(),
            inside_target.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_err(), "expected escape attempt to be rejected");

        best_effort_remove(&outside);
        best_effort_remove(&root);
    }

    #[test]
    fn test_delete_file() {
        let root = make_tmpdir("delete_file");
        let target = root.join("gone.txt");
        fs::write(&target, "bye").unwrap();

        let result = delete_path(
            target.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_ok(), "delete_file failed: {:?}", result.err());
        assert!(!target.exists(), "file should be gone");
        best_effort_remove(&root);
    }

    #[test]
    fn test_delete_directory_recursive() {
        let root = make_tmpdir("delete_dir");
        let dir = root.join("sub");
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("nested").join("a.txt"), "x").unwrap();
        fs::write(dir.join("b.txt"), "y").unwrap();

        let result = delete_path(
            dir.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_ok(), "delete_dir failed: {:?}", result.err());
        assert!(!dir.exists(), "directory tree should be gone");
        best_effort_remove(&root);
    }

    #[test]
    fn test_delete_rejects_root() {
        let root = make_tmpdir("delete_root");
        let result = delete_path(
            root.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_err(), "expected deleting the root to be rejected");
        assert!(
            result.as_ref().err().unwrap().contains("workspace root"),
            "error should mention the root guard, got: {:?}",
            result
        );
        best_effort_remove(&root);
    }

    #[test]
    fn test_delete_rejects_escape() {
        let root = make_tmpdir("delete_escape");
        // A file outside the root — must not be deletable through the guard.
        let outside = std::env::temp_dir().join(format!(
            "runjam-outside-del-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::write(&outside, "outside").unwrap();

        let result = delete_path(
            outside.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_err(), "expected escape attempt to be rejected");
        assert!(outside.exists(), "outside file must be untouched");

        best_effort_remove(&outside);
        best_effort_remove(&root);
    }

    #[test]
    fn test_delete_symlink_removes_link_not_target() {
        let root = make_tmpdir("delete_symlink");
        // A real directory OUTSIDE the root that a symlink points to.
        let real_target = std::env::temp_dir().join(format!(
            "runjam-symlink-target-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(real_target.join("inner")).unwrap();
        fs::write(real_target.join("inner").join("data.txt"), "keep me").unwrap();

        // Symlink inside the root pointing at the outside target.
        let link = root.join("link_to_target");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_target, &link).unwrap();

        let result = delete_path(
            link.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_ok(), "deleting the symlink should succeed: {:?}", result.err());
        // The link itself is gone...
        assert!(!link.symlink_metadata().is_ok(), "symlink should be removed");
        // ...but the target's contents are untouched.
        assert!(
            real_target.join("inner").join("data.txt").exists(),
            "symlink target contents must NOT be deleted"
        );

        best_effort_remove(&real_target);
        best_effort_remove(&root);
    }

    #[test]
    fn test_delete_symlink_to_file_removes_link() {
        let root = make_tmpdir("delete_symlink_file");
        let real_file = std::env::temp_dir().join(format!(
            "runjam-symlink-file-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::write(&real_file, "keep").unwrap();

        let link = root.join("file_link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_file, &link).unwrap();

        let result = delete_path(
            link.to_string_lossy().to_string(),
            root.to_string_lossy().to_string(),
        );
        assert!(result.is_ok(), "deleting symlink should succeed: {:?}", result.err());
        assert!(!link.symlink_metadata().is_ok(), "symlink should be removed");
        assert!(real_file.exists(), "target file must be untouched");

        best_effort_remove(&real_file);
        best_effort_remove(&root);
    }
}
