//! OpenDocument Presentation (.odp) writer.
//!
//! Produces a fully-native ODP file: ZIP archive containing mimetype,
//! META-INF/manifest.xml, meta.xml, styles.xml, content.xml, and any embedded
//! Pictures/. Opens natively in LibreOffice Impress, imports into Keynote and
//! Google Slides.
//!
//! Architecture mirrors `pptx.rs`: the same slide IR + layout + theme drive
//! emission, only the XML schema differs. Coordinates convert from EMU
//! (PPTX-native) to cm at output time.

use crate::image::{self, ImageMeta};
use crate::ir::*;
use crate::layout::{Layout, LayoutKind};
use crate::syntax::{self, Token};
use crate::theme::Theme;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::FileOptions;
use zip::CompressionMethod;

thread_local! {
    /// Per-slide body alignment (`"left"`/`"center"`/`"right"`) from the
    /// `align=` hint, read by `render_blocks` text frames.
    static SLIDE_ALIGN: std::cell::RefCell<&'static str> = const { std::cell::RefCell::new("left") };
    /// Per-slide left-column fraction from `width=` (`None` = even split).
    static SLIDE_COL_FRAC: std::cell::RefCell<Option<f32>> = const { std::cell::RefCell::new(None) };
}

fn slide_align() -> &'static str {
    SLIDE_ALIGN.with(|a| *a.borrow())
}

fn slide_col_frac() -> Option<f32> {
    SLIDE_COL_FRAC.with(|f| *f.borrow())
}

/// Resolve a per-slide `bg:` token into a bare 6-digit hex. Mirrors PDF/PPTX.
fn resolve_bg_color(token: &str, theme: &Theme) -> String {
    if let Some(hex) = token.strip_prefix('#') {
        return hex.to_string();
    }
    match token.to_ascii_lowercase().as_str() {
        "accent" => theme.accent.clone(),
        "section" => theme.section_bg.clone(),
        "dark" => "0F172A".to_string(),
        "light" => "FFFFFF".to_string(),
        _ => theme.bg.clone(),
    }
}

const NS: &str = concat!(
    " xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"",
    " xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\"",
    " xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"",
    " xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\"",
    " xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\"",
    " xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\"",
    " xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
    " xmlns:dc=\"http://purl.org/dc/elements/1.1/\"",
    " xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\"",
    " xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\"",
    " xmlns:presentation=\"urn:oasis:names:tc:opendocument:xmlns:presentation:1.0\"",
);

pub fn write(
    slides: &[Slide],
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    transition: Option<&str>,
    transition_dur: f32,
    direction: Option<&str>,
) -> Result<Vec<u8>> {
    let rtl = direction
        .map(|s| s.eq_ignore_ascii_case("rtl"))
        .unwrap_or(false);
    let mut metas: Vec<ImageMeta> = Vec::new();
    let mut by_src: HashMap<String, usize> = HashMap::new();
    for slide in slides {
        collect_block_images(&slide.blocks, base_dir, &mut metas, &mut by_src)?;
        if let Some(bg) = &slide.bg_image {
            if !by_src.contains_key(bg) {
                let path = base_dir.join(bg);
                let meta = image::load(&path)
                    .with_context(|| format!("loading background image {}", bg))?;
                by_src.insert(bg.clone(), metas.len());
                metas.push(meta);
            }
        }
    }
    let logo_key: Option<String> = if let Some(p) = logo {
        let key = format!("__logo:{}", p.display());
        if !by_src.contains_key(&key) {
            let meta =
                image::load(p).with_context(|| format!("loading logo image {}", p.display()))?;
            by_src.insert(key.clone(), metas.len());
            metas.push(meta);
        }
        Some(key)
    } else {
        None
    };
    let imgs = Imgs {
        by_src: &by_src,
        metas: &metas,
        logo_key: logo_key.as_deref(),
    };

    let buf: Vec<u8> = Vec::new();
    let cursor = Cursor::new(buf);
    let mut zip = zip::ZipWriter::new(cursor);
    let stored = FileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = FileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/vnd.oasis.opendocument.presentation")?;

    let manifest = build_manifest(&metas);
    write_file(&mut zip, "META-INF/manifest.xml", &manifest, deflated)?;

    let meta = build_meta(deck_title, author);
    write_file(&mut zip, "meta.xml", &meta, deflated)?;

    let styles = build_styles(theme, transition);
    write_file(&mut zip, "styles.xml", &styles, deflated)?;

    let content = build_content(
        slides,
        theme,
        layout,
        deck_title,
        &imgs,
        transition,
        transition_dur,
    );
    let content = if rtl {
        apply_rtl_odp(&content)
    } else {
        content
    };
    write_file(&mut zip, "content.xml", &content, deflated)?;

    for (idx, m) in metas.iter().enumerate() {
        zip.start_file(&format!("Pictures/image{}.{}", idx + 1, m.ext), stored)?;
        zip.write_all(&m.bytes)?;
    }

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}

fn write_file<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    content: &str,
    opts: FileOptions,
) -> Result<()> {
    zip.start_file(name, opts)?;
    zip.write_all(content.as_bytes())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Image collection
// ---------------------------------------------------------------------------

struct Imgs<'a> {
    by_src: &'a HashMap<String, usize>,
    metas: &'a [ImageMeta],
    logo_key: Option<&'a str>,
}

impl<'a> Imgs<'a> {
    fn dims(&self, src: &str) -> Option<(u32, u32)> {
        self.by_src
            .get(src)
            .map(|i| (self.metas[*i].width, self.metas[*i].height))
    }
    fn index(&self, src: &str) -> Option<usize> {
        self.by_src.get(src).copied()
    }
    fn href(&self, src: &str) -> Option<String> {
        let idx = self.index(src)?;
        Some(format!("Pictures/image{}.{}", idx + 1, self.metas[idx].ext))
    }
    fn logo(&self) -> Option<(&str, u32, u32)> {
        let k = self.logo_key?;
        let i = *self.by_src.get(k)?;
        Some((k, self.metas[i].width, self.metas[i].height))
    }
}

fn collect_block_images(
    blocks: &[Block],
    base_dir: &Path,
    metas: &mut Vec<ImageMeta>,
    by_src: &mut HashMap<String, usize>,
) -> Result<()> {
    for b in blocks {
        match b {
            Block::Image { src, .. } => {
                if by_src.contains_key(src) {
                    continue;
                }
                let meta = image::load_any_or_placeholder(base_dir, src);
                by_src.insert(src.clone(), metas.len());
                metas.push(meta);
            }
            Block::Columns { left, right } => {
                collect_block_images(left, base_dir, metas, by_src)?;
                collect_block_images(right, base_dir, metas, by_src)?;
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Static XML parts: mimetype, manifest, meta, styles
// ---------------------------------------------------------------------------

fn build_manifest(metas: &[ImageMeta]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
<manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.presentation"/>
<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
<manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>
<manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>
"#,
    );
    for (i, m) in metas.iter().enumerate() {
        let mime = if m.ext == "png" {
            "image/png"
        } else {
            "image/jpeg"
        };
        s.push_str(&format!(
            "<manifest:file-entry manifest:full-path=\"Pictures/image{}.{}\" manifest:media-type=\"{}\"/>\n",
            i + 1,
            m.ext,
            mime,
        ));
    }
    s.push_str("</manifest:manifest>");
    s
}

fn build_meta(deck_title: &str, author: &str) -> String {
    // Stable timestamp — md2any aims for byte-identical output across
    // re-renders so committed `examples/demo.odp` and similar artifacts
    // don't churn in git on every regeneration. The trade-off is that
    // every generated deck shows the same "created" date in viewer
    // metadata; consumers who care can edit it after the fact.
    let now = "2026-01-01T00:00:00";
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" office:version="1.2">
<office:meta>
<dc:title>{title}</dc:title>
<meta:initial-creator>{author}</meta:initial-creator>
<dc:creator>{author}</dc:creator>
<meta:creation-date>{now}</meta:creation-date>
<dc:date>{now}</dc:date>
<meta:generator>md2any</meta:generator>
</office:meta>
</office:document-meta>"#,
        title = escape_xml(deck_title),
        author = escape_xml(author),
        now = now,
    )
}

fn build_styles(theme: &Theme, transition: Option<&str>) -> String {
    let trans_attrs = odp_transition_attrs(transition);
    let (page_w, page_h) = (cm(theme.slide_w), cm(theme.slide_h));
    let orient = if theme.portrait {
        "portrait"
    } else {
        "landscape"
    };
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles{ns} office:version="1.2">
<office:styles>
<style:default-style style:family="graphic">
<style:graphic-properties draw:fill="none" draw:stroke="none"/>
<style:paragraph-properties fo:text-align="start"/>
<style:text-properties style:font-name="{body_font}" fo:color="#{body_color}"/>
</style:default-style>
</office:styles>
<office:automatic-styles>
<style:page-layout style:name="PL1">
<style:page-layout-properties fo:page-width="{w}" fo:page-height="{h}" style:print-orientation="{orient}"/>
</style:page-layout>
<style:style style:name="dp-default" style:family="drawing-page">
<style:drawing-page-properties draw:fill="solid" draw:fill-color="#{bg}" presentation:background-objects-visible="true" presentation:background-visible="true" presentation:display-header="false" presentation:display-footer="false" presentation:display-page-number="false" presentation:display-date-time="false"{trans}/>
</style:style>
<style:style style:name="dp-section" style:family="drawing-page">
<style:drawing-page-properties draw:fill="solid" draw:fill-color="#{section_bg}" presentation:background-objects-visible="true" presentation:background-visible="true" presentation:display-header="false" presentation:display-footer="false" presentation:display-page-number="false" presentation:display-date-time="false"{trans}/>
</style:style>
</office:automatic-styles>
<office:master-styles>
<style:master-page style:name="Default" style:page-layout-name="PL1" draw:style-name="dp-default"/>
<style:master-page style:name="Section" style:page-layout-name="PL1" draw:style-name="dp-section"/>
</office:master-styles>
</office:document-styles>"##,
        ns = NS,
        w = page_w,
        h = page_h,
        orient = orient,
        bg = theme.bg,
        section_bg = theme.section_bg,
        body_color = theme.body_color,
        body_font = escape_xml(&theme.body_font),
        trans = trans_attrs,
    )
}

/// Flip ODP paragraph alignment from `start`/`left` to `end`/`right` so the
/// slides read right-to-left. ODF also needs a writing-mode but most readers
/// derive that from the text alignment + script context.
fn apply_rtl_odp(xml: &str) -> String {
    xml.replace(r#"fo:text-align="start""#, r#"fo:text-align="end""#)
        .replace(r#"fo:text-align="left""#, r#"fo:text-align="right""#)
        .replace(r#"text-align:start"#, r#"text-align:end"#)
        .replace(r#"text-align:left"#, r#"text-align:right"#)
}

fn odp_transition_attrs(kind: Option<&str>) -> String {
    let Some(kind) = kind else {
        return String::new();
    };
    let kind = kind.to_ascii_lowercase();
    if kind.is_empty() || kind == "none" {
        return String::new();
    }
    let style = match kind.as_str() {
        "fade" => "fade",
        "push" => "move-from-right",
        "wipe" => "horizontal-stripes",
        "cover" => "uncover-to-left",
        "split" => "vertical-stripes",
        _ => return String::new(),
    };
    format!(
        r#" presentation:transition-type="automatic" presentation:transition-style="{}" presentation:transition-speed="medium""#,
        style,
    )
}

// ---------------------------------------------------------------------------
// Style registry (auto-styles emitted in content.xml header)
// ---------------------------------------------------------------------------

#[derive(Hash, Eq, PartialEq, Clone)]
struct TextStyleKey {
    color: String,
    size_pt: u32,
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    font: String,
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct ParaStyleKey {
    align: &'static str,
    line_spacing_pct: u16,
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct GraphicStyleKey {
    fill: Option<String>,
    no_fill: bool,
    stroke: Option<(String, u32)>,
    corner_radius_emu: u32,
    text_anchor: &'static str,
    text_align: &'static str,
}

struct Registry {
    text: HashMap<TextStyleKey, String>,
    para: HashMap<ParaStyleKey, String>,
    graphic: HashMap<GraphicStyleKey, String>,
    /// Per-slide drawing-page styles keyed by fill colour (bare hex), for
    /// `bg:` tinting. Emitted into content.xml automatic-styles.
    drawing_page: HashMap<String, String>,
    text_seq: u32,
    para_seq: u32,
    graphic_seq: u32,
    dp_seq: u32,
}

impl Registry {
    fn new() -> Self {
        Registry {
            text: HashMap::new(),
            para: HashMap::new(),
            graphic: HashMap::new(),
            drawing_page: HashMap::new(),
            text_seq: 0,
            para_seq: 0,
            graphic_seq: 0,
            dp_seq: 0,
        }
    }

    /// Register a drawing-page style filling the page with `fill` (bare hex)
    /// and return its style name for a `draw:page draw:style-name`.
    fn dp_name(&mut self, fill: String) -> String {
        if let Some(n) = self.drawing_page.get(&fill) {
            return n.clone();
        }
        let n = format!("dpx{}", self.dp_seq);
        self.dp_seq += 1;
        self.drawing_page.insert(fill, n.clone());
        n
    }

    fn text_name(&mut self, k: TextStyleKey) -> String {
        if let Some(n) = self.text.get(&k) {
            return n.clone();
        }
        let n = format!("T{}", self.text_seq);
        self.text_seq += 1;
        self.text.insert(k, n.clone());
        n
    }

    fn para_name(&mut self, k: ParaStyleKey) -> String {
        if let Some(n) = self.para.get(&k) {
            return n.clone();
        }
        let n = format!("P{}", self.para_seq);
        self.para_seq += 1;
        self.para.insert(k, n.clone());
        n
    }

    fn graphic_name(&mut self, k: GraphicStyleKey) -> String {
        if let Some(n) = self.graphic.get(&k) {
            return n.clone();
        }
        let n = format!("gr{}", self.graphic_seq);
        self.graphic_seq += 1;
        self.graphic.insert(k, n.clone());
        n
    }

    fn emit(&self) -> String {
        let mut s = String::new();

        let mut dps: Vec<(&String, &String)> = self.drawing_page.iter().collect();
        dps.sort_by(|a, b| a.1.cmp(b.1));
        for (fill, name) in dps {
            s.push_str(&format!(
                r##"<style:style style:name="{n}" style:family="drawing-page"><style:drawing-page-properties draw:fill="solid" draw:fill-color="#{c}" presentation:background-objects-visible="true" presentation:background-visible="true" presentation:display-header="false" presentation:display-footer="false" presentation:display-page-number="false" presentation:display-date-time="false"/></style:style>"##,
                n = name,
                c = fill,
            ));
        }

        let mut paras: Vec<(&ParaStyleKey, &String)> = self.para.iter().collect();
        paras.sort_by(|a, b| a.1.cmp(b.1));
        for (k, name) in paras {
            s.push_str(&format!(
                r#"<style:style style:name="{n}" style:family="paragraph"><style:paragraph-properties fo:text-align="{a}" style:line-height-at-least="0cm" fo:line-height="{lh}%"/></style:style>"#,
                n = name,
                a = k.align,
                lh = k.line_spacing_pct,
            ));
        }

        let mut texts: Vec<(&TextStyleKey, &String)> = self.text.iter().collect();
        texts.sort_by(|a, b| a.1.cmp(b.1));
        for (k, name) in texts {
            let mut attrs = format!(
                r##" fo:color="#{c}" fo:font-size="{sz}pt""##,
                c = k.color,
                sz = k.size_pt,
            );
            attrs.push_str(&format!(
                r#" style:font-name="{f}" fo:font-family="{f}""#,
                f = escape_xml(&k.font),
            ));
            if k.bold {
                attrs.push_str(r#" fo:font-weight="bold""#);
            }
            if k.italic {
                attrs.push_str(r#" fo:font-style="italic""#);
            }
            if k.strike {
                attrs.push_str(r#" style:text-line-through-style="solid""#);
            }
            if k.underline {
                attrs.push_str(r#" style:text-underline-style="solid""#);
            }
            s.push_str(&format!(
                r#"<style:style style:name="{n}" style:family="text"><style:text-properties{attrs}/></style:style>"#,
                n = name,
                attrs = attrs,
            ));
        }

        let mut graphics: Vec<(&GraphicStyleKey, &String)> = self.graphic.iter().collect();
        graphics.sort_by(|a, b| a.1.cmp(b.1));
        for (k, name) in graphics {
            let fill_attr = if k.no_fill {
                String::from(r#" draw:fill="none""#)
            } else if let Some(c) = &k.fill {
                format!(r##" draw:fill="solid" draw:fill-color="#{}""##, c)
            } else {
                String::from(r#" draw:fill="none""#)
            };
            let stroke_attr = if let Some((c, w)) = &k.stroke {
                format!(
                    r##" draw:stroke="solid" svg:stroke-color="#{}" svg:stroke-width="{}""##,
                    c,
                    cm(*w),
                )
            } else {
                String::from(r#" draw:stroke="none""#)
            };
            let corner = if k.corner_radius_emu > 0 {
                format!(r#" draw:corner-radius="{}""#, cm(k.corner_radius_emu))
            } else {
                String::new()
            };
            s.push_str(&format!(
                r#"<style:style style:name="{n}" style:family="graphic"><style:graphic-properties{fill}{stroke}{corner} draw:textarea-vertical-align="{anchor}" draw:textarea-horizontal-align="{halign}" fo:padding="0cm"/></style:style>"#,
                n = name,
                fill = fill_attr,
                stroke = stroke_attr,
                corner = corner,
                anchor = k.text_anchor,
                halign = k.text_align,
            ));
        }

        s
    }
}

// ---------------------------------------------------------------------------
// Slide rendering
// ---------------------------------------------------------------------------

fn build_content(
    slides: &[Slide],
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    imgs: &Imgs,
    _transition: Option<&str>,
    _transition_dur: f32,
) -> String {
    let mut reg = Registry::new();
    let mut body = String::new();

    for (i, slide) in slides.iter().enumerate() {
        let num = i + 1;
        let total = slides.len();
        // Publish per-slide layout hints for the block renderers.
        SLIDE_ALIGN.with(|a| {
            *a.borrow_mut() = match slide.text_align() {
                "center" => "center",
                "right" => "right",
                _ => "left",
            }
        });
        SLIDE_COL_FRAC.with(|f| *f.borrow_mut() = slide.col_frac());
        let xml = match &slide.kind {
            SlideKind::Title {
                subtitle,
                author,
                date,
            } => render_title_slide(
                slide,
                num,
                subtitle.as_deref(),
                author.as_deref(),
                date.as_deref(),
                theme,
                layout,
                imgs,
                &mut reg,
            ),
            SlideKind::Section => {
                render_section_slide(slide, num, total, theme, layout, imgs, &mut reg)
            }
            SlideKind::Content => {
                render_content_slide(slide, num, total, deck_title, theme, layout, imgs, &mut reg)
            }
        };
        body.push_str(&xml);
    }

    let auto_styles = reg.emit();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content{ns} office:version="1.2">
<office:automatic-styles>
{styles}
</office:automatic-styles>
<office:body>
<office:presentation>
{body}
</office:presentation>
</office:body>
</office:document-content>"#,
        ns = NS,
        styles = auto_styles,
        body = body,
    )
}

fn page_open(name: &str, master: &str) -> String {
    page_open_styled(name, master, "dp-default")
}

fn page_open_styled(name: &str, master: &str, dp_style: &str) -> String {
    format!(
        r#"<draw:page draw:name="{n}" draw:style-name="{dp}" draw:master-page-name="{m}">"#,
        n = escape_xml(name),
        dp = dp_style,
        m = master,
    )
}

fn page_close(notes: Option<&str>, theme: &Theme, reg: &mut Registry) -> String {
    let mut s = String::new();
    if let Some(notes_text) = notes {
        let text_style = reg.text_name(TextStyleKey {
            color: "1F2937".into(),
            size_pt: 12,
            bold: false,
            italic: false,
            strike: false,
            underline: false,
            font: theme.body_font.clone(),
        });
        let para_style = reg.para_name(ParaStyleKey {
            align: "start",
            line_spacing_pct: 120,
        });
        let graphic = reg.graphic_name(GraphicStyleKey {
            fill: None,
            no_fill: true,
            stroke: None,
            corner_radius_emu: 0,
            text_anchor: "top",
            text_align: "left",
        });
        let mut paras = String::new();
        for line in notes_text.split('\n') {
            paras.push_str(&format!(
                r#"<text:p text:style-name="{p}"><text:span text:style-name="{t}">{x}</text:span></text:p>"#,
                p = para_style,
                t = text_style,
                x = escape_xml(line.trim()),
            ));
        }
        s.push_str(&format!(
            r#"<presentation:notes><draw:frame draw:style-name="{g}" presentation:class="notes" svg:x="2cm" svg:y="13cm" svg:width="17cm" svg:height="12cm"><draw:text-box>{p}</draw:text-box></draw:frame></presentation:notes>"#,
            g = graphic,
            p = paras,
        ));
    }
    s.push_str("</draw:page>\n");
    s
}

// Slide rendering helpers ----------------------------------------------------

fn bg_image_xml(slide: &Slide, theme: &Theme, imgs: &Imgs, reg: &mut Registry) -> String {
    if let Some(bg) = &slide.bg_image {
        if let Some(href) = imgs.href(bg) {
            let g = reg.graphic_name(GraphicStyleKey {
                fill: None,
                no_fill: true,
                stroke: None,
                corner_radius_emu: 0,
                text_anchor: "top",
                text_align: "left",
            });
            return format!(
                r#"<draw:frame draw:style-name="{g}" svg:x="0cm" svg:y="0cm" svg:width="{w}" svg:height="{h}"><draw:image xlink:href="{href}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>"#,
                g = g,
                w = cm(theme.slide_w),
                h = cm(theme.slide_h),
                href = escape_xml(&href),
            );
        }
    }
    String::new()
}

fn render_title_slide(
    slide: &Slide,
    num: usize,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    layout: &Layout,
    imgs: &Imgs,
    reg: &mut Registry,
) -> String {
    let mut s = page_open(&format!("page{}", num), "Default");
    s.push_str(&bg_image_xml(slide, theme, imgs, reg));
    match layout.kind {
        LayoutKind::Clean => title_clean(&mut s, slide, subtitle, author, date, theme, reg),
        LayoutKind::Studio => title_studio(&mut s, slide, subtitle, author, date, theme, reg),
        LayoutKind::Frame => title_frame(&mut s, slide, subtitle, author, date, theme, reg),
        LayoutKind::Bold => title_bold(&mut s, slide, subtitle, author, date, theme, reg),
    }
    s.push_str(&page_close(slide.notes.as_deref(), theme, reg));
    s
}

fn render_section_slide(
    slide: &Slide,
    num: usize,
    total: usize,
    theme: &Theme,
    layout: &Layout,
    imgs: &Imgs,
    reg: &mut Registry,
) -> String {
    let master = if matches!(layout.kind, LayoutKind::Clean | LayoutKind::Frame) {
        "Section"
    } else {
        "Default"
    };
    let mut s = format!(
        r#"<draw:page draw:name="page{}" draw:style-name="dp-{}" draw:master-page-name="{}">"#,
        num,
        if master == "Section" {
            "section"
        } else {
            "default"
        },
        master,
    );
    s.push_str(&bg_image_xml(slide, theme, imgs, reg));
    match layout.kind {
        LayoutKind::Clean | LayoutKind::Frame => section_centered(&mut s, slide, theme, reg),
        LayoutKind::Studio => section_studio(&mut s, slide, num, total, theme, reg),
        LayoutKind::Bold => section_bold(&mut s, slide, theme, reg),
    }
    s.push_str(&page_close(slide.notes.as_deref(), theme, reg));
    s
}

fn render_content_slide(
    slide: &Slide,
    num: usize,
    total: usize,
    deck_title: &str,
    theme: &Theme,
    layout: &Layout,
    imgs: &Imgs,
    reg: &mut Registry,
) -> String {
    // Per-slide bg: tint → a registered drawing-page style; else the default.
    let dp_style = match slide.bg_color() {
        Some(c) => reg.dp_name(resolve_bg_color(c, theme)),
        None => "dp-default".to_string(),
    };
    let mut s = page_open_styled(&format!("page{}", num), "Default", &dp_style);
    s.push_str(&bg_image_xml(slide, theme, imgs, reg));
    if let Some((src, alt, _)) = slide.full_page_image() {
        s.push_str(&render_full_page_image(src, alt, theme, imgs, reg));
    } else if let Some((lines, lang)) = slide.full_page_code() {
        s.push_str(&render_full_page_code(lines, lang, theme, reg));
    } else {
        content_slide_body(
            &mut s, slide, num, total, deck_title, theme, layout, imgs, reg,
        );
    }
    s.push_str(&page_close(slide.notes.as_deref(), theme, reg));
    s
}

// ---------------------------------------------------------------------------
// Title slide variants
// ---------------------------------------------------------------------------

fn title_clean(
    s: &mut String,
    slide: &Slide,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    reg: &mut Registry,
) {
    let w = theme.slide_w;
    let h = theme.slide_h;

    s.push_str(&rect(reg, 0, 0, w, 360000, &theme.accent));
    s.push_str(&rect(
        reg,
        600000,
        h / 2 - 1700000,
        100000,
        600000,
        &theme.accent,
    ));

    let title_y = h / 2 - 1600000;
    s.push_str(&text_frame(
        reg,
        800000,
        title_y,
        w - 1200000,
        1400000,
        "top",
        "left",
        &[Run::plain(&slide.title)],
        theme.hero_size,
        &theme.title_color,
        true,
        false,
        &theme.title_font,
    ));

    if let Some(sub) = subtitle {
        let sub_size = if theme.portrait { 1800 } else { 2400 };
        s.push_str(&text_frame(
            reg,
            800000,
            title_y + 1400000,
            w - 1200000,
            700000,
            "top",
            "left",
            &[Run::plain(sub)],
            sub_size,
            &theme.accent,
            false,
            false,
            &theme.body_font,
        ));
    }

    if let Some(text) = author_date(author, date) {
        s.push_str(&text_frame(
            reg,
            800000,
            h - 700000,
            w - 1200000,
            400000,
            "top",
            "left",
            &[Run::plain(&text)],
            1400,
            &theme.muted_color,
            false,
            false,
            &theme.body_font,
        ));
    }
}

fn title_studio(
    s: &mut String,
    slide: &Slide,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    reg: &mut Registry,
) {
    let w = theme.slide_w;
    let h = theme.slide_h;

    s.push_str(&rect(reg, 0, 0, 90000, h, &theme.accent));

    if let Some(text) = author_date(author, date) {
        let kicker = letterspaced(&text);
        s.push_str(&text_frame(
            reg,
            900000,
            900000,
            w - 1500000,
            400000,
            "top",
            "left",
            &[Run::plain(&kicker)],
            1200,
            &theme.muted_color,
            true,
            false,
            &theme.body_font,
        ));
    }

    let title_y = h / 2 - 1000000;
    s.push_str(&text_frame(
        reg,
        900000,
        title_y,
        w - 1500000,
        1600000,
        "top",
        "left",
        &[Run::plain(&slide.title)],
        theme.hero_size,
        &theme.title_color,
        false,
        true,
        &theme.title_font,
    ));

    if let Some(sub) = subtitle {
        let sub_y = title_y + 1600000;
        let sub_size = if theme.portrait { 1700 } else { 2200 };
        s.push_str(&text_frame(
            reg,
            900000,
            sub_y,
            w - 1500000,
            700000,
            "top",
            "left",
            &[Run::plain(sub)],
            sub_size,
            &theme.body_color,
            false,
            false,
            &theme.body_font,
        ));
        s.push_str(&rect(
            reg,
            900000,
            sub_y + 800000,
            600000,
            30000,
            &theme.accent,
        ));
    }
}

fn title_frame(
    s: &mut String,
    slide: &Slide,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    reg: &mut Registry,
) {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let sidebar = 2_200_000_u32;

    s.push_str(&rect(reg, 0, 0, sidebar, h, &theme.accent));
    s.push_str(&rect(reg, 300000, 300000, 380000, 40000, &theme.on_accent));

    let mut meta_lines = Vec::new();
    if let Some(a) = author {
        meta_lines.push(a);
    }
    if let Some(d) = date {
        meta_lines.push(d);
    }
    if !meta_lines.is_empty() {
        let runs: Vec<Run> = meta_lines
            .iter()
            .enumerate()
            .flat_map(|(i, line)| {
                let mut v = Vec::new();
                if i > 0 {
                    v.push(Run::plain("\n"));
                }
                v.push(Run::plain(*line));
                v
            })
            .collect();
        s.push_str(&text_frame(
            reg,
            300000,
            h - 1200000,
            sidebar - 600000,
            900000,
            "top",
            "left",
            &runs,
            1300,
            &theme.on_accent,
            false,
            false,
            &theme.body_font,
        ));
    }

    let title_x = sidebar + 600000;
    let title_y = h / 2 - 1100000;
    let title_w = w - title_x - 600000;
    s.push_str(&text_frame(
        reg,
        title_x,
        title_y,
        title_w,
        1500000,
        "top",
        "left",
        &[Run::plain(&slide.title)],
        theme.hero_size,
        &theme.title_color,
        true,
        false,
        &theme.title_font,
    ));

    if let Some(sub) = subtitle {
        let sub_size = if theme.portrait { 1700 } else { 2200 };
        s.push_str(&text_frame(
            reg,
            title_x,
            title_y + 1500000,
            title_w,
            700000,
            "top",
            "left",
            &[Run::plain(sub)],
            sub_size,
            &theme.accent,
            false,
            false,
            &theme.body_font,
        ));
    }
}

fn title_bold(
    s: &mut String,
    slide: &Slide,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    reg: &mut Registry,
) {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let block_h = h * 60 / 100;
    let pad = 700000;

    s.push_str(&rect(reg, 0, 0, w, block_h, &theme.accent));

    let title_y = block_h - 1400000 - pad;
    s.push_str(&text_frame(
        reg,
        pad,
        title_y,
        w - 2 * pad,
        1400000,
        "top",
        "left",
        &[Run::plain(&slide.title)],
        theme.hero_size,
        &theme.on_accent,
        true,
        false,
        &theme.title_font,
    ));

    if let Some(sub) = subtitle {
        let sub_size = if theme.portrait { 1800 } else { 2400 };
        s.push_str(&text_frame(
            reg,
            pad,
            block_h + 500000,
            w - 2 * pad,
            700000,
            "top",
            "left",
            &[Run::plain(sub)],
            sub_size,
            &theme.title_color,
            false,
            false,
            &theme.body_font,
        ));
    }

    if let Some(text) = author_date(author, date) {
        s.push_str(&text_frame(
            reg,
            pad,
            h - 700000,
            w - 2 * pad,
            400000,
            "top",
            "left",
            &[Run::plain(&text)],
            1400,
            &theme.muted_color,
            false,
            false,
            &theme.body_font,
        ));
    }
}

// ---------------------------------------------------------------------------
// Section slide variants
// ---------------------------------------------------------------------------

fn section_centered(s: &mut String, slide: &Slide, theme: &Theme, reg: &mut Registry) {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let bar_w = 1200000;
    s.push_str(&rect(
        reg,
        (w - bar_w) / 2,
        h / 2 - 1000000,
        bar_w,
        60000,
        &theme.section_text,
    ));
    s.push_str(&text_frame(
        reg,
        800000,
        h / 2 - 700000,
        w - 1600000,
        1500000,
        "middle",
        "center",
        &[Run::plain(&slide.title)],
        theme.hero_size,
        &theme.section_text,
        true,
        false,
        &theme.title_font,
    ));
}

fn section_studio(
    s: &mut String,
    slide: &Slide,
    num: usize,
    total: usize,
    theme: &Theme,
    reg: &mut Registry,
) {
    let w = theme.slide_w;
    let h = theme.slide_h;

    s.push_str(&rect(reg, 0, 0, 90000, h, &theme.accent));

    let huge = format!("{:02}", num.min(99));
    let huge_size = if theme.portrait { 9000 } else { 13000 };
    s.push_str(&text_frame(
        reg,
        w - 2_400_000,
        380000,
        2_300_000,
        2_400_000,
        "top",
        "right",
        &[Run::plain(&huge)],
        huge_size,
        &theme.divider,
        true,
        false,
        &theme.title_font,
    ));

    let kicker_x = 900000;
    let kicker_y = h - 2_600_000;
    let kicker_text = format!(
        "{}   ·   {}",
        letterspaced("section"),
        letterspaced(&format!("{} of {}", num, total))
    );
    s.push_str(&text_frame(
        reg,
        kicker_x,
        kicker_y,
        w - 1800000,
        350000,
        "top",
        "left",
        &[Run::plain(&kicker_text)],
        1100,
        &theme.muted_color,
        true,
        false,
        &theme.body_font,
    ));

    let title_y = kicker_y + 400000;
    s.push_str(&text_frame(
        reg,
        kicker_x,
        title_y,
        w - kicker_x - 600000,
        1800000,
        "top",
        "left",
        &[Run::plain(&slide.title)],
        theme.hero_size,
        &theme.title_color,
        false,
        true,
        &theme.title_font,
    ));

    s.push_str(&rect(
        reg,
        kicker_x,
        title_y + 1900000,
        600000,
        30000,
        &theme.accent,
    ));
}

fn section_bold(s: &mut String, slide: &Slide, theme: &Theme, reg: &mut Registry) {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let block_h = h * 70 / 100;
    let pad = 700000;
    s.push_str(&rect(reg, 0, 0, w, block_h, &theme.accent));
    s.push_str(&text_frame(
        reg,
        pad,
        block_h - 1500000 - pad,
        w - 2 * pad,
        1500000,
        "top",
        "left",
        &[Run::plain(&slide.title)],
        theme.hero_size,
        &theme.on_accent,
        true,
        false,
        &theme.title_font,
    ));
}

// ---------------------------------------------------------------------------
// Content slide
// ---------------------------------------------------------------------------

fn content_slide_body(
    s: &mut String,
    slide: &Slide,
    num: usize,
    total: usize,
    deck_title: &str,
    theme: &Theme,
    layout: &Layout,
    imgs: &Imgs,
    reg: &mut Registry,
) {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let base_margin: u32 = 533400;
    let left_offset = layout.content_left_offset();
    let extra_left = if layout.shows_rail() { 200000 } else { 0 };
    let content_x = if left_offset > 0 {
        left_offset + 480000
    } else {
        base_margin + extra_left
    };
    let content_w = w.saturating_sub(content_x + base_margin);

    if layout.shows_rail() {
        s.push_str(&rect(reg, 0, 0, layout.rail_width(), h, &theme.accent));
    }

    if layout.shows_sidebar() {
        let sb_w = layout.sidebar_width();
        s.push_str(&rect(reg, 0, 0, sb_w, h, &theme.accent));
        let pad = 300000;
        s.push_str(&text_frame(
            reg,
            pad,
            pad,
            sb_w - 2 * pad,
            600000,
            "top",
            "left",
            &[Run::plain(deck_title)],
            1200,
            &theme.on_accent,
            true,
            false,
            &theme.body_font,
        ));
        s.push_str(&text_frame(
            reg,
            pad,
            h - 600000,
            sb_w - 2 * pad,
            400000,
            "top",
            "left",
            &[Run::plain(&format!("{:02} / {:02}", num, total))],
            1100,
            &theme.on_accent,
            false,
            false,
            &theme.body_font,
        ));
    }

    let title_y: u32 = if matches!(layout.kind, LayoutKind::Bold) {
        280000
    } else {
        360000
    };
    let title_h: u32 = if matches!(layout.kind, LayoutKind::Bold) {
        820000
    } else {
        720000
    };

    if matches!(layout.kind, LayoutKind::Bold) {
        let block_h = title_h + 240000;
        s.push_str(&rect(reg, 0, 0, w, title_y + block_h, &theme.accent));
        s.push_str(&text_frame(
            reg,
            base_margin,
            title_y,
            w - 2 * base_margin,
            title_h,
            "top",
            "left",
            &[Run::plain(&slide.title)],
            theme.title_size,
            &theme.on_accent,
            true,
            false,
            &theme.title_font,
        ));
    } else {
        s.push_str(&text_frame(
            reg,
            content_x,
            title_y,
            content_w,
            title_h,
            "top",
            "left",
            &[Run::plain(&slide.title)],
            theme.title_size,
            &theme.title_color,
            true,
            false,
            &theme.title_font,
        ));
    }

    let underline_y = if matches!(layout.kind, LayoutKind::Clean) {
        let y = title_y + title_h + 30000;
        s.push_str(&rect(
            reg,
            content_x,
            y + 18000,
            content_w,
            14000,
            &theme.divider,
        ));
        s.push_str(&rect(
            reg,
            content_x,
            y,
            progress_width(content_w, num, total),
            50000,
            &theme.accent,
        ));
        y
    } else {
        title_y + title_h
    };

    let content_y_start = underline_y
        + if matches!(layout.kind, LayoutKind::Clean) {
            200000
        } else {
            280000
        };
    let footer_y = h - 400000;
    let content_max_y = footer_y - 100000;
    let content_h_total = content_max_y.saturating_sub(content_y_start);

    render_blocks(
        s,
        &slide.blocks,
        theme,
        content_x,
        content_y_start,
        content_w,
        content_h_total,
        imgs,
        reg,
    );

    if !layout.shows_sidebar() {
        let rule_y = footer_y.saturating_sub(90000);
        s.push_str(&rect(
            reg,
            content_x,
            rule_y,
            content_w,
            18000,
            &theme.divider,
        ));
        s.push_str(&rect(
            reg,
            content_x,
            rule_y,
            content_w.min(420000),
            18000,
            &theme.accent,
        ));
        if let Some((key, iw, ih)) = imgs.logo() {
            let logo_h: u32 = 300000;
            let logo_w = if ih > 0 {
                ((logo_h as u64 * iw as u64) / ih as u64) as u32
            } else {
                logo_h
            };
            if let Some(href) = imgs.href(key) {
                let g = reg.graphic_name(GraphicStyleKey {
                    fill: None,
                    no_fill: true,
                    stroke: None,
                    corner_radius_emu: 0,
                    text_anchor: "top",
                    text_align: "left",
                });
                s.push_str(&format!(
                    r#"<draw:frame draw:style-name="{g}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:image xlink:href="{href}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>"#,
                    g = g,
                    x = cm(content_x),
                    y = cm(footer_y - 50000),
                    w = cm(logo_w),
                    h = cm(logo_h),
                    href = escape_xml(&href),
                ));
            }
        } else {
            s.push_str(&text_frame(
                reg,
                content_x,
                footer_y,
                content_w.saturating_sub(800000),
                300000,
                "top",
                "left",
                &[Run::plain(deck_title)],
                1000,
                &theme.muted_color,
                false,
                false,
                &theme.body_font,
            ));
        }
        s.push_str(&text_frame(
            reg,
            w - base_margin - 800000,
            footer_y,
            800000,
            300000,
            "top",
            "right",
            &[Run::plain(&format!("{} / {}", num, total))],
            1000,
            &theme.muted_color,
            false,
            false,
            &theme.body_font,
        ));
    }

    if layout.shows_corner_decoration() {
        s.push_str(&rect(
            reg,
            w - 280000,
            h - 280000,
            120000,
            120000,
            &theme.accent,
        ));
    }
}

// ---------------------------------------------------------------------------
// Block rendering
// ---------------------------------------------------------------------------

fn render_blocks(
    out: &mut String,
    blocks: &[Block],
    theme: &Theme,
    x: u32,
    y_start: u32,
    w: u32,
    h_total: u32,
    imgs: &Imgs,
    reg: &mut Registry,
) {
    let estimated: u32 = blocks
        .iter()
        .map(|b| block_height_emu(b, w, theme, imgs))
        .sum::<u32>()
        + (blocks.len().saturating_sub(1) as u32) * 80000;
    let scale: f32 = if estimated > h_total && estimated > 0 {
        h_total as f32 / estimated as f32
    } else {
        1.0
    };

    let mut y = y_start;
    for block in blocks {
        let raw_h = block_height_emu(block, w, theme, imgs);
        let h = (((raw_h as f32) * scale) as u32).max(200000);
        match block {
            Block::Paragraph(runs) => {
                out.push_str(&text_frame(
                    reg,
                    x,
                    y,
                    w,
                    h,
                    "top",
                    slide_align(),
                    runs,
                    theme.body_size,
                    &theme.body_color,
                    false,
                    false,
                    &theme.body_font,
                ));
            }
            Block::Heading { level, runs } => {
                let sz = match level {
                    3 => theme.title_size - 400,
                    4 => theme.title_size - 600,
                    _ => theme.title_size - 800,
                };
                out.push_str(&text_frame(
                    reg,
                    x,
                    y,
                    w,
                    h,
                    "top",
                    slide_align(),
                    runs,
                    sz,
                    &theme.title_color,
                    true,
                    false,
                    &theme.title_font,
                ));
            }
            Block::List(items) => {
                if items.len() > crate::theme::LONG_LIST_THRESHOLD && !theme.portrait {
                    let half = items.len().div_ceil(2);
                    let (l, r) = items.split_at(half);
                    let gap: u32 = 200000;
                    let col_w = (w.saturating_sub(gap)) / 2;
                    out.push_str(&list_frame(reg, x, y, col_w, h, l, theme));
                    out.push_str(&list_frame(reg, x + col_w + gap, y, col_w, h, r, theme));
                } else {
                    out.push_str(&list_frame(reg, x, y, w, h, items, theme));
                }
            }
            Block::CodeBlock {
                lang,
                title,
                lines,
                line_numbers,
                start_line,
                ..
            } => {
                out.push_str(&code_block(
                    reg,
                    x,
                    y,
                    w,
                    h,
                    lines,
                    title.as_deref(),
                    lang.as_deref(),
                    *line_numbers,
                    *start_line,
                    theme,
                ));
            }
            Block::Quote(paras) | Block::Callout { body: paras, .. } => {
                out.push_str(&rect(reg, x, y, 60000, h, &theme.accent));
                let mut runs: Vec<Run> = Vec::new();
                for (i, prun) in paras.iter().enumerate() {
                    if i > 0 {
                        runs.push(Run::plain("\n"));
                    }
                    runs.extend(prun.clone());
                }
                out.push_str(&text_frame(
                    reg,
                    x + 180000,
                    y,
                    w.saturating_sub(180000),
                    h,
                    "top",
                    "left",
                    &runs,
                    theme.body_size - 100,
                    &theme.body_color,
                    false,
                    true,
                    &theme.body_font,
                ));
            }
            Block::Table { headers, rows, .. } => {
                out.push_str(&render_table(reg, x, y, w, h, headers, rows, theme));
            }
            Block::Columns { left, right } => {
                let gap: u32 = 280000;
                let avail = w.saturating_sub(gap);
                let left_w = match slide_col_frac() {
                    Some(f) => (avail as f32 * f) as u32,
                    None => avail / 2,
                };
                let right_w = avail.saturating_sub(left_w);
                render_blocks(out, left, theme, x, y, left_w, h, imgs, reg);
                render_blocks(
                    out,
                    right,
                    theme,
                    x + left_w + gap,
                    y,
                    right_w,
                    h,
                    imgs,
                    reg,
                );
            }
            Block::ColumnBreak => {}
            Block::Image {
                src,
                alt,
                width_pct,
                fit: _,
                rounded: _,
            } => {
                let effective_w = match width_pct {
                    Some(pct) => w * (*pct as u32) / 100,
                    None => w,
                };
                out.push_str(&render_image(
                    reg,
                    x,
                    y,
                    effective_w,
                    h,
                    src,
                    alt,
                    theme,
                    imgs,
                ));
            }
            Block::Footnotes(items) => {
                // Render footnotes as a small-text list block, in muted color.
                out.push_str(&render_footnotes_block(reg, x, y, w, h, items, theme));
            }
            Block::Cards { cards, .. } => {
                render_blocks(out, &cards_as_blocks(cards), theme, x, y, w, h, imgs, reg);
            }
        }
        y += h + 80000;
    }
}

fn render_footnotes_block(
    reg: &mut Registry,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    items: &[ListItem],
    theme: &Theme,
) -> String {
    let mut combined: Vec<Run> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            combined.push(Run::plain("\n"));
        }
        combined.extend(item.runs.iter().cloned());
    }
    text_frame(
        reg,
        x,
        y,
        w,
        h,
        "top",
        "left",
        &combined,
        (theme.body_size as f32 * 0.7) as u32,
        &theme.muted_color,
        false,
        false,
        &theme.body_font,
    )
}

fn block_height_emu(b: &Block, w: u32, theme: &Theme, imgs: &Imgs) -> u32 {
    let cpl = chars_per_line(w);
    match b {
        Block::Paragraph(runs) => {
            let chars = total_chars(runs) as u32;
            let lines = (chars / cpl).max(1) + 1;
            lines * 270000 + 60000
        }
        Block::Heading { .. } => 520000,
        Block::List(items) => {
            let total: u32 = items
                .iter()
                .map(|i| {
                    let chars = i.runs.iter().map(|r| r.text.chars().count()).sum::<usize>() as u32;
                    let item_cpl = cpl.saturating_sub(4 + (i.level as u32) * 4).max(12);
                    let lines = (chars / item_cpl).max(1) + 1;
                    lines * 280000
                })
                .sum::<u32>()
                + 80000;
            if items.len() > crate::theme::LONG_LIST_THRESHOLD && !theme.portrait {
                let col_w = (w.saturating_sub(200000)) / 2;
                let col_cpl = chars_per_line(col_w);
                let col_total: u32 = items
                    .iter()
                    .map(|i| {
                        let chars =
                            i.runs.iter().map(|r| r.text.chars().count()).sum::<usize>() as u32;
                        let lines = (chars / col_cpl).max(1) + 1;
                        lines * 280000
                    })
                    .sum::<u32>();
                (col_total / 2) + 240000
            } else {
                total
            }
        }
        Block::CodeBlock {
            lines, title, lang, ..
        } => {
            let title_h = if title.is_some() || lang.is_some() {
                320000
            } else {
                0
            };
            (lines.len() as u32).max(1) * 280000 + 320000 + title_h
        }
        Block::Quote(paras) | Block::Callout { body: paras, .. } => {
            paras
                .iter()
                .map(|runs| {
                    let chars = total_chars(runs) as u32;
                    let lines = (chars / cpl.saturating_sub(2).max(12)).max(1) + 1;
                    lines * 280000
                })
                .sum::<u32>()
                + 120000
        }
        Block::Table { headers, rows, .. } => table_height_emu(headers, rows, w, theme),
        Block::Columns { left, right } => {
            let gap: u32 = 280000;
            let half = w.saturating_sub(gap) / 2;
            let lh: u32 = left
                .iter()
                .map(|b| block_height_emu(b, half, theme, imgs))
                .sum::<u32>()
                + (left.len().saturating_sub(1) as u32) * 80000;
            let rh: u32 = right
                .iter()
                .map(|b| block_height_emu(b, half, theme, imgs))
                .sum::<u32>()
                + (right.len().saturating_sub(1) as u32) * 80000;
            lh.max(rh)
        }
        Block::ColumnBreak => 0,
        Block::Image {
            src,
            alt,
            width_pct: _,
            fit: _,
            rounded: _,
        } => {
            let max_image_h = theme.slide_h * crate::theme::IMAGE_MAX_HEIGHT_FRACTION_NUM
                / crate::theme::IMAGE_MAX_HEIGHT_FRACTION_DEN;
            let placeholder_h = theme.slide_h * 30 / 100;
            let display_h = if let Some((iw, ih)) = imgs.dims(src) {
                let (_, dh) = fit_image_for_block(src, alt, iw, ih, w, max_image_h, theme.slide_h);
                dh
            } else {
                placeholder_h
            };
            let caption = if alt.is_empty() { 0 } else { 260000 };
            display_h + caption + 80000
        }
        Block::Footnotes(items) => {
            let total: u32 = items
                .iter()
                .map(|i| {
                    let chars = i.runs.iter().map(|r| r.text.chars().count()).sum::<usize>() as u32;
                    let lines = (chars / cpl.saturating_sub(2).max(20)).max(1);
                    lines * 200000
                })
                .sum::<u32>();
            total + 120000
        }
        Block::Cards { cards, .. } => cards_as_blocks(cards)
            .iter()
            .map(|b| block_height_emu(b, w, theme, imgs))
            .sum(),
    }
}

fn chars_per_line(w: u32) -> u32 {
    (w / 120000).max(20)
}

fn total_chars(runs: &[Run]) -> usize {
    runs.iter().map(|r| r.text.chars().count()).sum()
}

fn fit_image(iw: u32, ih: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if iw == 0 || ih == 0 {
        return (max_w, max_h);
    }
    let iw = iw as u64;
    let ih = ih as u64;
    let mw = max_w as u64;
    let mh = max_h as u64;
    let h_at_mw = mw * ih / iw;
    if h_at_mw <= mh {
        (mw as u32, h_at_mw as u32)
    } else {
        let w_at_mh = mh * iw / ih;
        (w_at_mh as u32, mh as u32)
    }
}

fn fit_image_for_block(
    src: &str,
    alt: &str,
    iw: u32,
    ih: u32,
    max_w: u32,
    max_h: u32,
    slide_h: u32,
) -> (u32, u32) {
    let Some(math_meta) = crate::math::math_image_meta(src, alt) else {
        return fit_image(iw, ih, max_w, max_h);
    };
    let natural_w = ((iw.max(1) as u64 * 12_700) / 2).min(u32::MAX as u64) as u32;
    let natural_h = ((ih.max(1) as u64 * 12_700) / 2).min(u32::MAX as u64) as u32;
    let configured_max_h = math_meta
        .max_height_px
        .map(|px| u32::from(px).saturating_mul(12_700));
    let math_max_h = configured_max_h
        .unwrap_or(slide_h * 28 / 100)
        .min(max_h)
        .max(1)
        .min(natural_h.max(1));
    fit_image(
        natural_w,
        natural_h,
        max_w.min(natural_w.max(1)),
        math_max_h,
    )
}

fn image_x_for_block(src: &str, alt: &str, x: u32, w: u32, display_w: u32) -> u32 {
    match crate::math::math_image_meta(src, alt).map(|meta| meta.align) {
        Some(crate::math::MathBlockAlign::Left) => x,
        Some(crate::math::MathBlockAlign::Right) => x + w.saturating_sub(display_w),
        Some(crate::math::MathBlockAlign::Center) | None => x + w.saturating_sub(display_w) / 2,
    }
}

// ---------------------------------------------------------------------------
// Frame + run emission
// ---------------------------------------------------------------------------

fn text_frame(
    reg: &mut Registry,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    anchor: &'static str,
    halign: &'static str,
    runs: &[Run],
    size_centipt: u32,
    color: &str,
    bold: bool,
    italic: bool,
    font: &str,
) -> String {
    let gr = reg.graphic_name(GraphicStyleKey {
        fill: None,
        no_fill: true,
        stroke: None,
        corner_radius_emu: 0,
        text_anchor: anchor,
        text_align: halign,
    });
    let para_align = match halign {
        "center" => "center",
        "right" => "end",
        _ => "start",
    };
    let para = reg.para_name(ParaStyleKey {
        align: para_align,
        line_spacing_pct: 120,
    });

    let runs_xml = runs_to_spans(reg, runs, size_centipt, color, bold, italic, font);

    format!(
        r#"<draw:frame draw:style-name="{g}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:text-box><text:p text:style-name="{p}">{r}</text:p></draw:text-box></draw:frame>"#,
        g = gr,
        x = cm(x),
        y = cm(y),
        w = cm(w),
        h = cm(h),
        p = para,
        r = runs_xml,
    )
}

fn runs_to_spans(
    reg: &mut Registry,
    runs: &[Run],
    base_size: u32,
    base_color: &str,
    base_bold: bool,
    base_italic: bool,
    font: &str,
) -> String {
    let mut s = String::new();
    for r in runs {
        if r.text.is_empty() {
            continue;
        }
        if r.text == "\n" {
            s.push_str("</text:p><text:p>");
            continue;
        }
        let color = if r.link.is_some() {
            // fallback to base if no theme accent known here; caller passes color directly
            base_color
        } else if r.code {
            base_color
        } else {
            base_color
        };
        let style = reg.text_name(TextStyleKey {
            color: color.to_string(),
            size_pt: base_size / 100,
            bold: base_bold || r.bold,
            italic: base_italic || r.italic,
            strike: r.strike,
            underline: r.link.is_some(),
            font: if r.code {
                "Consolas".into()
            } else {
                font.to_string()
            },
        });
        let text = expand_breaks(&escape_xml(&r.text));
        let span = format!(
            r#"<text:span text:style-name="{}">{}</text:span>"#,
            style, text
        );
        if let Some(url) = &r.link {
            s.push_str(&format!(
                r#"<text:a xlink:type="simple" xlink:href="{}">{}</text:a>"#,
                escape_xml(url),
                span,
            ));
        } else {
            s.push_str(&span);
        }
    }
    if s.is_empty() {
        s.push_str(r#"<text:span/>"#);
    }
    s
}

fn expand_breaks(s: &str) -> String {
    s.replace('\t', "<text:tab/>")
}

/// ODF collapses runs of whitespace by default. For code, preserve leading
/// whitespace and any run of 2+ spaces as `<text:s text:c="N"/>` elements.
fn code_text(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut at_start = true;
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' {
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let count = j - i;
            if at_start || count >= 2 {
                if count == 1 {
                    out.push_str("<text:s/>");
                } else {
                    out.push_str(&format!(r#"<text:s text:c="{}"/>"#, count));
                }
            } else {
                out.push(' ');
            }
            i = j;
            at_start = false;
        } else if c == '\t' {
            out.push_str("<text:tab/>");
            i += 1;
            at_start = false;
        } else {
            match c {
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '&' => out.push_str("&amp;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                c if (c as u32) < 0x20 && c != '\n' && c != '\r' => {}
                c => out.push(c),
            }
            i += 1;
            at_start = false;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

fn list_frame(
    reg: &mut Registry,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    items: &[ListItem],
    theme: &Theme,
) -> String {
    let gr = reg.graphic_name(GraphicStyleKey {
        fill: None,
        no_fill: true,
        stroke: None,
        corner_radius_emu: 0,
        text_anchor: "top",
        text_align: "left",
    });
    let para = reg.para_name(ParaStyleKey {
        align: "start",
        line_spacing_pct: 120,
    });
    let bullet_style = reg.text_name(TextStyleKey {
        color: theme.accent.clone(),
        size_pt: theme.body_size / 100,
        bold: false,
        italic: false,
        strike: false,
        underline: false,
        font: theme.body_font.clone(),
    });

    let mut paras = String::new();
    let mut ordered_counters: Vec<u32> = Vec::new();

    for item in items {
        let lvl = item.level.min(8) as usize;
        while ordered_counters.len() <= lvl {
            ordered_counters.push(0);
        }
        ordered_counters[lvl] += 1;
        for c in &mut ordered_counters[lvl + 1..] {
            *c = 0;
        }

        let bullet_char = if item.ordered {
            format!("{}.", ordered_counters[lvl])
        } else {
            match lvl {
                0 => "●",
                1 => "○",
                2 => "▪",
                _ => "·",
            }
            .to_string()
        };

        let indent_spaces = "  ".repeat(lvl);
        let bullet_run = Run {
            text: format!("{indent_spaces}"),
            bold: false,
            italic: false,
            code: false,
            strike: false,
            link: None,
        };

        let bullet_xml = format!(
            r#"<text:span text:style-name="{b}">{bc}</text:span><text:span> </text:span>"#,
            b = bullet_style,
            bc = escape_xml(&bullet_char),
        );

        let indent_xml = if !indent_spaces.is_empty() {
            runs_to_spans(
                reg,
                &[bullet_run],
                theme.body_size,
                &theme.body_color,
                false,
                false,
                &theme.body_font,
            )
        } else {
            String::new()
        };

        let body_xml = runs_to_spans(
            reg,
            &item.runs,
            theme.body_size,
            &theme.body_color,
            false,
            false,
            &theme.body_font,
        );

        paras.push_str(&format!(
            r#"<text:p text:style-name="{p}">{ind}{bul}{body}</text:p>"#,
            p = para,
            ind = indent_xml,
            bul = bullet_xml,
            body = body_xml,
        ));
    }

    format!(
        r#"<draw:frame draw:style-name="{g}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:text-box>{p}</draw:text-box></draw:frame>"#,
        g = gr,
        x = cm(x),
        y = cm(y),
        w = cm(w),
        h = cm(h),
        p = paras,
    )
}

// ---------------------------------------------------------------------------
// Code block (with syntax highlighting)
// ---------------------------------------------------------------------------

fn code_block(
    reg: &mut Registry,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    lines: &[String],
    title: Option<&str>,
    lang: Option<&str>,
    line_numbers: bool,
    start_line: usize,
    theme: &Theme,
) -> String {
    let mut s = String::new();
    let title_h: u32 = if title.is_some() || lang.is_some() {
        320000
    } else {
        0
    };
    let code_y = y + title_h;
    let code_h = h.saturating_sub(title_h);

    if title_h > 0 {
        let title_g = reg.graphic_name(GraphicStyleKey {
            fill: Some(theme.divider.clone()),
            no_fill: false,
            stroke: Some((theme.divider.clone(), 6350)),
            corner_radius_emu: 45000,
            text_anchor: "middle",
            text_align: "left",
        });
        let para = reg.para_name(ParaStyleKey {
            align: "start",
            line_spacing_pct: 120,
        });
        let title_span = title.map(|t| {
            let st = reg.text_name(TextStyleKey {
                color: theme.title_color.clone(),
                size_pt: 12,
                bold: true,
                italic: false,
                strike: false,
                underline: false,
                font: theme.mono_font.clone(),
            });
            format!(
                r#"<text:span text:style-name="{s}">{t}</text:span>"#,
                s = st,
                t = escape_xml(t),
            )
        });
        let lang_span = lang.map(|l| {
            let st = reg.text_name(TextStyleKey {
                color: theme.muted_color.clone(),
                size_pt: 11,
                bold: false,
                italic: false,
                strike: false,
                underline: false,
                font: theme.body_font.clone(),
            });
            if title.is_some() {
                format!(
                    r#"<text:span text:style-name="{s}">  ·  {l}</text:span>"#,
                    s = st,
                    l = escape_xml(l),
                )
            } else {
                format!(
                    r#"<text:span text:style-name="{s}">{l}</text:span>"#,
                    s = st,
                    l = escape_xml(l),
                )
            }
        });
        let combined: String = [title_span, lang_span]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("");
        s.push_str(&format!(
            r#"<draw:rect draw:style-name="{g}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{th}"><text:p text:style-name="{p}">{c}</text:p></draw:rect>"#,
            g = title_g,
            x = cm(x),
            y = cm(y),
            w = cm(w),
            th = cm(title_h),
            p = para,
            c = combined,
        ));
    }

    let body_g = reg.graphic_name(GraphicStyleKey {
        fill: Some(theme.code_bg.clone()),
        no_fill: false,
        stroke: Some((theme.divider.clone(), 6350)),
        corner_radius_emu: 45000,
        text_anchor: "top",
        text_align: "left",
    });

    let highlighted: Vec<Vec<Token>> = syntax::tokenize(lines, lang);
    let last_line = start_line.saturating_add(lines.len().saturating_sub(1));
    let gutter_w = if line_numbers {
        Some(last_line.to_string().len())
    } else {
        None
    };

    let mut paras = String::new();
    let para = reg.para_name(ParaStyleKey {
        align: "start",
        line_spacing_pct: 115,
    });
    for (idx, line_tokens) in highlighted.iter().enumerate() {
        let mut runs_xml = String::new();
        if let Some(width) = gutter_w {
            let n = format!("{:>w$}", start_line + idx, w = width);
            let st = reg.text_name(TextStyleKey {
                color: theme.muted_color.clone(),
                size_pt: theme.code_size / 100,
                bold: false,
                italic: false,
                strike: false,
                underline: false,
                font: theme.mono_font.clone(),
            });
            runs_xml.push_str(&format!(
                r#"<text:span text:style-name="{s}">{t}<text:s text:c="2"/></text:span>"#,
                s = st,
                t = escape_xml(&n),
            ));
        }
        if line_tokens.is_empty() || line_tokens.iter().all(|t| t.text.is_empty()) {
            let st = reg.text_name(TextStyleKey {
                color: theme.code_text.clone(),
                size_pt: theme.code_size / 100,
                bold: false,
                italic: false,
                strike: false,
                underline: false,
                font: theme.mono_font.clone(),
            });
            runs_xml.push_str(&format!(
                r#"<text:span text:style-name="{s}"><text:s/></text:span>"#,
                s = st
            ));
        } else {
            for token in line_tokens {
                if token.text.is_empty() {
                    continue;
                }
                let style = theme.syntax_style(token.kind);
                let st = reg.text_name(TextStyleKey {
                    color: style.color.clone(),
                    size_pt: theme.code_size / 100,
                    bold: style.bold,
                    italic: style.italic,
                    strike: false,
                    underline: false,
                    font: theme.mono_font.clone(),
                });
                runs_xml.push_str(&format!(
                    r#"<text:span text:style-name="{s}">{t}</text:span>"#,
                    s = st,
                    t = code_text(&token.text),
                ));
            }
        }
        paras.push_str(&format!(
            r#"<text:p text:style-name="{p}">{r}</text:p>"#,
            p = para,
            r = runs_xml,
        ));
    }

    s.push_str(&format!(
        r#"<draw:frame draw:style-name="{g}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:text-box>{p}</draw:text-box></draw:frame>"#,
        g = body_g,
        x = cm(x),
        y = cm(code_y),
        w = cm(w),
        h = cm(code_h),
        p = paras,
    ));
    s
}

// ---------------------------------------------------------------------------
// Table (simple grid via positioned text-boxes)
// ---------------------------------------------------------------------------

fn render_table(
    reg: &mut Registry,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    headers: &[Vec<Run>],
    rows: &[Vec<Vec<Run>>],
    theme: &Theme,
) -> String {
    let cols = headers
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if cols == 0 {
        return String::new();
    }
    let col_widths = table_column_widths(headers, rows, w);
    let row_heights = fit_table_row_heights(
        table_natural_row_heights(headers, rows, &col_widths, theme),
        h,
    );

    let mut s = String::new();

    for c in 0..cols {
        let cx = x + col_widths.iter().take(c).sum::<u32>();
        let max_chars = table_chars_per_line(col_widths[c], table_header_size(theme));
        s.push_str(&cell(
            reg,
            cx,
            y,
            col_widths[c],
            row_heights[0],
            headers.get(c).map(|r| r.as_slice()).unwrap_or(&[]),
            &theme.accent,
            &theme.on_accent,
            table_header_size(theme),
            true,
            max_chars,
            theme,
        ));
    }
    let table_band_bg = theme.table_band_bg();
    for (i, row) in rows.iter().enumerate() {
        let ry = y + row_heights.iter().take(i + 1).sum::<u32>();
        let banded = i % 2 == 1;
        let bg = if banded {
            table_band_bg.clone()
        } else {
            theme.bg.clone()
        };
        for c in 0..cols {
            let cx = x + col_widths.iter().take(c).sum::<u32>();
            let max_chars = table_chars_per_line(col_widths[c], table_body_size(theme));
            s.push_str(&cell(
                reg,
                cx,
                ry,
                col_widths[c],
                row_heights[i + 1],
                row.get(c).map(|r| r.as_slice()).unwrap_or(&[]),
                &bg,
                &theme.body_color,
                table_body_size(theme),
                false,
                max_chars,
                theme,
            ));
        }
    }
    s
}

fn cell(
    reg: &mut Registry,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    runs: &[Run],
    fill: &str,
    text_color: &str,
    size_centipt: u32,
    bold: bool,
    max_chars: usize,
    theme: &Theme,
) -> String {
    let g = reg.graphic_name(GraphicStyleKey {
        fill: Some(fill.to_string()),
        no_fill: false,
        stroke: Some((theme.divider.clone(), 6350)),
        corner_radius_emu: 0,
        text_anchor: "middle",
        text_align: "left",
    });
    let para = reg.para_name(ParaStyleKey {
        align: "start",
        line_spacing_pct: 120,
    });
    let wrapped = wrap_runs_for_table(runs, max_chars);
    let mut wrapped_runs = Vec::new();
    for (idx, line) in wrapped.into_iter().enumerate() {
        if idx > 0 {
            wrapped_runs.push(Run::plain("\n"));
        }
        wrapped_runs.extend(line);
    }
    let runs_xml = runs_to_spans(
        reg,
        &wrapped_runs,
        size_centipt,
        text_color,
        bold,
        false,
        &theme.body_font,
    );
    format!(
        r#"<draw:rect draw:style-name="{g}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><text:p text:style-name="{p}">{r}</text:p></draw:rect>"#,
        g = g,
        x = cm(x),
        y = cm(y),
        w = cm(w),
        h = cm(h),
        p = para,
        r = runs_xml,
    )
}

fn table_height_emu(headers: &[Vec<Run>], rows: &[Vec<Vec<Run>>], w: u32, theme: &Theme) -> u32 {
    let col_widths = table_column_widths(headers, rows, w);
    table_natural_row_heights(headers, rows, &col_widths, theme)
        .iter()
        .sum::<u32>()
        + 80000
}

fn table_header_size(theme: &Theme) -> u32 {
    theme.body_size.saturating_sub(200)
}

fn table_body_size(theme: &Theme) -> u32 {
    theme.body_size.saturating_sub(400)
}

fn table_natural_row_heights(
    headers: &[Vec<Run>],
    rows: &[Vec<Vec<Run>>],
    col_widths: &[u32],
    theme: &Theme,
) -> Vec<u32> {
    let mut heights = Vec::with_capacity(rows.len() + 1);
    heights.push(table_row_height(
        headers,
        col_widths,
        table_header_size(theme),
        380000,
    ));
    for row in rows {
        heights.push(table_row_height(
            row,
            col_widths,
            table_body_size(theme),
            340000,
        ));
    }
    heights
}

fn table_column_widths(headers: &[Vec<Run>], rows: &[Vec<Vec<Run>>], w: u32) -> Vec<u32> {
    let cols = headers
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if cols == 0 {
        return Vec::new();
    }
    let equal = || {
        let base = w / cols as u32;
        let mut widths = vec![base; cols];
        if let Some(last) = widths.last_mut() {
            *last = w.saturating_sub(base * (cols as u32 - 1));
        }
        widths
    };
    let desired_min = if cols <= 2 {
        w * 18 / 100
    } else {
        w * 10 / 100
    };
    let min_w = desired_min.min(w / cols as u32);
    let total_min = min_w.saturating_mul(cols as u32);
    if total_min >= w {
        return equal();
    }

    let mut weights = Vec::with_capacity(cols);
    for c in 0..cols {
        let mut max_len = headers
            .get(c)
            .map(|runs| runs_text(runs).chars().count())
            .unwrap_or(0);
        for row in rows {
            max_len = max_len.max(
                row.get(c)
                    .map(|runs| runs_text(runs).chars().count())
                    .unwrap_or(0),
            );
        }
        weights.push(max_len.clamp(6, 90) as u32);
    }
    let weight_total = weights.iter().sum::<u32>().max(1);
    let remaining = w - total_min;
    let mut widths: Vec<u32> = weights
        .iter()
        .map(|weight| min_w + remaining * *weight / weight_total)
        .collect();
    let used = widths.iter().take(cols.saturating_sub(1)).sum::<u32>();
    if let Some(last) = widths.last_mut() {
        *last = w.saturating_sub(used);
    }
    widths
}

fn table_row_height(cells: &[Vec<Run>], col_widths: &[u32], size_centipt: u32, min_h: u32) -> u32 {
    let lines = col_widths
        .iter()
        .enumerate()
        .map(|(idx, width)| {
            let max_chars = table_chars_per_line(*width, size_centipt);
            wrap_runs_for_table(
                cells.get(idx).map(|runs| runs.as_slice()).unwrap_or(&[]),
                max_chars,
            )
            .len()
        })
        .max()
        .unwrap_or(1) as u32;
    let line_h = (size_centipt as f32 * 160.0) as u32;
    min_h.max(lines * line_h + 120000)
}

fn table_chars_per_line(col_w: u32, size_centipt: u32) -> usize {
    let inner = col_w.saturating_sub(180000).max(300000) as f32;
    let char_w = (size_centipt as f32 / 100.0) * 12700.0 * 0.52;
    (inner / char_w).floor().max(8.0) as usize
}

fn fit_table_row_heights(mut heights: Vec<u32>, target_h: u32) -> Vec<u32> {
    let natural = heights.iter().sum::<u32>();
    if natural == 0 || target_h == 0 || natural <= target_h {
        return heights;
    }
    for h in &mut heights {
        *h = ((*h as u64 * target_h as u64) / natural as u64) as u32;
    }
    heights
}

fn wrap_runs_for_table(runs: &[Run], max_chars: usize) -> Vec<Vec<Run>> {
    let max_chars = max_chars.max(8);
    let mut lines: Vec<Vec<Run>> = vec![Vec::new()];
    let mut current_len = 0usize;
    for run in runs {
        for word in run.text.split_whitespace() {
            let word_len = word.chars().count();
            if current_len > 0 && current_len + 1 + word_len > max_chars {
                lines.push(Vec::new());
                current_len = 0;
            }
            if current_len > 0 {
                lines
                    .last_mut()
                    .unwrap()
                    .push(run_with_text(run, " ".to_string()));
                current_len += 1;
            }
            if word_len <= max_chars {
                lines
                    .last_mut()
                    .unwrap()
                    .push(run_with_text(run, word.to_string()));
                current_len += word_len;
            } else {
                let mut chunk = String::new();
                let mut chunk_len = 0usize;
                for ch in word.chars() {
                    if chunk_len == max_chars {
                        lines
                            .last_mut()
                            .unwrap()
                            .push(run_with_text(run, std::mem::take(&mut chunk)));
                        lines.push(Vec::new());
                        chunk_len = 0;
                        current_len = 0;
                    }
                    chunk.push(ch);
                    chunk_len += 1;
                }
                if !chunk.is_empty() {
                    lines.last_mut().unwrap().push(run_with_text(run, chunk));
                    current_len += chunk_len;
                }
            }
        }
    }
    if lines.iter().all(|line| line.is_empty()) {
        vec![vec![Run::plain("")]]
    } else {
        lines
    }
}

fn run_with_text(run: &Run, text: String) -> Run {
    let mut out = run.clone();
    out.text = text;
    out
}

// ---------------------------------------------------------------------------
// Image
// ---------------------------------------------------------------------------

fn render_image(
    reg: &mut Registry,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    src: &str,
    alt: &str,
    theme: &Theme,
    imgs: &Imgs,
) -> String {
    let mut s = String::new();
    let caption_alt = crate::math::visible_image_alt(src, alt);
    let caption_h = if caption_alt.is_empty() { 0 } else { 260000 };
    let image_h = h.saturating_sub(caption_h + 80000);

    let (display_w, display_h) = if let Some((iw, ih)) = imgs.dims(src) {
        fit_image_for_block(src, alt, iw, ih, w, image_h, theme.slide_h)
    } else {
        (w, image_h)
    };

    if let Some(href) = imgs.href(src) {
        let g = reg.graphic_name(GraphicStyleKey {
            fill: None,
            no_fill: true,
            stroke: None,
            corner_radius_emu: 0,
            text_anchor: "top",
            text_align: "left",
        });
        let img_x = image_x_for_block(src, alt, x, w, display_w);
        s.push_str(&format!(
            r#"<draw:frame draw:style-name="{g}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:image xlink:href="{href}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>"#,
            g = g,
            x = cm(img_x),
            y = cm(y),
            w = cm(display_w),
            h = cm(display_h),
            href = escape_xml(&href),
        ));
    }

    if !caption_alt.is_empty() {
        let caption_y = y + display_h + 60000;
        s.push_str(&text_frame(
            reg,
            x,
            caption_y,
            w,
            caption_h,
            "top",
            "center",
            &[Run::plain(caption_alt)],
            1300,
            &theme.muted_color,
            false,
            true,
            &theme.body_font,
        ));
    }
    s
}

fn render_full_page_image(
    src: &str,
    _alt: &str,
    theme: &Theme,
    imgs: &Imgs,
    reg: &mut Registry,
) -> String {
    let (display_w, display_h) = if let Some((iw, ih)) = imgs.dims(src) {
        fit_image(iw, ih, theme.slide_w, theme.slide_h)
    } else {
        (theme.slide_w, theme.slide_h)
    };
    let x = theme.slide_w.saturating_sub(display_w) / 2;
    let y = theme.slide_h.saturating_sub(display_h) / 2;

    if let Some(href) = imgs.href(src) {
        let g = reg.graphic_name(GraphicStyleKey {
            fill: None,
            no_fill: true,
            stroke: None,
            corner_radius_emu: 0,
            text_anchor: "top",
            text_align: "left",
        });
        format!(
            r#"<draw:frame draw:style-name="{g}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"><draw:image xlink:href="{href}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>"#,
            g = g,
            x = cm(x),
            y = cm(y),
            w = cm(display_w),
            h = cm(display_h),
            href = escape_xml(&href),
        )
    } else {
        rect(reg, 0, 0, theme.slide_w, theme.slide_h, &theme.divider)
    }
}

fn render_full_page_code(
    lines: &[String],
    lang: Option<&str>,
    theme: &Theme,
    reg: &mut Registry,
) -> String {
    let math_markup = crate::math::is_markup_text_language(lang);
    let rendered_lines = crate::math::translate_markup_lines(lines, lang);
    let margin_x = if theme.portrait { 280000 } else { 360000 };
    let margin_y = if theme.portrait { 320000 } else { 300000 };
    let w = theme.slide_w.saturating_sub(margin_x * 2);
    let h = theme.slide_h.saturating_sub(margin_y * 2);
    let base_size = theme.code_size.min(if theme.portrait { 850 } else { 950 });
    let line_count = rendered_lines.len().max(1) as f32;
    let max_chars = rendered_lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let char_factor = if math_markup { 0.50_f32 } else { 0.58_f32 };
    let line_factor = if math_markup { 1.10_f32 } else { 1.18_f32 };
    let size_by_width = ((w as f32 / 12_700.0) / (max_chars * char_factor) * 100.0) as u32;
    let size_by_height = ((h as f32 / 12_700.0) / (line_count * line_factor) * 100.0) as u32;
    let size = base_size.min(size_by_width).min(size_by_height).max(450);
    let mut runs = Vec::with_capacity(rendered_lines.len().saturating_mul(2));
    for (idx, line) in rendered_lines.iter().enumerate() {
        if idx > 0 {
            runs.push(Run::plain("\n"));
        }
        runs.push(Run::plain(line));
    }
    text_frame(
        reg,
        margin_x,
        margin_y,
        w,
        h,
        "top",
        if math_markup { "center" } else { "left" },
        &runs,
        size,
        &theme.title_color,
        false,
        math_markup,
        if math_markup {
            &theme.body_font
        } else {
            &theme.mono_font
        },
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rect(reg: &mut Registry, x: u32, y: u32, w: u32, h: u32, color: &str) -> String {
    let g = reg.graphic_name(GraphicStyleKey {
        fill: Some(color.to_string()),
        no_fill: false,
        stroke: None,
        corner_radius_emu: 0,
        text_anchor: "top",
        text_align: "left",
    });
    format!(
        r#"<draw:rect draw:style-name="{g}" svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}"/>"#,
        g = g,
        x = cm(x),
        y = cm(y),
        w = cm(w),
        h = cm(h),
    )
}

fn cm(emu: u32) -> String {
    format!("{:.4}cm", emu as f64 / 360000.0)
}

fn progress_width(width: u32, num: usize, total: usize) -> u32 {
    if width == 0 || total == 0 {
        return 0;
    }
    ((width as u64 * num.max(1) as u64) / total as u64)
        .min(width as u64)
        .max(1) as u32
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

fn letterspaced(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        for u in c.to_uppercase() {
            out.push(u);
        }
    }
    out
}

fn author_date(author: Option<&str>, date: Option<&str>) -> Option<String> {
    let mut s = String::new();
    if let Some(a) = author {
        s.push_str(a);
    }
    if author.is_some() && date.is_some() {
        s.push_str("  ·  ");
    }
    if let Some(d) = date {
        s.push_str(d);
    }
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
