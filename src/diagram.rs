//! External diagram renderers (Graphviz `dot`, Mermaid `mmdc`, PlantUML).
//!
//! When a code fence carries a recognised diagram language (`dot`, `graphviz`,
//! `mermaid`, `plantuml`), we try to shell out to the corresponding CLI to
//! render the diagram to PNG and replace the code block with an image. If
//! the tool isn't installed or rendering fails, we leave the original code
//! block intact so the source is still visible in the slide.
//!
//! Rendered images are written into a per-build cache directory and embedded
//! through the normal image pipeline.
//!
//! On `wasm32` diagram rendering is a no-op (no process spawning in the browser).

use crate::ir::*;
use std::path::{Path, PathBuf};

/// Walk slides; replace recognised diagram code blocks with rendered images.
/// Writes PNGs into `cache_dir`. Returns the number of diagrams rendered.
pub fn pre_render(slides: &mut [Slide], cache_dir: &Path) -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (slides, cache_dir);
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        native::pre_render(slides, cache_dir)
    }
}

/// Build a per-deck cache directory under the system temp area, keyed by a
/// hash of the input deck path so concurrent invocations don't trample each
/// other. Returns the directory path.
pub fn cache_dir_for(deck_stem: &str) -> PathBuf {
    #[cfg(target_arch = "wasm32")]
    {
        PathBuf::from(format!("md2any-diagrams-{deck_stem}"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut dir = std::env::temp_dir();
        dir.push(format!("md2any-diagrams-{}", deck_stem));
        dir
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::process::{Command, Stdio};

    /// Lang identifiers we treat as renderable diagrams.
    fn detect(lang: Option<&str>) -> Option<DiagramKind> {
        let lang = lang?.trim().to_ascii_lowercase();
        match lang.as_str() {
            "dot" | "graphviz" => Some(DiagramKind::Dot),
            "mermaid" | "mmd" => Some(DiagramKind::Mermaid),
            "plantuml" | "puml" => Some(DiagramKind::PlantUml),
            _ => None,
        }
    }

    #[derive(Clone, Copy)]
    enum DiagramKind {
        Dot,
        Mermaid,
        PlantUml,
    }

    impl DiagramKind {
        fn tool(&self) -> &'static str {
            match self {
                DiagramKind::Dot => "dot",
                DiagramKind::Mermaid => "mmdc",
                DiagramKind::PlantUml => "plantuml",
            }
        }
    }

    pub fn pre_render(slides: &mut [Slide], cache_dir: &Path) -> usize {
        let mut rendered = 0;
        let mut idx = 0;
        for slide in slides.iter_mut() {
            rendered += render_blocks(&mut slide.blocks, cache_dir, &mut idx);
        }
        rendered
    }

    fn render_blocks(blocks: &mut Vec<Block>, cache_dir: &Path, idx: &mut usize) -> usize {
        let mut rendered = 0;
        let mut i = 0;
        while i < blocks.len() {
            if let Block::Columns { left, right } = &mut blocks[i] {
                rendered += render_blocks(left, cache_dir, idx);
                rendered += render_blocks(right, cache_dir, idx);
                i += 1;
                continue;
            }
            let take_kind = if let Block::CodeBlock { lang, .. } = &blocks[i] {
                detect(lang.as_deref())
            } else {
                None
            };
            if let Some(kind) = take_kind {
                let Block::CodeBlock { lines, .. } = &blocks[i] else {
                    unreachable!()
                };
                let source = lines.join("\n");
                *idx += 1;
                let out_path = cache_dir.join(format!("diagram_{}.png", *idx));
                match render_one(kind, &source, &out_path) {
                    Ok(()) => {
                        let src = out_path.to_string_lossy().into_owned();
                        blocks[i] = Block::Image {
                            src,
                            alt: format!("{} diagram", kind.tool()),
                            width_pct: None,
                            fit: None,
                            rounded: false,
                        };
                        rendered += 1;
                    }
                    Err(_) => {
                        // Leave the code block in place; user can still read source.
                    }
                }
            }
            i += 1;
        }
        rendered
    }

    fn render_one(kind: DiagramKind, source: &str, out: &Path) -> Result<(), String> {
        if !tool_available(kind.tool()) {
            return Err(format!("{} not on PATH", kind.tool()));
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        match kind {
            DiagramKind::Dot => run_with_stdin(
                "dot",
                &["-Tpng", "-Gdpi=144", "-o", out.to_str().unwrap_or("")],
                source,
            ),
            DiagramKind::Mermaid => {
                let in_path = out.with_extension("mmd");
                std::fs::write(&in_path, source).map_err(|e| e.to_string())?;
                let r = run_quiet(
                    "mmdc",
                    &[
                        "-i",
                        in_path.to_str().unwrap_or(""),
                        "-o",
                        out.to_str().unwrap_or(""),
                        "-b",
                        "transparent",
                        "-s",
                        "2",
                    ],
                );
                let _ = std::fs::remove_file(&in_path);
                r
            }
            DiagramKind::PlantUml => {
                run_capture_stdin_to_file("plantuml", &["-tpng", "-pipe"], source, out)
            }
        }
    }

    fn tool_available(name: &str) -> bool {
        Command::new(name)
            .arg("-V")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or_else(|_| {
                Command::new(name)
                    .arg("--version")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            })
    }

    fn run_with_stdin(cmd: &str, args: &[&str], stdin_text: &str) -> Result<(), String> {
        use std::io::Write;
        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn {}: {}", cmd, e))?;
        {
            let stdin = child.stdin.as_mut().ok_or("no stdin")?;
            stdin
                .write_all(stdin_text.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(())
    }

    fn run_capture_stdin_to_file(
        cmd: &str,
        args: &[&str],
        stdin_text: &str,
        out_path: &Path,
    ) -> Result<(), String> {
        use std::io::Write;
        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn {}: {}", cmd, e))?;
        {
            let stdin = child.stdin.as_mut().ok_or("no stdin")?;
            stdin
                .write_all(stdin_text.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        if out.stdout.is_empty() {
            return Err(format!("{} produced no output", cmd));
        }
        std::fs::write(out_path, &out.stdout).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn run_quiet(cmd: &str, args: &[&str]) -> Result<(), String> {
        let out = Command::new(cmd)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("spawn {}: {}", cmd, e))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(())
    }
}
