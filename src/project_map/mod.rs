//! Generates a single-file snapshot of a Rust project for use as AI context.
//!
//! Ported from the standalone `project-map` binary so web_ai_automation can
//! invoke it in-process during intake without an external dependency.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".cargo-audit-db",
    ".cargo-home",
    ".ci-home",
    ".claude",
    ".copilot_sync",
    ".tmp-test-sockets",
    "target",
    "artifacts",
    "node_modules",
    ".idea",
    ".vscode",
    "tmpcfg",
    "third_party",
];

const EXCLUDED_DIR_PREFIXES: &[&str] = &["test_results"];

const INCLUDED_EXTENSIONS: &[&str] = &[
    "rs",
    "toml",
    "md",
    "txt",
    "json",
    "yaml",
    "yml",
    "ron",
    "gitignore",
    "gitattributes",
    "editorconfig",
    "example",
    // Shell / build scripts — frequently carry security-relevant logic
    // (key handling, networking commands, sudo escalation) that the AI
    // needs to see alongside the Rust sources.
    "sh",
    "bash",
    "zsh",
];

const INCLUDED_FILENAMES: &[&str] = &[
    // NOTE: `Cargo.lock` is intentionally excluded here. It's huge (~500 KB
    // for Rustynet) and rarely useful as raw text. We instead extract a
    // compressed `<name> = <version> (source)` summary into the workspace
    // overview file via `summarise_cargo_lock`.
    "Cargo.toml",
    "README",
    "README.md",
    "LICENSE",
    "LICENSE.md",
    "NOTICE",
    "CHANGELOG",
    "CHANGELOG.md",
    "AGENTS.md",
    "CLAUDE.md",
    "COPILOT.md",
    ".gitignore",
    ".gitattributes",
    ".editorconfig",
    "rust-toolchain",
    "rust-toolchain.toml",
    "rustfmt.toml",
    "clippy.toml",
    ".env.example",
];

struct SourceFile {
    absolute_path: PathBuf,
    relative_path: PathBuf,
    contents: String,
}

#[derive(Default)]
struct TreeNode {
    children: BTreeMap<String, TreeNode>,
}

/// Per-file split threshold. DeepSeek's upload size limit sits around 10 MB
/// — staying under 8 MB leaves headroom for upload encoding and avoids
/// borderline-sized files that flap between accept/reject.
const SPLIT_THRESHOLD_BYTES: usize = 8 * 1024 * 1024;

/// Result of a project-map generation.
#[derive(Debug)]
pub struct GeneratedMap {
    /// Files written. Always at least one path. When the project fits into a
    /// single file this is `[output_path]`. When the project is split per
    /// crate, the first entry is the workspace overview file (root-level
    /// docs + tree + per-crate index) and the remainder are per-crate maps.
    pub paths: Vec<PathBuf>,
    /// Total number of source files collected across all output files.
    pub file_count: usize,
}

/// Scan `root_dir`, collect all relevant Rust project files, and write a
/// snapshot to disk.
///
/// **Single-file mode** (default): everything goes into `output_path`.
///
/// **Split mode** (auto): if the rendered single-file report would exceed
/// [`SPLIT_THRESHOLD_BYTES`] AND the project contains more than one crate,
/// the output is broken into one file per crate plus a workspace overview.
/// File names are derived from `output_path`'s stem (e.g.
/// `session/rust_project_map.txt` → `session/rust_project_map_<crate>.txt`).
pub fn generate(root_dir: &Path, output_path: &Path) -> io::Result<GeneratedMap> {
    let root = fs::canonicalize(root_dir)?;
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a directory: {}", root.display()),
        ));
    }

    let output_abs = if output_path.is_absolute() {
        output_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(output_path)
    };
    let output_abs = {
        // Canonicalize parent but not the file itself (it may not exist yet).
        let parent = output_abs.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        fs::canonicalize(parent)?.join(
            output_abs
                .file_name()
                .unwrap_or(OsStr::new("rust_project_map.txt")),
        )
    };

    let mut files = Vec::new();
    collect_files(&root, &root, &output_abs, &mut files)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let file_count = files.len();

    // Discover crates up front so the graph + per-crate stats are
    // available to BOTH single-file and split-file renderers.
    let crate_roots = discover_crate_roots(&root)?;
    let graph = build_workspace_graph(&crate_roots);
    let provenance = collect_provenance(&root);

    // Decide single-file vs split based on rendered size.
    let single_report = render_report(&root, &files, &graph, &provenance);
    if single_report.len() <= SPLIT_THRESHOLD_BYTES {
        fs::write(&output_abs, single_report)?;
        return Ok(GeneratedMap {
            paths: vec![output_abs],
            file_count,
        });
    }

    if crate_roots.len() <= 1 {
        fs::write(&output_abs, single_report)?;
        return Ok(GeneratedMap {
            paths: vec![output_abs],
            file_count,
        });
    }

    let groups = group_files_by_crate(&files, &crate_roots);
    // Compute project-wide annotations and `unsafe` index once so the
    // workspace overview can reference the WHOLE tree, not just the
    // workspace bucket's docs/configs.
    let all_files: Vec<&SourceFile> = files.iter().collect();
    let project_annotations = render_annotations(&extract_annotations(&all_files));
    let project_unsafe = render_unsafe(&extract_unsafe_blocks(&all_files));
    let paths = write_split_outputs(
        &root,
        &output_abs,
        &groups,
        &graph,
        &crate_roots,
        &provenance,
        project_annotations.as_deref(),
        project_unsafe.as_deref(),
    )?;
    Ok(GeneratedMap { paths, file_count })
}

fn collect_files(
    root: &Path,
    dir: &Path,
    output_abs: &Path,
    files: &mut Vec<SourceFile>,
) -> io::Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if file_type.is_dir() {
            if should_skip_dir(&name) {
                continue;
            }
            collect_files(root, &path, output_abs, files)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let abs = fs::canonicalize(&path)?;
        if abs == output_abs {
            continue;
        }

        if !should_include_file(&path) {
            continue;
        }

        let bytes = fs::read(&path)?;
        if bytes.contains(&0u8) {
            continue;
        }

        let contents = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if is_generated_file(&contents) {
            continue;
        }

        let rel = abs
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| abs.clone());

        files.push(SourceFile {
            absolute_path: abs,
            relative_path: rel,
            contents,
        });
    }

    Ok(())
}

/// True when a source file announces itself as auto-generated and isn't
/// worth shipping verbatim. Looks at the first 30 lines for any of the
/// usual telltales — `@generated`, `Code generated by …`, `DO NOT EDIT`,
/// `automatically generated`. Case-insensitive.
fn is_generated_file(contents: &str) -> bool {
    let header: String = contents
        .lines()
        .take(30)
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    header.contains("@generated")
        || header.contains("code generated by")
        || header.contains("automatically generated")
        || header.contains("auto-generated")
        || header.contains("autogenerated")
        || header.contains("do not edit")
        || header.contains("do not modify")
}

fn should_skip_dir(name: &str) -> bool {
    EXCLUDED_DIRS.contains(&name) || EXCLUDED_DIR_PREFIXES.iter().any(|p| name.starts_with(p))
}

fn should_include_file(path: &Path) -> bool {
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if INCLUDED_FILENAMES.contains(&name) {
        return true;
    }
    path.extension()
        .and_then(OsStr::to_str)
        .map(|ext| INCLUDED_EXTENSIONS.contains(&ext))
        .unwrap_or(false)
}

fn render_report(
    root: &Path,
    files: &[SourceFile],
    graph: &WorkspaceGraph,
    provenance: &CaptureProvenance,
) -> String {
    let mut out = String::new();
    out.push_str("# Rust Project Map\n\nRoot: ");
    out.push_str(&root.display().to_string());
    out.push('\n');
    out.push_str(&render_provenance_block(provenance));
    let total_loc: usize = files.iter().map(|f| line_count(&f.contents)).sum();
    out.push_str(&format!(
        "Files: {}  ·  Total lines of source: {}\n",
        files.len(),
        total_loc
    ));

    // Hoist the workspace-root README to the top so the AI reads
    // human-written orientation before any code.
    let file_refs: Vec<&SourceFile> = files.iter().collect();
    let hoisted_readme = find_readme(&file_refs, None);
    if let Some(readme) = hoisted_readme {
        out.push_str("\n## Workspace README — hoisted\n\n");
        out.push_str("===== ");
        out.push_str(&repo_absolute_path(&readme.relative_path));
        out.push_str(" =====\n\n");
        out.push_str(&dump_with_line_numbers(&readme.contents));
        out.push('\n');
    }

    if !graph.forward.is_empty() {
        out.push_str("\n## Workspace Dependency Graph\n\n");
        out.push_str(&render_workspace_graph(graph));
    }

    if let Some(lock_summary) = summarise_cargo_lock(root) {
        out.push_str("\n## Resolved Dependencies (from Cargo.lock)\n\n");
        out.push_str(&lock_summary);
    }

    let annotations = extract_annotations(&file_refs);
    if let Some(rendered) = render_annotations(&annotations) {
        out.push_str("\n## Project Annotations (TODO / FIXME / SECURITY / SAFETY / HACK)\n\n");
        out.push_str(&rendered);
    }

    let unsafe_hits = extract_unsafe_blocks(&file_refs);
    if let Some(rendered) = render_unsafe(&unsafe_hits) {
        out.push_str("\n## `unsafe` Sites\n\n");
        out.push_str(&rendered);
    }

    out.push_str("\n## Document Tree\n\n");
    out.push_str(&render_tree(files));
    out.push_str("\n\n## Files\n\n");

    let hoisted_path: Option<&Path> = hoisted_readme.map(|f| f.absolute_path.as_path());
    for f in files {
        if Some(f.absolute_path.as_path()) == hoisted_path {
            continue;
        }
        out.push_str("===== ");
        out.push_str(&repo_absolute_path(&f.relative_path));
        out.push_str(" =====\n\n");
        out.push_str(&dump_with_line_numbers(&f.contents));
        out.push('\n');
    }

    out
}

fn line_count(contents: &str) -> usize {
    if contents.is_empty() {
        return 0;
    }
    let mut n = contents.lines().count();
    if !contents.ends_with('\n') {
        // `lines()` drops the trailing-newline-less last line — wait, it
        // doesn't: it produces it. Above is correct without adjustment.
        // (kept guard branch to make the invariant explicit.)
        let _ = &mut n;
    }
    n
}

/// Format a file's path relative to the repo root with a leading `/` and
/// forward-slash separators — i.e. its "absolute repo path". This is more
/// useful for the AI than the local filesystem absolute path (which leaks
/// the user's home directory and is non-portable across machines).
fn repo_absolute_path(relative: &Path) -> String {
    let mut s = String::with_capacity(relative.as_os_str().len() + 1);
    s.push('/');
    let mut first = true;
    for component in relative.components() {
        if !first {
            s.push('/');
        }
        first = false;
        s.push_str(&component.as_os_str().to_string_lossy());
    }
    s
}

/// Lets `render_tree` accept both `&[SourceFile]` (owned) and
/// `&[&SourceFile]` (borrowed) without requiring callers to clone
/// content just to get the path + line count.
trait AsSourceFile {
    fn relative_path(&self) -> &Path;
    fn contents_str(&self) -> &str;
}
impl AsSourceFile for SourceFile {
    fn relative_path(&self) -> &Path {
        &self.relative_path
    }
    fn contents_str(&self) -> &str {
        &self.contents
    }
}
impl AsSourceFile for &SourceFile {
    fn relative_path(&self) -> &Path {
        &self.relative_path
    }
    fn contents_str(&self) -> &str {
        &self.contents
    }
}

fn render_tree<F: AsSourceFile>(files: &[F]) -> String {
    // Build path → (size, summary) so leaf nodes can show line count + a
    // one-line summary pulled from each file's `//!` doc-comment.
    let metadata: std::collections::HashMap<PathBuf, (usize, Option<String>)> = files
        .iter()
        .map(|f| {
            (
                f.relative_path().to_path_buf(),
                (
                    line_count(f.contents_str()),
                    extract_file_summary(f.contents_str()),
                ),
            )
        })
        .collect();
    let mut root = TreeNode::default();
    for f in files {
        insert_path(&mut root, f.relative_path());
    }
    let mut lines = Vec::new();
    render_node_with_metadata(&root, &PathBuf::new(), &metadata, "", &mut lines);
    if lines.is_empty() {
        "(no included files)\n".to_string()
    } else {
        lines.join("\n") + "\n"
    }
}

fn insert_path(node: &mut TreeNode, path: &Path) {
    let mut cur = node;
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy().to_string();
        cur = cur.children.entry(name).or_default();
    }
}

/// A discovered crate root: directory containing a Cargo.toml that has a
/// `[package]` section. Workspace-only Cargo.tomls (no `[package]`) do
/// not count.
struct CrateRoot {
    abs_dir: PathBuf,
    name: String,
    /// All declared dependency names from this crate's Cargo.toml across
    /// every `[dependencies]`-flavoured section (regular, dev, build,
    /// target.*). Filtered later against the set of workspace crate names
    /// to derive the inter-workspace dependency graph.
    declared_deps: Vec<String>,
}

/// Walk `root` for crate roots. Cheap — only reads Cargo.toml files we
/// encounter, skips the same noise dirs as `collect_files`.
fn discover_crate_roots(root: &Path) -> io::Result<Vec<CrateRoot>> {
    let mut out = Vec::new();
    walk_for_crates(root, &mut out)?;
    // Deepest paths first so `group_files_by_crate` picks the most-specific
    // owner when iterating.
    out.sort_by(|a, b| {
        b.abs_dir
            .components()
            .count()
            .cmp(&a.abs_dir.components().count())
    });
    Ok(out)
}

fn walk_for_crates(dir: &Path, out: &mut Vec<CrateRoot>) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if should_skip_dir(&name) {
                    continue;
                }
                let cargo_toml = path.join("Cargo.toml");
                if cargo_toml.is_file() {
                    if let Some(name) = parse_crate_name(&cargo_toml) {
                        let declared_deps = parse_crate_dependencies(&cargo_toml);
                        out.push(CrateRoot {
                            abs_dir: fs::canonicalize(&path).unwrap_or(path.clone()),
                            name,
                            declared_deps,
                        });
                    }
                }
                walk_for_crates(&path, out)?;
            }
        }
    }
    // Also check the root dir itself.
    if dir.parent().is_some() {
        return Ok(());
    }
    let cargo_toml = dir.join("Cargo.toml");
    if cargo_toml.is_file() {
        if let Some(name) = parse_crate_name(&cargo_toml) {
            let declared_deps = parse_crate_dependencies(&cargo_toml);
            out.push(CrateRoot {
                abs_dir: fs::canonicalize(dir).unwrap_or(dir.to_path_buf()),
                name,
                declared_deps,
            });
        }
    }
    Ok(())
}

/// Pull dependency names from every `[dependencies]`-style section of a
/// Cargo.toml: `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`,
/// and any `[target.<cfg>.dependencies]` variant. Simple section-aware
/// text parse — no TOML dep — handles the common cases:
/// - `name = "1.0"`
/// - `name = { version = "1.0", path = "../foo" }`
/// - `name = { workspace = true }`
/// Sub-tables like `[dependencies.foo]` also contribute `foo`.
fn parse_crate_dependencies(cargo_toml: &Path) -> Vec<String> {
    let body = match fs::read_to_string(cargo_toml) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let mut deps = Vec::new();
    let mut in_deps_section = false;
    let mut current_subtable_dep: Option<String> = None;
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            current_subtable_dep = None;
            // Section header: matches anything ending in "dependencies]"
            // or its sub-table form "dependencies.<name>]".
            let header = line.trim_start_matches('[').trim_end_matches(']');
            // Sub-table form: e.g. `dependencies.serde` or
            // `target.'cfg(unix)'.dev-dependencies.foo`
            if let Some((section_part, dep_name)) = split_subtable(header) {
                if section_part.ends_with("dependencies") {
                    in_deps_section = false;
                    current_subtable_dep = Some(dep_name.to_string());
                    continue;
                }
            }
            in_deps_section = header.ends_with("dependencies");
            continue;
        }
        if let Some(name) = current_subtable_dep.take() {
            // We're inside `[dependencies.foo]` — record `foo` once and
            // ignore the rest of its body. The next blank line / section
            // header resets state.
            if !deps.contains(&name) {
                deps.push(name);
            }
            current_subtable_dep = None;
            // Fall through; the line itself is just k=v inside the table.
        }
        if !in_deps_section {
            continue;
        }
        // Lines look like `name = "1.0"` or `name = { ... }`. Take the
        // identifier up to the first '=' or whitespace.
        let name = line
            .split('=')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'');
        if name.is_empty() {
            continue;
        }
        // Skip continuation lines of a multi-line inline table.
        if name.starts_with('}') || name.starts_with('{') || name.starts_with(',') {
            continue;
        }
        // Reasonable crate-name shape: starts with alphanumeric, no spaces.
        if !name
            .chars()
            .next()
            .map(|c| c.is_ascii_alphanumeric())
            .unwrap_or(false)
        {
            continue;
        }
        if name.contains(' ') {
            continue;
        }
        if !deps.contains(&name.to_string()) {
            deps.push(name.to_string());
        }
    }
    deps
}

/// Split a TOML section header like `dependencies.foo` into
/// `("dependencies", "foo")`. Returns `None` for non-subtable headers.
fn split_subtable(header: &str) -> Option<(&str, &str)> {
    let last_dot = header.rfind('.')?;
    let parent = &header[..last_dot];
    let leaf = &header[last_dot + 1..];
    Some((parent, leaf))
}

/// Read a Cargo.toml and pull `name = "..."` from inside `[package]`.
/// Returns `None` for workspace-only manifests (no `[package]`) or
/// malformed files. Simple section-aware text scan — avoids pulling in a
/// TOML parser dep.
fn parse_crate_name(cargo_toml: &Path) -> Option<String> {
    let body = fs::read_to_string(cargo_toml).ok()?;
    let mut in_package = false;
    for raw in body.lines() {
        let line = raw.trim();
        if let Some(stripped) = line.strip_prefix('[') {
            in_package = stripped.starts_with("package]") || stripped.starts_with("package.");
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = line.strip_prefix("name") {
            let after_eq = rest.trim_start().strip_prefix('=')?.trim_start();
            // Strip surrounding quotes and trailing comments.
            let val = after_eq
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(|c: char| c == '"' || c == '\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Split-output grouping: every collected file lands in exactly one bucket.
/// Files under multiple crate roots go to the deepest (most specific) one.
/// Files outside every crate go to the synthetic "_workspace" bucket.
struct CrateGroup<'a> {
    name: String,
    files: Vec<&'a SourceFile>,
}

fn group_files_by_crate<'a>(
    files: &'a [SourceFile],
    crate_roots: &[CrateRoot],
) -> Vec<CrateGroup<'a>> {
    use std::collections::HashMap;
    let mut buckets: HashMap<String, Vec<&'a SourceFile>> = HashMap::new();
    let workspace_key = "_workspace".to_string();

    for f in files {
        // crate_roots is sorted deepest-first → first match wins.
        let owner = crate_roots
            .iter()
            .find(|c| f.absolute_path.starts_with(&c.abs_dir))
            .map(|c| c.name.clone())
            .unwrap_or_else(|| workspace_key.clone());
        buckets.entry(owner).or_default().push(f);
    }

    let mut out: Vec<CrateGroup> = buckets
        .into_iter()
        .map(|(name, files)| CrateGroup { name, files })
        .collect();
    // Workspace bucket first (overview), then crates alphabetical for
    // deterministic ordering across runs.
    out.sort_by(|a, b| {
        let aw = a.name == workspace_key;
        let bw = b.name == workspace_key;
        match (aw, bw) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        }
    });
    out
}

/// Write one file per crate group. Returns the list of written paths, with
/// the workspace overview first if present.
fn write_split_outputs(
    root: &Path,
    base_output: &Path,
    groups: &[CrateGroup<'_>],
    graph: &WorkspaceGraph,
    crate_roots: &[CrateRoot],
    provenance: &CaptureProvenance,
    project_annotations: Option<&str>,
    project_unsafe: Option<&str>,
) -> io::Result<Vec<PathBuf>> {
    let parent = base_output.parent().unwrap_or(Path::new("."));
    let stem = base_output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("rust_project_map");
    let ext = base_output
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("txt");

    let mut written = Vec::with_capacity(groups.len());
    for g in groups {
        let safe: String = g
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = parent.join(format!("{stem}_{safe}.{ext}"));
        let crate_dir = crate_roots
            .iter()
            .find(|c| c.name == g.name)
            .map(|c| c.abs_dir.as_path());
        let report = render_group_report(
            root,
            &g.name,
            &g.files,
            graph,
            crate_dir,
            provenance,
            project_annotations,
            project_unsafe,
        );
        fs::write(&path, report)?;
        written.push(path);
    }
    Ok(written)
}

fn render_group_report(
    root: &Path,
    group_name: &str,
    files: &[&SourceFile],
    graph: &WorkspaceGraph,
    crate_dir: Option<&Path>,
    provenance: &CaptureProvenance,
    project_annotations: Option<&str>,
    project_unsafe: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str("# Rust Project Map — ");
    out.push_str(group_name);
    out.push_str("\n\nRoot: ");
    out.push_str(&root.display().to_string());
    out.push('\n');
    out.push_str(&render_provenance_block(provenance));

    let file_count = files.len();
    let total_loc: usize = files.iter().map(|f| line_count(&f.contents)).sum();
    out.push_str(&format!(
        "Files: {file_count}  ·  Lines of source: {total_loc}\n"
    ));
    if group_name == "_workspace" {
        if !graph.forward.is_empty() {
            out.push_str("\n## Workspace Dependency Graph\n\n");
            out.push_str(&render_workspace_graph(graph));
        }
        if let Some(lock_summary) = summarise_cargo_lock(root) {
            out.push_str("\n## Resolved Dependencies (from Cargo.lock)\n\n");
            out.push_str(&lock_summary);
        }
        // Annotations + `unsafe` are project-wide indexes, not "files in
        // this bucket only" — caller passes them in pre-computed.
        if let Some(rendered) = project_annotations.as_ref() {
            out.push_str("\n## Project Annotations (TODO / FIXME / SECURITY / SAFETY / HACK)\n\n");
            out.push_str(rendered);
        }
        if let Some(rendered) = project_unsafe.as_ref() {
            out.push_str("\n## `unsafe` Sites\n\n");
            out.push_str(rendered);
        }
    } else {
        let empty: Vec<String> = Vec::new();
        let deps = graph.forward.get(group_name).unwrap_or(&empty);
        let parents = graph.reverse.get(group_name).unwrap_or(&empty);
        out.push_str("\n## Crate Dependencies (within this workspace)\n\n");
        if deps.is_empty() {
            out.push_str("Depends on (workspace): (none)\n");
        } else {
            out.push_str(&format!("Depends on (workspace): {}\n", deps.join(", ")));
        }
        if parents.is_empty() {
            out.push_str("Depended on by (workspace): (none — top-level crate)\n");
        } else {
            out.push_str(&format!(
                "Depended on by (workspace): {}\n",
                parents.join(", ")
            ));
        }
    }

    // README hoisting — top of the file dump.
    let hoisted_readme = find_readme(files, crate_dir);
    if let Some(readme) = hoisted_readme {
        out.push_str("\n## README — hoisted\n\n");
        out.push_str("===== ");
        out.push_str(&repo_absolute_path(&readme.relative_path));
        out.push_str(" =====\n\n");
        out.push_str(&dump_with_line_numbers(&readme.contents));
        out.push('\n');
    }

    // Module hierarchy (only for crate splits, not the workspace bucket).
    if let Some(dir) = crate_dir {
        if let Some(tree) = build_module_tree(dir, files) {
            out.push_str("\n## Module Hierarchy\n\n");
            out.push_str(&tree);
            out.push('\n');
        }
    }

    // Public API surface — every `pub` item across the crate's .rs files.
    if let Some(api) = render_public_api(files) {
        out.push_str("\n## Public API Surface\n\n");
        out.push_str(&api);
    }

    out.push_str("\n## Document Tree\n\n");
    out.push_str(&render_tree(files));
    out.push_str("\n\n## Files\n\n");
    let hoisted_path: Option<&Path> = hoisted_readme.map(|f| f.absolute_path.as_path());

    // Hoist lib.rs / main.rs ahead of alphabetical so the AI reads the
    // entry point before any other file in the crate.
    let mut ordered: Vec<&&SourceFile> = files.iter().collect();
    ordered.sort_by(|a, b| {
        let entry_rank = |f: &&SourceFile| -> u8 {
            let p = f.relative_path.to_string_lossy();
            if p.ends_with("/lib.rs") || p.ends_with("/main.rs") {
                0
            } else {
                1
            }
        };
        let ra = entry_rank(a);
        let rb = entry_rank(b);
        match ra.cmp(&rb) {
            std::cmp::Ordering::Equal => a.relative_path.cmp(&b.relative_path),
            other => other,
        }
    });
    for f in ordered {
        if Some(f.absolute_path.as_path()) == hoisted_path {
            continue;
        }
        out.push_str("===== ");
        out.push_str(&repo_absolute_path(&f.relative_path));
        out.push_str(" =====\n\n");
        out.push_str(&dump_with_line_numbers(&f.contents));
        out.push('\n');
    }
    out
}

/// Snapshot of git provenance for the captured tree. Used as a header on
/// every output file so the AI can ground its analysis: "this is commit
/// abc1234 on branch main, captured at <ts>, with N uncommitted files".
struct CaptureProvenance {
    captured_at: String,
    git: Option<GitInfo>,
}

struct GitInfo {
    head: String,
    branch: String,
    dirty_files: usize,
}

fn collect_provenance(root: &Path) -> CaptureProvenance {
    let captured_at = chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();
    CaptureProvenance {
        captured_at,
        git: collect_git_info(root),
    }
}

fn collect_git_info(root: &Path) -> Option<GitInfo> {
    let head = run_git(root, &["rev-parse", "--short=12", "HEAD"])?;
    // `branch --show-current` returns empty on detached HEAD — that's
    // fine, we just emit "(detached)".
    let branch_raw = run_git(root, &["branch", "--show-current"]).unwrap_or_default();
    let branch = if branch_raw.is_empty() {
        "(detached)".to_string()
    } else {
        branch_raw
    };
    let porcelain = run_git(root, &["status", "--porcelain"]).unwrap_or_default();
    let dirty_files = if porcelain.is_empty() {
        0
    } else {
        porcelain.lines().count()
    };
    Some(GitInfo {
        head,
        branch,
        dirty_files,
    })
}

fn run_git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn render_provenance_block(prov: &CaptureProvenance) -> String {
    let mut s = String::new();
    s.push_str(&format!("Captured: {}\n", prov.captured_at));
    if let Some(g) = &prov.git {
        s.push_str(&format!(
            "Git: branch `{}` @ {}  ·  uncommitted files: {}\n",
            g.branch, g.head, g.dirty_files
        ));
        if g.dirty_files > 0 {
            s.push_str(
                "Note: working tree was dirty at capture — some files in this map may not match the named commit.\n",
            );
        }
    } else {
        s.push_str("Git: (not a git repo or git unavailable)\n");
    }
    s
}

/// Find the most prominent README file in a set of files, case-insensitive.
/// Prefers `README.md` over plain `README`.
fn find_readme<'a>(files: &[&'a SourceFile], crate_root: Option<&Path>) -> Option<&'a SourceFile> {
    let mut best: Option<(u8, &SourceFile)> = None;
    for f in files {
        // Only consider READMEs that live at the crate's root, not deep
        // sub-directory READMEs that happen to be in this group.
        if let Some(root) = crate_root {
            if let Ok(rel) = f.absolute_path.strip_prefix(root) {
                if rel.components().count() != 1 {
                    continue;
                }
            }
        } else if f.relative_path.components().count() != 1 {
            // Workspace bucket: only top-level README counts.
            continue;
        }
        let name = f
            .relative_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let priority: u8 = match name.as_str() {
            "readme.md" => 3,
            "readme" => 2,
            "readme.txt" => 1,
            _ => 0,
        };
        if priority == 0 {
            continue;
        }
        match best {
            Some((p, _)) if p >= priority => {}
            _ => best = Some((priority, f)),
        }
    }
    best.map(|(_, f)| f)
}

/// Parse `mod foo;` / `pub mod foo;` declarations from a Rust file.
/// Inline `mod foo { ... }` blocks are skipped — they don't map to a
/// separate file in the module tree.
fn parse_mod_declarations(contents: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in contents.lines() {
        let line = raw.trim();
        // Strip leading `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`.
        let after_vis = if let Some(rest) = line.strip_prefix("pub") {
            rest.trim_start_matches(|c: char| c == ' ' || c == '\t')
                .trim_start_matches(|c: char| c == '(')
                .splitn(2, ')')
                .nth(1)
                .map(|s| s.trim_start())
                .unwrap_or_else(|| {
                    // No `(...)` — strip plain `pub `
                    line.trim_start_matches("pub").trim_start()
                })
        } else {
            line
        };
        let rest = match after_vis.strip_prefix("mod") {
            Some(r) if r.starts_with(' ') || r.starts_with('\t') => r.trim_start(),
            _ => continue,
        };
        // We want declarations ending in `;` — file-backed mods. Skip
        // inline `mod foo { ... }` blocks.
        let semi_pos = match rest.find(';') {
            Some(p) => p,
            None => continue,
        };
        let brace_pos = rest.find('{');
        if let Some(b) = brace_pos {
            if b < semi_pos {
                continue;
            }
        }
        let name = rest[..semi_pos].trim();
        if name.is_empty() {
            continue;
        }
        // Strip any attribute-like `#[cfg(...)]` that ended up in name (defensive).
        if name.starts_with('#') {
            continue;
        }
        if !name
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        {
            continue;
        }
        out.push(name.to_string());
    }
    out
}

/// Build a tree of module relationships starting from a crate's entry
/// point (`lib.rs` or `main.rs`). Each declared `mod foo;` is resolved by
/// looking for either `<dir>/foo.rs` or `<dir>/foo/mod.rs`. Modules
/// without a matching file are still listed so the AI can spot stale
/// declarations.
fn build_module_tree<'a>(crate_dir: &Path, files: &[&'a SourceFile]) -> Option<String> {
    // Find lib.rs first, then main.rs as fallback.
    let entry = files
        .iter()
        .find(|f| {
            f.absolute_path
                .strip_prefix(crate_dir)
                .ok()
                .and_then(|r| r.to_str())
                .map(|p| p == "src/lib.rs")
                .unwrap_or(false)
        })
        .or_else(|| {
            files.iter().find(|f| {
                f.absolute_path
                    .strip_prefix(crate_dir)
                    .ok()
                    .and_then(|r| r.to_str())
                    .map(|p| p == "src/main.rs")
                    .unwrap_or(false)
            })
        })?;

    // Build a fast lookup of every file in the crate by its absolute path.
    let by_path: std::collections::HashMap<&Path, &SourceFile> = files
        .iter()
        .map(|f| (f.absolute_path.as_path(), *f))
        .collect();

    let mut lines = Vec::new();
    let entry_label = entry.relative_path.to_string_lossy().to_string();
    lines.push(format!("crate root ({entry_label})"));
    walk_module(entry, &by_path, "", true, &mut lines);
    if lines.len() == 1 {
        // Just the entry point, no submodules — not worth a section.
        return None;
    }
    Some(lines.join("\n"))
}

fn walk_module(
    file: &SourceFile,
    by_path: &std::collections::HashMap<&Path, &SourceFile>,
    prefix: &str,
    is_root: bool,
    lines: &mut Vec<String>,
) {
    let mods = parse_mod_declarations(&file.contents);
    let dir = if is_root {
        // Top-level: child files live next to lib.rs / main.rs (i.e. src/)
        file.absolute_path.parent().map(Path::to_path_buf)
    } else {
        // For child modules at `src/foo.rs`, children live in `src/foo/`.
        // For `src/foo/mod.rs`, children live in `src/foo/`.
        let stem = file
            .absolute_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if stem == "mod" {
            file.absolute_path.parent().map(Path::to_path_buf)
        } else {
            file.absolute_path.parent().map(|p| p.join(stem))
        }
    };
    for (i, m) in mods.iter().enumerate() {
        let last = i + 1 == mods.len();
        let branch = if last { "`-- " } else { "|-- " };
        let next_prefix = if last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}|   ")
        };
        // Resolve `mod foo;` to `<dir>/foo.rs` or `<dir>/foo/mod.rs`.
        let mut resolved: Option<&&SourceFile> = None;
        if let Some(d) = dir.as_ref() {
            let cand_a = d.join(format!("{m}.rs"));
            let cand_b = d.join(m).join("mod.rs");
            resolved = by_path
                .get(cand_a.as_path())
                .or_else(|| by_path.get(cand_b.as_path()));
        }
        match resolved {
            Some(child) => {
                let rel = child.relative_path.to_string_lossy();
                lines.push(format!("{prefix}{branch}{m}  ({rel})"));
                walk_module(child, by_path, &next_prefix, false, lines);
            }
            None => {
                lines.push(format!(
                    "{prefix}{branch}{m}  (declared but file not in map)"
                ));
            }
        }
    }
}

/// Extract one-line public-API signatures from a Rust source file. Picks
/// up `pub fn`, `pub struct`, `pub enum`, `pub trait`, `pub type`,
/// `pub const`, `pub static`, `pub mod` declarations. Truncates each
/// signature at the first `{` or `;` (or end-of-line) so bodies are
/// dropped. Returns `(line_number, one_line_signature)` pairs.
fn extract_public_signatures(contents: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let kinds = [
        "fn ", "struct ", "enum ", "trait ", "type ", "const ", "static ", "mod ",
        "use ", // pub use re-exports — also part of public API
    ];
    for (idx, raw) in contents.lines().enumerate() {
        let trimmed = raw.trim_start();
        // Match `pub fn`, `pub(crate) fn`, `pub(super) fn`, etc. but NOT
        // `pub(in self)` or non-pub items.
        let after_vis = if let Some(rest) = trimmed.strip_prefix("pub") {
            // Skip optional visibility scope `(crate)` / `(super)` etc.
            let r = rest.trim_start();
            if let Some(close_idx) = r.strip_prefix('(').and_then(|inner| inner.find(')')) {
                // `pub(...)` — the visibility is restricted; still public
                // outside the restricted scope so include it.
                r[1..]
                    .split_at(close_idx)
                    .1
                    .trim_start_matches(')')
                    .trim_start()
            } else {
                r
            }
        } else {
            continue;
        };
        let kind = kinds.iter().find(|k| after_vis.starts_with(*k));
        let Some(_kind) = kind else {
            continue;
        };
        // Don't capture `pub use std::...` (re-exporting external paths is
        // noise); only capture re-exports that look like `pub use crate::`
        // or `pub use self::`.
        if after_vis.starts_with("use ") {
            let body = &after_vis[4..];
            if !body.starts_with("crate::")
                && !body.starts_with("self::")
                && !body.starts_with("super::")
            {
                continue;
            }
        }
        // Drop the body — cut at the first `{` or `;` or `where`.
        let mut sig: &str = trimmed;
        for cut in ['{', ';'] {
            if let Some(p) = sig.find(cut) {
                sig = &sig[..p];
                break;
            }
        }
        // Collapse whitespace and trim.
        let sig = sig
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
        if sig.is_empty() {
            continue;
        }
        out.push((idx + 1, sig));
    }
    out
}

fn render_public_api(files: &[&SourceFile]) -> Option<String> {
    let mut by_file: Vec<(String, Vec<(usize, String)>)> = Vec::new();
    for f in files {
        if f.relative_path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let sigs = extract_public_signatures(&f.contents);
        if sigs.is_empty() {
            continue;
        }
        by_file.push((repo_absolute_path(&f.relative_path), sigs));
    }
    if by_file.is_empty() {
        return None;
    }
    // Sort: lib.rs / main.rs first (entry points), then alphabetical.
    by_file.sort_by(|a, b| {
        let entry_a = a.0.ends_with("/lib.rs") || a.0.ends_with("/main.rs");
        let entry_b = b.0.ends_with("/lib.rs") || b.0.ends_with("/main.rs");
        match (entry_a, entry_b) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.0.cmp(&b.0),
        }
    });
    let mut s = String::new();
    let total: usize = by_file.iter().map(|(_, sigs)| sigs.len()).sum();
    s.push_str(&format!(
        "{} public items across {} file(s).\n\n",
        total,
        by_file.len()
    ));
    for (path, sigs) in &by_file {
        s.push_str(path);
        s.push('\n');
        for (line, sig) in sigs {
            s.push_str(&format!("  L{:>5}  {sig}\n", line));
        }
        s.push('\n');
    }
    Some(s)
}

/// Find the byte index of a real `//` or `/*` comment marker on a line,
/// ignoring sequences that appear inside a `"…"` or `'…'` string
/// literal. Returns the index of the first character of the marker.
/// Heuristic — doesn't handle raw strings or `r"…"#` perfectly, but
/// good enough to filter the worst false positives where annotation
/// tags appear inside string literals in test fixtures or docs.
fn locate_comment_start(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut in_dq = false; // inside double-quoted string
    let mut in_sq = false; // inside single-quoted char/string
    while i + 1 < bytes.len() {
        let c = bytes[i];
        if c == b'\\' && (in_dq || in_sq) {
            // Skip escape sequence — including `\"` etc.
            i += 2;
            continue;
        }
        if c == b'"' && !in_sq {
            in_dq = !in_dq;
            i += 1;
            continue;
        }
        if c == b'\'' && !in_dq {
            in_sq = !in_sq;
            i += 1;
            continue;
        }
        if !in_dq && !in_sq {
            if c == b'/' && (bytes[i + 1] == b'/' || bytes[i + 1] == b'*') {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Annotation tags we hunt for across all source files. Order matters:
/// it controls how the AI sees them in the rendered listing (highest-
/// signal first).
const ANNOTATION_TAGS: &[&str] = &[
    "SECURITY", "SAFETY", "FIXME", "TODO", "HACK", "XXX", "BUG", "PANIC", "UNSAFE",
];

/// One match of an annotation comment, captured for the workspace index.
struct AnnotationHit {
    tag: &'static str,
    repo_path: String,
    line: usize,
    text: String,
}

fn extract_annotations(files: &[&SourceFile]) -> Vec<AnnotationHit> {
    let mut out = Vec::new();
    for f in files {
        for (idx, raw) in f.contents.lines().enumerate() {
            // Find a comment marker that is NOT inside a string literal.
            // Skip the false-positive case `"// TODO inside a string"`.
            let Some(comment_start) = locate_comment_start(raw) else {
                continue;
            };
            let body = &raw[comment_start + 2..];
            let body_trim = body.trim_start();
            // Match a tag at the START of the comment body, optionally
            // followed by `:` or `(` or `!`. Avoids matching the literal
            // string "TODO" inside prose.
            for tag in ANNOTATION_TAGS {
                let upper = body_trim.to_ascii_uppercase();
                if upper.starts_with(tag) {
                    let after_tag = &body_trim[tag.len()..];
                    let separator_ok = after_tag.is_empty()
                        || after_tag.starts_with(':')
                        || after_tag.starts_with('(')
                        || after_tag.starts_with('!')
                        || after_tag.starts_with(' ');
                    if separator_ok {
                        out.push(AnnotationHit {
                            tag,
                            repo_path: repo_absolute_path(&f.relative_path),
                            line: idx + 1,
                            text: body_trim.trim_end().to_string(),
                        });
                        break;
                    }
                }
            }
        }
    }
    // Sort: tag priority order (matches ANNOTATION_TAGS), then path, then line.
    out.sort_by(|a, b| {
        let ai = ANNOTATION_TAGS
            .iter()
            .position(|t| *t == a.tag)
            .unwrap_or(usize::MAX);
        let bi = ANNOTATION_TAGS
            .iter()
            .position(|t| *t == b.tag)
            .unwrap_or(usize::MAX);
        ai.cmp(&bi)
            .then_with(|| a.repo_path.cmp(&b.repo_path))
            .then_with(|| a.line.cmp(&b.line))
    });
    out
}

fn render_annotations(hits: &[AnnotationHit]) -> Option<String> {
    if hits.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!(
        "{} comment annotations across the project. Each entry is the tag the project itself attached to that line:\n\n",
        hits.len()
    ));
    let max_tag = ANNOTATION_TAGS.iter().map(|t| t.len()).max().unwrap_or(0);
    for h in hits {
        // Truncate very long comments so the index stays readable; the
        // AI can read the file directly for full context.
        let text = if h.text.len() > 240 {
            let mut s = h.text.chars().take(237).collect::<String>();
            s.push_str("...");
            s
        } else {
            h.text.clone()
        };
        out.push_str(&format!(
            "  {tag:<width$}  {path}:{line}  {text}\n",
            tag = h.tag,
            width = max_tag,
            path = h.repo_path,
            line = h.line,
            text = text,
        ));
    }
    Some(out)
}

/// Locate every `unsafe { … }`, `unsafe fn`, `unsafe trait`, and `unsafe impl`
/// in the project's `.rs` files. Returns one entry per occurrence with
/// repo path, line, and the head of the unsafe construct (truncated).
struct UnsafeHit {
    repo_path: String,
    line: usize,
    head: String,
}

fn extract_unsafe_blocks(files: &[&SourceFile]) -> Vec<UnsafeHit> {
    let mut out = Vec::new();
    for f in files {
        if f.relative_path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        for (idx, raw) in f.contents.lines().enumerate() {
            let trimmed = raw.trim_start();
            // Match: `unsafe {`, `unsafe fn`, `unsafe trait`, `unsafe impl`,
            // and `pub unsafe …` variants. Skip false positives like
            // `// unsafe …` in comments and `unsafe_…_name` identifiers.
            let candidate = trimmed
                .strip_prefix("pub ")
                .unwrap_or(trimmed)
                .strip_prefix("pub(crate) ")
                .or_else(|| trimmed.strip_prefix("pub(crate) "))
                .unwrap_or_else(|| trimmed.strip_prefix("pub ").unwrap_or(trimmed));
            // Comments
            if candidate.starts_with("//") || candidate.starts_with("/*") {
                continue;
            }
            let after = if let Some(rest) = candidate.strip_prefix("unsafe") {
                rest
            } else {
                continue;
            };
            // Guard against `unsafe_*` identifiers.
            let next = after.chars().next();
            let valid = matches!(next, Some(' ') | Some('\t') | Some('{') | None);
            if !valid {
                continue;
            }
            let head = trimmed.trim_end().to_string();
            let head = if head.len() > 200 {
                let mut s = head.chars().take(197).collect::<String>();
                s.push_str("...");
                s
            } else {
                head
            };
            out.push(UnsafeHit {
                repo_path: repo_absolute_path(&f.relative_path),
                line: idx + 1,
                head,
            });
        }
    }
    out.sort_by(|a, b| {
        a.repo_path
            .cmp(&b.repo_path)
            .then_with(|| a.line.cmp(&b.line))
    });
    out
}

fn render_unsafe(hits: &[UnsafeHit]) -> Option<String> {
    if hits.is_empty() {
        return Some(
            "(no `unsafe` blocks, fns, traits, or impls were found in any .rs file in this map)\n"
                .to_string(),
        );
    }
    let mut out = String::new();
    out.push_str(&format!(
        "{} `unsafe` site(s) in the project. Each line is the literal source line where `unsafe` appears:\n\n",
        hits.len()
    ));
    for h in hits {
        out.push_str(&format!(
            "  {path}:{line}  {head}\n",
            path = h.repo_path,
            line = h.line,
            head = h.head,
        ));
    }
    Some(out)
}

/// Parse a workspace-root `Cargo.lock` and emit a deduped package table.
/// Drops the `dependencies = [...]` arrays that bloat the file — the AI
/// can read each crate's `Cargo.toml` for direct deps when it needs to.
fn summarise_cargo_lock(root: &Path) -> Option<String> {
    let body = fs::read_to_string(root.join("Cargo.lock")).ok()?;
    // `[[package]]` blocks contain `name = "…"`, `version = "…"`,
    // optionally `source = "…"`. Cargo writes them in deterministic order.
    let mut entries: Vec<(String, String, Option<String>)> = Vec::new();
    let mut in_pkg = false;
    let mut name = String::new();
    let mut version = String::new();
    let mut source: Option<String> = None;
    let flush = |entries: &mut Vec<(String, String, Option<String>)>,
                 name: &mut String,
                 version: &mut String,
                 source: &mut Option<String>| {
        if !name.is_empty() && !version.is_empty() {
            entries.push((std::mem::take(name), std::mem::take(version), source.take()));
        } else {
            name.clear();
            version.clear();
            *source = None;
        }
    };
    for raw in body.lines() {
        let line = raw.trim();
        if line == "[[package]]" {
            flush(&mut entries, &mut name, &mut version, &mut source);
            in_pkg = true;
            continue;
        }
        if line.starts_with('[') {
            flush(&mut entries, &mut name, &mut version, &mut source);
            in_pkg = false;
            continue;
        }
        if !in_pkg {
            continue;
        }
        let parse_str = |val: &str| -> String { val.trim().trim_matches('"').to_string() };
        if let Some(v) = line.strip_prefix("name = ") {
            name = parse_str(v);
        } else if let Some(v) = line.strip_prefix("version = ") {
            version = parse_str(v);
        } else if let Some(v) = line.strip_prefix("source = ") {
            source = Some(parse_str(v));
        }
    }
    flush(&mut entries, &mut name, &mut version, &mut source);
    if entries.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!(
        "{} resolved packages from `Cargo.lock` (raw lockfile not included to save space).\n\n",
        entries.len()
    ));
    let max_name = entries.iter().map(|(n, _, _)| n.len()).max().unwrap_or(0);
    let max_ver = entries.iter().map(|(_, v, _)| v.len()).max().unwrap_or(0);
    for (n, v, s) in &entries {
        match s {
            Some(src) => out.push_str(&format!(
                "  {n:<nw$}  {v:<vw$}  {src}\n",
                nw = max_name,
                vw = max_ver
            )),
            None => out.push_str(&format!(
                "  {n:<nw$}  {v:<vw$}  (workspace member)\n",
                nw = max_name,
                vw = max_ver
            )),
        }
    }
    Some(out)
}

/// Inter-workspace dependency edges. `forward[crate]` lists workspace
/// crates `crate` depends on; `reverse[crate]` lists workspace crates
/// that depend on `crate`. Sorted, deduped.
struct WorkspaceGraph {
    forward: BTreeMap<String, Vec<String>>,
    reverse: BTreeMap<String, Vec<String>>,
}

fn build_workspace_graph(crates: &[CrateRoot]) -> WorkspaceGraph {
    let known: std::collections::HashSet<&str> = crates.iter().map(|c| c.name.as_str()).collect();
    let mut forward: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut reverse: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Seed every known crate with empty entries so the graph lists even
    // crates that have no inter-workspace edges.
    for c in crates {
        forward.entry(c.name.clone()).or_default();
        reverse.entry(c.name.clone()).or_default();
    }
    for c in crates {
        for dep in &c.declared_deps {
            if !known.contains(dep.as_str()) || dep == &c.name {
                continue;
            }
            let f = forward.entry(c.name.clone()).or_default();
            if !f.contains(dep) {
                f.push(dep.clone());
            }
            let r = reverse.entry(dep.clone()).or_default();
            if !r.contains(&c.name) {
                r.push(c.name.clone());
            }
        }
    }
    for v in forward.values_mut() {
        v.sort();
    }
    for v in reverse.values_mut() {
        v.sort();
    }
    WorkspaceGraph { forward, reverse }
}

/// Render the inter-workspace dep graph as a tree-style listing for the
/// `_workspace` overview file. Roots (crates nothing else depends on)
/// are listed first, then their dependents follow.
fn render_workspace_graph(graph: &WorkspaceGraph) -> String {
    if graph.forward.is_empty() {
        return "(no crates discovered)\n".to_string();
    }
    let mut out = String::new();
    out.push_str("Format: `<crate>  →  <inter-workspace dependencies>`\n\n");
    let max_name = graph.forward.keys().map(|k| k.len()).max().unwrap_or(0);
    for (name, deps) in &graph.forward {
        if deps.is_empty() {
            out.push_str(&format!(
                "  {name:<width$}  (no workspace deps)\n",
                width = max_name
            ));
        } else {
            out.push_str(&format!(
                "  {name:<width$}  → {}\n",
                deps.join(", "),
                width = max_name
            ));
        }
    }
    out.push('\n');
    out.push_str("Reverse dependencies (who depends on each crate):\n\n");
    for (name, parents) in &graph.reverse {
        if parents.is_empty() {
            out.push_str(&format!(
                "  {name:<width$}  (top-level — no workspace dependents)\n",
                width = max_name
            ));
        } else {
            out.push_str(&format!(
                "  {name:<width$}  ← {}\n",
                parents.join(", "),
                width = max_name
            ));
        }
    }
    out
}

/// Embed a file's contents in the dump with line-number prefixes. Each
/// line becomes `   42: <text>`. Width is sized to the file's largest
/// line number so the colons line up. Empty trailing lines are
/// preserved. The resulting block always ends with `\n`.
fn dump_with_line_numbers(contents: &str) -> String {
    if contents.is_empty() {
        return String::new();
    }
    let line_count = contents.lines().count().max(
        // `lines()` drops a trailing newline; account for files that
        // legitimately end with a non-empty last line.
        if contents.ends_with('\n') { 0 } else { 1 },
    );
    let width = number_width(line_count);
    let mut out = String::with_capacity(contents.len() + line_count * (width + 2));
    for (idx, line) in contents.lines().enumerate() {
        out.push_str(&format!("{:>width$}: {line}\n", idx + 1, width = width));
    }
    out
}

fn number_width(n: usize) -> usize {
    let mut n = n;
    let mut w = 1;
    while n >= 10 {
        n /= 10;
        w += 1;
    }
    w.max(4) // Minimum 4 so small files still align with prefix
}

/// Pull the first line of a file's file-level doc-comment (`//!` for
/// Rust, `# `-prefixed first non-empty line for shell scripts and
/// markdown headers) as a one-line summary. Returns `None` when the
/// file doesn't open with anything that looks like a useful summary.
fn extract_file_summary(contents: &str) -> Option<String> {
    let mut iter = contents.lines();
    while let Some(raw) = iter.next() {
        let line = raw.trim_start();
        if line.is_empty() {
            continue;
        }
        // Rust file-level doc-comment.
        if let Some(rest) = line.strip_prefix("//!") {
            let summary = rest.trim();
            if !summary.is_empty() {
                return Some(summary.to_string());
            }
            continue; // empty //! line — keep scanning a few lines
        }
        // Shell script shebang — skip the shebang and look at next non-empty.
        if line.starts_with("#!") {
            continue;
        }
        // Shell / Makefile-style top comment.
        if let Some(rest) = line.strip_prefix("# ") {
            let s = rest.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        // Markdown / README — first heading or first paragraph.
        if line.starts_with('#') {
            let s = line.trim_start_matches('#').trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        // First non-empty non-comment line is a poor signal — bail rather
        // than misreport.
        return None;
    }
    None
}

fn render_node_with_metadata(
    node: &TreeNode,
    accumulated: &Path,
    metadata: &std::collections::HashMap<PathBuf, (usize, Option<String>)>,
    prefix: &str,
    lines: &mut Vec<String>,
) {
    let n = node.children.len();
    // Find the longest leaf name in this subtree to pad LOC alignment.
    for (i, (name, child)) in node.children.iter().enumerate() {
        let last = i + 1 == n;
        let branch = if last { "`-- " } else { "|-- " };
        let child_path = accumulated.join(name);
        let is_leaf = child.children.is_empty();
        let line = if is_leaf {
            match metadata.get(&child_path) {
                Some((loc, summary)) => {
                    let loc_str = format!("({loc} lines)");
                    match summary {
                        Some(s) if !s.is_empty() => {
                            // Truncate long summaries to keep tree scannable.
                            let s = if s.len() > 80 {
                                let mut t = s.chars().take(77).collect::<String>();
                                t.push_str("...");
                                t
                            } else {
                                s.clone()
                            };
                            format!("{prefix}{branch}{name}  {loc_str}  — {s}")
                        }
                        _ => format!("{prefix}{branch}{name}  {loc_str}"),
                    }
                }
                None => format!("{prefix}{branch}{name}"),
            }
        } else {
            format!("{prefix}{branch}{name}")
        };
        lines.push(line);
        if !is_leaf {
            let next = if last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}|   ")
            };
            render_node_with_metadata(child, &child_path, metadata, &next, lines);
        }
    }
}
