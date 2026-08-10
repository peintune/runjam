use serde::Serialize;
use std::collections::HashSet;
use std::fs;
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
