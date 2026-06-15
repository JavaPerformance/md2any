//! PowerPoint OOXML (`.pptx`) writer.
//!
//! Produces a fully-native PPTX file: ZIP archive of XML parts following
//! the OOXML spec (ECMA-376 / ISO/IEC 29500). Opens unmodified in
//! PowerPoint, Keynote, LibreOffice Impress, and Google Slides.
//!
//! The implementation is intentionally low-level — we write the XML and
//! relationship tables directly rather than depending on a generic OOXML
//! library. Three reasons:
//!   1. No transitive C/Rust dep tree.
//!   2. Faster builds (~3 ms for a 100-slide deck).
//!   3. Total control over the output — round-trips through every tool we
//!      tested without warnings or auto-repair prompts.
//!
//! Architecture: [`write()`] is the entry point. It collects all referenced
//! images upfront (so they get stable rel IDs), then renders each slide
//! into a `<p:sld>` XML document plus its relationship file, then bundles
//! everything into a ZIP. Per-slide rendering is dispatched on [`SlideKind`]
//! to layout-specific helpers in the lower half of this file.

use crate::image::{self, ImageMeta};
use crate::ir::*;
use crate::layout::{Layout, LayoutKind};
use crate::syntax::{self, Token};
use crate::theme::Theme;
use anyhow::{Context, Result};
use std::collections::{BTreeSet, HashMap};
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::FileOptions;
use zip::CompressionMethod;

const REL_LAYOUT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const REL_NOTES_SLIDE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide";
const REL_HYPERLINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

thread_local! {
    /// Per-slide link buffer. `run_xml` pushes URLs and emits `r:id="MD2LINK<n>"`
    /// placeholders; `slide_xml` post-processes the rendered shapes to register
    /// each URL with rels and rewrite the placeholders into real `rId` values.
    static LINK_BUFFER: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::new());

    /// Per-slide body alignment (`"l"`/`"ctr"`/`"r"`), set from the slide's
    /// `align=` hint and applied to body paragraphs/headings in `render_blocks`.
    static SLIDE_ALIGN: std::cell::RefCell<&'static str> = const { std::cell::RefCell::new("l") };

    /// Per-slide left-column fraction from the `width=` hint (`None` = even).
    static SLIDE_COL_FRAC: std::cell::RefCell<Option<f32>> = const { std::cell::RefCell::new(None) };

    /// Per-slide vertical alignment (`"top"`/`"center"`/`"bottom"`) from the
    /// `valign=` hint, used to offset the content band when it underfills.
    static SLIDE_VALIGN: std::cell::RefCell<&'static str> = const { std::cell::RefCell::new("top") };

    /// Per-slide body-text scale (`text-scale`); 1.0 = theme default.
    static SLIDE_TEXT_SCALE: std::cell::RefCell<f32> = const { std::cell::RefCell::new(1.0) };
}

fn slide_align() -> &'static str {
    SLIDE_ALIGN.with(|a| *a.borrow())
}

/// Scale a centipoint font size by the active slide's text-scale.
fn tscaled(size: u32) -> u32 {
    (size as f32 * SLIDE_TEXT_SCALE.with(|s| *s.borrow())) as u32
}

fn slide_col_frac() -> Option<f32> {
    SLIDE_COL_FRAC.with(|f| *f.borrow())
}

fn slide_valign() -> &'static str {
    SLIDE_VALIGN.with(|v| *v.borrow())
}

fn take_link_buffer() -> Vec<String> {
    LINK_BUFFER.with(|b| std::mem::take(&mut *b.borrow_mut()))
}

fn resolve_link_placeholders(xml: &str, urls: &[String], rels: &mut SlideRels) -> String {
    if urls.is_empty() {
        return xml.to_string();
    }
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    let needle = "MD2LINK";
    while let Some(pos) = rest.find(needle) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + needle.len()..];
        let digits_end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        if digits_end == 0 {
            out.push_str(&rest[pos..pos + needle.len()]);
            rest = after;
            continue;
        }
        let idx: usize = after[..digits_end].parse().unwrap_or(usize::MAX);
        if let Some(url) = urls.get(idx) {
            let rid = rels.add_external(REL_HYPERLINK, url);
            out.push_str(&rid);
        } else {
            out.push_str(&rest[pos..pos + needle.len() + digits_end]);
        }
        rest = &after[digits_end..];
    }
    out.push_str(rest);
    out
}

struct SlideRels {
    entries: Vec<(String, &'static str, String, bool)>,
    next: usize,
}

impl SlideRels {
    fn new() -> Self {
        let mut s = SlideRels {
            entries: Vec::new(),
            next: 1,
        };
        s.add(REL_LAYOUT, "../slideLayouts/slideLayout1.xml");
        s
    }
    fn add(&mut self, rel_type: &'static str, target: &str) -> String {
        self.add_full(rel_type, target, false)
    }
    fn add_external(&mut self, rel_type: &'static str, target: &str) -> String {
        self.add_full(rel_type, target, true)
    }
    fn add_full(&mut self, rel_type: &'static str, target: &str, external: bool) -> String {
        let id = format!("rId{}", self.next);
        self.next += 1;
        self.entries
            .push((id.clone(), rel_type, target.to_string(), external));
        id
    }
    fn serialize(&self) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n",
        );
        for (id, ty, target, external) in &self.entries {
            if *external {
                s.push_str(&format!(
                    "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\" TargetMode=\"External\"/>\n",
                    id,
                    ty,
                    escape_xml(target),
                ));
            } else {
                s.push_str(&format!(
                    "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"/>\n",
                    id,
                    ty,
                    escape_xml(target),
                ));
            }
        }
        s.push_str("</Relationships>");
        s
    }
}

pub struct Imgs<'a> {
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
    fn logo(&self) -> Option<(&str, u32, u32)> {
        let k = self.logo_key?;
        let i = *self.by_src.get(k)?;
        Some((k, self.metas[i].width, self.metas[i].height))
    }
}

/// Serialise the slide deck to PowerPoint OOXML bytes.
///
/// * `slides`       — paginated slide tree from [`crate::paginate::paginate`].
/// * `theme`        — colours, fonts, slide dimensions in EMU.
/// * `layout`       — which of the four built-in slide skeletons to use.
/// * `deck_title`   — used in the document core-properties and in
///                    layout footers that reference the deck title.
/// * `author`       — written to `docProps/core.xml`.
/// * `base_dir`     — directory containing the markdown file; relative
///                    image paths resolve from here.
/// * `logo`         — optional footer logo (PNG/JPEG path).
/// * `transition`   — optional slide-to-slide transition name.
/// * `transition_dur` — transition duration in seconds.
/// * `direction`    — `Some("rtl")` flips paragraph direction throughout.
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

    let extensions: BTreeSet<&str> = metas.iter().map(|m| m.ext).collect();
    let has_notes = slides.iter().any(|s| s.notes.is_some());

    let buf: Vec<u8> = Vec::new();
    let cursor = Cursor::new(buf);
    let mut zip = zip::ZipWriter::new(cursor);
    let stored = FileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = FileOptions::default().compression_method(CompressionMethod::Deflated);

    write_file(
        &mut zip,
        "[Content_Types].xml",
        &content_types(slides, &extensions, has_notes),
        stored,
    )?;
    write_file(&mut zip, "_rels/.rels", &root_rels(), stored)?;
    write_file(
        &mut zip,
        "docProps/app.xml",
        &app_xml(slides.len(), deck_title),
        deflated,
    )?;
    write_file(
        &mut zip,
        "docProps/core.xml",
        &core_xml(deck_title, author),
        deflated,
    )?;

    write_file(
        &mut zip,
        "ppt/theme/theme1.xml",
        &theme_xml(theme),
        deflated,
    )?;
    write_file(
        &mut zip,
        "ppt/slideMasters/slideMaster1.xml",
        &slide_master_xml(theme),
        deflated,
    )?;
    write_file(
        &mut zip,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        &slide_master_rels(),
        stored,
    )?;
    write_file(
        &mut zip,
        "ppt/slideLayouts/slideLayout1.xml",
        &slide_layout_xml(),
        deflated,
    )?;
    write_file(
        &mut zip,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        &slide_layout_rels(),
        stored,
    )?;

    write_file(
        &mut zip,
        "ppt/presentation.xml",
        &presentation_xml(slides.len(), theme, has_notes),
        deflated,
    )?;
    write_file(
        &mut zip,
        "ppt/_rels/presentation.xml.rels",
        &presentation_rels(slides.len(), has_notes),
        stored,
    )?;

    if has_notes {
        write_file(
            &mut zip,
            "ppt/notesMasters/notesMaster1.xml",
            &notes_master_xml(theme),
            deflated,
        )?;
        write_file(
            &mut zip,
            "ppt/notesMasters/_rels/notesMaster1.xml.rels",
            &notes_master_rels(),
            stored,
        )?;
    }

    for (i, slide) in slides.iter().enumerate() {
        let num = i + 1;
        let mut rels = SlideRels::new();
        let mut xml = slide_xml(
            slide,
            num,
            slides.len(),
            theme,
            layout,
            deck_title,
            &imgs,
            &mut rels,
            transition,
            transition_dur,
        );
        if rtl {
            xml = apply_rtl_pptx(&xml);
        }
        if slide.notes.is_some() {
            rels.add(
                REL_NOTES_SLIDE,
                &format!("../notesSlides/notesSlide{}.xml", num),
            );
        }
        write_file(
            &mut zip,
            &format!("ppt/slides/slide{}.xml", num),
            &xml,
            deflated,
        )?;
        write_file(
            &mut zip,
            &format!("ppt/slides/_rels/slide{}.xml.rels", num),
            &rels.serialize(),
            stored,
        )?;

        if let Some(notes) = &slide.notes {
            write_file(
                &mut zip,
                &format!("ppt/notesSlides/notesSlide{}.xml", num),
                &notes_slide_xml(notes, theme),
                deflated,
            )?;
            write_file(
                &mut zip,
                &format!("ppt/notesSlides/_rels/notesSlide{}.xml.rels", num),
                &notes_slide_rels(num),
                stored,
            )?;
        }
    }

    for (idx, meta) in metas.iter().enumerate() {
        let opts = if meta.ext == "png" || meta.ext == "jpeg" {
            stored
        } else {
            deflated
        };
        zip.start_file(&format!("ppt/media/image{}.{}", idx + 1, meta.ext), opts)?;
        zip.write_all(&meta.bytes)?;
    }

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
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

fn content_types(slides: &[Slide], extensions: &BTreeSet<&str>, has_notes: bool) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
"#,
    );
    for ext in extensions {
        let mime = match *ext {
            "png" => "image/png",
            "jpeg" => "image/jpeg",
            _ => continue,
        };
        s.push_str(&format!(
            "<Default Extension=\"{}\" ContentType=\"{}\"/>\n",
            ext, mime
        ));
    }
    s.push_str(
        r#"<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
"#,
    );
    if has_notes {
        s.push_str(r#"<Override PartName="/ppt/notesMasters/notesMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml"/>
"#);
    }
    for (i, slide) in slides.iter().enumerate() {
        let n = i + 1;
        s.push_str(&format!(
            r#"<Override PartName="/ppt/slides/slide{}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
"#,
            n
        ));
        if slide.notes.is_some() {
            s.push_str(&format!(
                r#"<Override PartName="/ppt/notesSlides/notesSlide{}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/>
"#,
                n
            ));
        }
    }
    s.push_str("</Types>");
    s
}

fn root_rels() -> String {
    String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>"#,
    )
}

fn app_xml(n: usize, deck_title: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">
<Application>md2any</Application>
<Slides>{n}</Slides>
<TitlesOfParts>
<vt:vector size="1" baseType="lpstr"><vt:lpstr>{title}</vt:lpstr></vt:vector>
</TitlesOfParts>
<Company></Company>
<AppVersion>16.0000</AppVersion>
</Properties>"#,
        n = n,
        title = escape_xml(deck_title),
    )
}

fn core_xml(title: &str, author: &str) -> String {
    // Stable timestamp — see odp::build_meta for the rationale.
    // Byte-identical output across re-renders keeps committed example
    // bundles from churning in git.
    let now = "2026-01-01T00:00:00Z";
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<dc:title>{title}</dc:title>
<dc:creator>{author}</dc:creator>
<cp:lastModifiedBy>{author}</cp:lastModifiedBy>
<cp:revision>1</cp:revision>
<dcterms:created xsi:type="dcterms:W3CDTF">{now}</dcterms:created>
<dcterms:modified xsi:type="dcterms:W3CDTF">{now}</dcterms:modified>
</cp:coreProperties>"#,
        title = escape_xml(title),
        author = escape_xml(author),
        now = now,
    )
}

fn theme_xml(theme: &Theme) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="md2any">
<a:themeElements>
<a:clrScheme name="md2any">
<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
<a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
<a:dk2><a:srgbClr val="{title_color}"/></a:dk2>
<a:lt2><a:srgbClr val="{divider}"/></a:lt2>
<a:accent1><a:srgbClr val="{accent}"/></a:accent1>
<a:accent2><a:srgbClr val="{title_color}"/></a:accent2>
<a:accent3><a:srgbClr val="{body_color}"/></a:accent3>
<a:accent4><a:srgbClr val="{muted_color}"/></a:accent4>
<a:accent5><a:srgbClr val="{divider}"/></a:accent5>
<a:accent6><a:srgbClr val="{accent_soft}"/></a:accent6>
<a:hlink><a:srgbClr val="{link}"/></a:hlink>
<a:folHlink><a:srgbClr val="{muted_color}"/></a:folHlink>
</a:clrScheme>
<a:fontScheme name="md2any">
<a:majorFont>
<a:latin typeface="{title_font}"/>
<a:ea typeface=""/>
<a:cs typeface=""/>
</a:majorFont>
<a:minorFont>
<a:latin typeface="{body_font}"/>
<a:ea typeface=""/>
<a:cs typeface=""/>
</a:minorFont>
</a:fontScheme>
<a:fmtScheme name="md2any">
<a:fillStyleLst>
<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
</a:fillStyleLst>
<a:lnStyleLst>
<a:ln w="9525" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln>
<a:ln w="25400" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln>
<a:ln w="38100" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln>
</a:lnStyleLst>
<a:effectStyleLst>
<a:effectStyle><a:effectLst/></a:effectStyle>
<a:effectStyle><a:effectLst/></a:effectStyle>
<a:effectStyle><a:effectLst/></a:effectStyle>
</a:effectStyleLst>
<a:bgFillStyleLst>
<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
</a:bgFillStyleLst>
</a:fmtScheme>
</a:themeElements>
</a:theme>"#,
        title_color = theme.title_color,
        body_color = theme.body_color,
        muted_color = theme.muted_color,
        accent = theme.accent,
        accent_soft = theme.accent_soft,
        divider = theme.divider,
        link = theme.link,
        title_font = escape_xml(&theme.title_font),
        body_font = escape_xml(&theme.body_font),
    )
}

fn slide_master_xml(theme: &Theme) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cSld>
<p:bg><p:bgPr><a:solidFill><a:srgbClr val="{bg}"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>
<p:spTree>
<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
</p:spTree>
</p:cSld>
<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>
<p:sldLayoutIdLst>
<p:sldLayoutId id="2147483649" r:id="rId1"/>
</p:sldLayoutIdLst>
<p:txStyles>
<p:titleStyle>
<a:lvl1pPr algn="l" rtl="0"><a:defRPr sz="3200" b="1"><a:solidFill><a:srgbClr val="{title_color}"/></a:solidFill><a:latin typeface="{title_font}"/></a:defRPr></a:lvl1pPr>
</p:titleStyle>
<p:bodyStyle>
<a:lvl1pPr><a:defRPr sz="1800"><a:solidFill><a:srgbClr val="{body_color}"/></a:solidFill><a:latin typeface="{body_font}"/></a:defRPr></a:lvl1pPr>
<a:lvl2pPr><a:defRPr sz="1600"><a:solidFill><a:srgbClr val="{body_color}"/></a:solidFill><a:latin typeface="{body_font}"/></a:defRPr></a:lvl2pPr>
<a:lvl3pPr><a:defRPr sz="1500"><a:solidFill><a:srgbClr val="{body_color}"/></a:solidFill><a:latin typeface="{body_font}"/></a:defRPr></a:lvl3pPr>
</p:bodyStyle>
<p:otherStyle><a:lvl1pPr><a:defRPr sz="1800"><a:latin typeface="{body_font}"/></a:defRPr></a:lvl1pPr></p:otherStyle>
</p:txStyles>
</p:sldMaster>"#,
        bg = theme.bg,
        title_color = theme.title_color,
        body_color = theme.body_color,
        title_font = escape_xml(&theme.title_font),
        body_font = escape_xml(&theme.body_font),
    )
}

fn slide_master_rels() -> String {
    String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
</Relationships>"#,
    )
}

fn slide_layout_xml() -> String {
    String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank" preserve="1">
<p:cSld name="Blank">
<p:spTree>
<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
</p:spTree>
</p:cSld>
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sldLayout>"#,
    )
}

fn slide_layout_rels() -> String {
    String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"#,
    )
}

fn presentation_xml(n: usize, theme: &Theme, has_notes: bool) -> String {
    let mut sld_id_lst = String::new();
    for i in 0..n {
        sld_id_lst.push_str(&format!(
            r#"<p:sldId id="{}" r:id="rId{}"/>"#,
            256 + i,
            2 + i
        ));
        sld_id_lst.push('\n');
    }
    let notes_master_lst = if has_notes {
        let rid = 2 + n;
        format!(
            "<p:notesMasterIdLst><p:notesMasterId r:id=\"rId{}\"/></p:notesMasterIdLst>\n",
            rid
        )
    } else {
        String::new()
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" saveSubsetFonts="1">
<p:sldMasterIdLst>
<p:sldMasterId id="2147483648" r:id="rId1"/>
</p:sldMasterIdLst>
{notes_master_lst}<p:sldIdLst>
{sld_id_lst}</p:sldIdLst>
<p:sldSz cx="{w}" cy="{h}" type="{kind}"/>
<p:notesSz cx="6858000" cy="9144000"/>
<p:defaultTextStyle>
<a:defPPr><a:defRPr lang="en-US"/></a:defPPr>
</p:defaultTextStyle>
</p:presentation>"#,
        w = theme.slide_w,
        h = theme.slide_h,
        kind = theme.aspect_kind,
    )
}

fn presentation_rels(n: usize, has_notes: bool) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
"#,
    );
    for i in 0..n {
        s.push_str(&format!(
            r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{}.xml"/>
"#,
            2 + i,
            i + 1
        ));
    }
    let mut next = 2 + n;
    if has_notes {
        s.push_str(&format!(
            r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster" Target="notesMasters/notesMaster1.xml"/>
"#,
            next
        ));
        next += 1;
    }
    s.push_str(&format!(
        r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>
</Relationships>"#,
        next
    ));
    s
}

fn notes_master_xml(theme: &Theme) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notesMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cSld>
<p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>
<p:spTree>
<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
</p:spTree>
</p:cSld>
<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>
<p:notesStyle>
<a:lvl1pPr><a:defRPr sz="1200"><a:solidFill><a:srgbClr val="{c}"/></a:solidFill><a:latin typeface="{f}"/></a:defRPr></a:lvl1pPr>
</p:notesStyle>
</p:notesMaster>"#,
        c = "1F2937",
        f = escape_xml(&theme.body_font),
    )
}

fn notes_master_rels() -> String {
    String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
</Relationships>"#,
    )
}

fn notes_slide_xml(notes: &str, theme: &Theme) -> String {
    let mut paras_xml = String::new();
    for para in notes.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            paras_xml.push_str("<a:p><a:endParaRPr lang=\"en-US\"/></a:p>");
            continue;
        }
        let mut line_iter = para.split('\n').peekable();
        while let Some(line) = line_iter.next() {
            let text = line.trim();
            paras_xml.push_str(&format!(
                r#"<a:p><a:pPr/><a:r><a:rPr lang="en-US" sz="1400"><a:solidFill><a:srgbClr val="1F2937"/></a:solidFill><a:latin typeface="{f}"/></a:rPr><a:t>{t}</a:t></a:r></a:p>"#,
                f = escape_xml(&theme.body_font),
                t = escape_xml(text),
            ));
            if line_iter.peek().is_none() {
                paras_xml.push_str("<a:p><a:endParaRPr lang=\"en-US\"/></a:p>");
            }
        }
    }
    if paras_xml.is_empty() {
        paras_xml.push_str("<a:p><a:endParaRPr lang=\"en-US\"/></a:p>");
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cSld>
<p:spTree>
<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
<p:sp>
<p:nvSpPr>
<p:cNvPr id="2" name="Slide Image Placeholder 1"/>
<p:cNvSpPr><a:spLocks noGrp="1" noRot="1" noChangeAspect="1"/></p:cNvSpPr>
<p:nvPr><p:ph type="sldImg"/></p:nvPr>
</p:nvSpPr>
<p:spPr/>
</p:sp>
<p:sp>
<p:nvSpPr>
<p:cNvPr id="3" name="Notes Placeholder 2"/>
<p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr>
<p:nvPr><p:ph type="body" idx="1"/></p:nvPr>
</p:nvSpPr>
<p:spPr/>
<p:txBody>
<a:bodyPr/>
<a:lstStyle/>
{paras_xml}
</p:txBody>
</p:sp>
</p:spTree>
</p:cSld>
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:notes>"#,
    )
}

fn notes_slide_rels(slide_num: usize) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="../slides/slide{n}.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster" Target="../notesMasters/notesMaster1.xml"/>
</Relationships>"#,
        n = slide_num,
    )
}

fn slide_xml(
    slide: &Slide,
    num: usize,
    total: usize,
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    imgs: &Imgs,
    rels: &mut SlideRels,
    transition: Option<&str>,
    transition_dur: f32,
) -> String {
    let _ = take_link_buffer();
    let mut id: u32 = 100;
    // Publish per-slide layout hints to the thread-locals that the block
    // renderers read. Cleared at the end of this function.
    SLIDE_ALIGN.with(|a| {
        *a.borrow_mut() = match slide.text_align() {
            "center" => "ctr",
            "right" => "r",
            _ => "l",
        }
    });
    SLIDE_COL_FRAC.with(|f| *f.borrow_mut() = slide.col_frac());
    SLIDE_VALIGN.with(|v| {
        *v.borrow_mut() = match slide.valign() {
            "center" => "center",
            "bottom" => "bottom",
            _ => "top",
        }
    });
    SLIDE_TEXT_SCALE.with(|s| *s.borrow_mut() = slide.text_scale());

    let (shapes, bg_override) = match &slide.kind {
        SlideKind::Title {
            subtitle,
            author,
            date,
        } => (
            render_title_slide(
                slide,
                subtitle.as_deref(),
                author.as_deref(),
                date.as_deref(),
                theme,
                layout,
                &mut id,
            ),
            None,
        ),
        SlideKind::Section => {
            render_section_slide(slide, num, total, deck_title, theme, layout, &mut id)
        }
        SlideKind::Content => (
            render_content_slide(
                slide, num, total, deck_title, theme, layout, &mut id, imgs, rels,
            ),
            slide.bg_color().map(|c| resolve_bg_color(c, theme)),
        ),
    };

    let link_urls = take_link_buffer();
    let shapes = resolve_link_placeholders(&shapes, &link_urls, rels);

    let bg_block = if let Some(color) = bg_override {
        format!(
            r#"<p:bg><p:bgPr><a:solidFill><a:srgbClr val="{}"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>"#,
            color
        )
    } else {
        String::new()
    };

    let bg_image_xml = if let Some(bg) = &slide.bg_image {
        if let Some(idx) = imgs.index(bg) {
            let rid = rels.add(
                REL_IMAGE,
                &format!("../media/image{}.{}", idx + 1, imgs.metas[idx].ext),
            );
            let pic_id = id;
            // Bump even though no further shape on this slide consumes it; it
            // keeps the id sequence symmetric with the other render paths.
            id = id.saturating_add(1);
            let _ = id;
            format!(
                r#"<p:pic>
<p:nvPicPr>
<p:cNvPr id="{pid}" name="BgImage{pid}" descr="background"/>
<p:cNvPicPr><a:picLocks noChangeAspect="0"/></p:cNvPicPr>
<p:nvPr/>
</p:nvPicPr>
<p:blipFill>
<a:blip r:embed="{rid}"/>
<a:stretch><a:fillRect/></a:stretch>
</p:blipFill>
<p:spPr>
<a:xfrm><a:off x="0" y="0"/><a:ext cx="{w}" cy="{h}"/></a:xfrm>
<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
</p:spPr>
</p:pic>"#,
                pid = pic_id,
                rid = rid,
                w = theme.slide_w,
                h = theme.slide_h,
            )
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let transition_xml = pptx_transition_xml(transition, transition_dur);

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cSld>
{bg}<p:spTree>
<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
{bg_img}{shapes}
</p:spTree>
</p:cSld>
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
{trans}</p:sld>"#,
        bg = bg_block,
        bg_img = bg_image_xml,
        shapes = shapes,
        trans = transition_xml,
    )
}

/// Apply RTL layout to a slide's XML in a single string sweep. PowerPoint
/// honours `rtl="1"` on `<a:pPr>` and flips bullet/text alignment from
/// `algn="l"` to `algn="r"`. Centered (`ctr`) paragraphs stay centered.
fn apply_rtl_pptx(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len() + xml.matches("<a:pPr").count() * 10);
    let bytes = xml.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 6 <= bytes.len() && &bytes[i..i + 6] == b"<a:pPr" {
            out.push_str("<a:pPr rtl=\"1\"");
            i += 6;
            continue;
        }
        if i + 8 <= bytes.len() && &bytes[i..i + 8] == b"algn=\"l\"" {
            out.push_str("algn=\"r\"");
            i += 8;
            continue;
        }
        let ch_end = if bytes[i] < 0x80 {
            i + 1
        } else if bytes[i] < 0xE0 {
            i + 2
        } else if bytes[i] < 0xF0 {
            i + 3
        } else {
            i + 4
        };
        let ch_end = ch_end.min(bytes.len());
        out.push_str(&xml[i..ch_end]);
        i = ch_end;
    }
    out
}

fn pptx_transition_xml(kind: Option<&str>, duration_s: f32) -> String {
    let Some(kind) = kind else {
        return String::new();
    };
    let kind = kind.to_ascii_lowercase();
    if kind.is_empty() || kind == "none" {
        return String::new();
    }
    let inner = match kind.as_str() {
        "fade" => "<p:fade/>".to_string(),
        "push" => r#"<p:push dir="l"/>"#.to_string(),
        "wipe" => r#"<p:wipe dir="l"/>"#.to_string(),
        "cover" => r#"<p:cover dir="l"/>"#.to_string(),
        "split" => r#"<p:split orient="horz" dir="out"/>"#.to_string(),
        _ => return String::new(),
    };
    let dur_ms = (duration_s.clamp(0.05, 5.0) * 1000.0) as u32;
    format!(
        r#"<p:transition spd="med" advClick="1" advTm="0" dur="{}">{}</p:transition>
"#,
        dur_ms, inner,
    )
}

fn render_title_slide(
    slide: &Slide,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    layout: &Layout,
    id: &mut u32,
) -> String {
    match layout.kind {
        LayoutKind::Clean => title_slide_clean(slide, subtitle, author, date, theme, id),
        LayoutKind::Studio => title_slide_studio(slide, subtitle, author, date, theme, id),
        LayoutKind::Frame => title_slide_frame(slide, subtitle, author, date, theme, id),
        LayoutKind::Bold => title_slide_bold(slide, subtitle, author, date, theme, id),
    }
}

fn title_slide_clean(
    slide: &Slide,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    id: &mut u32,
) -> String {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let mut s = String::new();

    s.push_str(&filled_rect(
        *id,
        "TopStripe",
        0,
        0,
        w,
        360000,
        &theme.accent,
    ));
    *id += 1;

    let accent_block_y = h / 2 - 1700000;
    s.push_str(&filled_rect(
        *id,
        "AccentBar",
        600000,
        accent_block_y,
        100000,
        600000,
        &theme.accent,
    ));
    *id += 1;

    let title_y = h / 2 - 1600000;
    let title_x = 800000;
    let title_w = w - 1200000;
    let title_body = render_paragraph(
        &[Run::plain(&slide.title)],
        theme,
        theme.hero_size,
        &theme.title_color,
        true,
        "l",
        false,
        &theme.title_font,
    );
    s.push_str(&text_box(
        *id,
        "Title",
        title_x,
        title_y,
        title_w,
        1400000,
        &title_body,
        "t",
    ));
    *id += 1;

    if let Some(sub) = subtitle {
        let sub_y = title_y + 1400000;
        let sub_size = if theme.portrait { 1800 } else { 2400 };
        let sub_body = render_paragraph(
            &[Run::plain(sub)],
            theme,
            sub_size,
            &theme.accent,
            false,
            "l",
            false,
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id, "Subtitle", title_x, sub_y, title_w, 700000, &sub_body, "t",
        ));
        *id += 1;
    }

    s.push_str(&title_footer(
        author,
        date,
        title_x,
        h - 700000,
        title_w,
        theme,
        id,
        "l",
    ));
    s
}

fn title_slide_studio(
    slide: &Slide,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    id: &mut u32,
) -> String {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let mut s = String::new();

    s.push_str(&filled_rect(*id, "Rail", 0, 0, 90000, h, &theme.accent));
    *id += 1;

    let kicker_y = 900000;
    let kicker_x = 900000;
    let kicker_w = w - kicker_x - 600000;
    let mut kicker_text = String::new();
    if let Some(a) = author {
        kicker_text.push_str(&letterspaced(a));
    }
    if author.is_some() && date.is_some() {
        kicker_text.push_str("   ·   ");
    }
    if let Some(d) = date {
        kicker_text.push_str(&letterspaced(d));
    }
    if !kicker_text.is_empty() {
        let body = render_paragraph(
            &[Run::plain(&kicker_text)],
            theme,
            1200,
            &theme.muted_color,
            true,
            "l",
            false,
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id, "Kicker", kicker_x, kicker_y, kicker_w, 400000, &body, "t",
        ));
        *id += 1;
    }

    let title_y = h / 2 - 1000000;
    let title_w = w - kicker_x - 600000;
    let body = render_paragraph(
        &[Run::plain(&slide.title)],
        theme,
        theme.hero_size,
        &theme.title_color,
        false,
        "l",
        true,
        &theme.title_font,
    );
    s.push_str(&text_box(
        *id, "Title", kicker_x, title_y, title_w, 1600000, &body, "t",
    ));
    *id += 1;

    if let Some(sub) = subtitle {
        let sub_y = title_y + 1600000;
        let sub_size = if theme.portrait { 1700 } else { 2200 };
        let sub_body = render_paragraph(
            &[Run::plain(sub)],
            theme,
            sub_size,
            &theme.body_color,
            false,
            "l",
            false,
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id, "Subtitle", kicker_x, sub_y, title_w, 700000, &sub_body, "t",
        ));
        *id += 1;

        let dash_y = sub_y + 800000;
        s.push_str(&filled_rect(
            *id,
            "Dash",
            kicker_x,
            dash_y,
            600000,
            30000,
            &theme.accent,
        ));
        *id += 1;
    }

    s
}

fn title_slide_frame(
    slide: &Slide,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    id: &mut u32,
) -> String {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let sidebar = 2_200_000_u32;
    let mut s = String::new();

    s.push_str(&filled_rect(
        *id,
        "Sidebar",
        0,
        0,
        sidebar,
        h,
        &theme.accent,
    ));
    *id += 1;

    let pad_x = 300000;
    let mut text = String::new();
    if let Some(a) = author {
        text.push_str(a);
    }
    if let Some(d) = date {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(d);
    }
    if !text.is_empty() {
        let body = render_lines_paragraph(
            &text,
            theme,
            1300,
            &theme.on_accent,
            false,
            "l",
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id,
            "SidebarMeta",
            pad_x,
            h - 1200000,
            sidebar - 2 * pad_x,
            900000,
            &body,
            "t",
        ));
        *id += 1;
    }

    let mark_y = pad_x;
    s.push_str(&filled_rect(
        *id,
        "Mark",
        pad_x,
        mark_y,
        380000,
        40000,
        &theme.on_accent,
    ));
    *id += 1;

    let title_x = sidebar + 600000;
    let title_y = h / 2 - 1100000;
    let title_w = w - title_x - 600000;
    let body = render_paragraph(
        &[Run::plain(&slide.title)],
        theme,
        theme.hero_size,
        &theme.title_color,
        true,
        "l",
        false,
        &theme.title_font,
    );
    s.push_str(&text_box(
        *id, "Title", title_x, title_y, title_w, 1500000, &body, "t",
    ));
    *id += 1;

    if let Some(sub) = subtitle {
        let sub_y = title_y + 1500000;
        let sub_size = if theme.portrait { 1700 } else { 2200 };
        let sub_body = render_paragraph(
            &[Run::plain(sub)],
            theme,
            sub_size,
            &theme.accent,
            false,
            "l",
            false,
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id, "Subtitle", title_x, sub_y, title_w, 700000, &sub_body, "t",
        ));
        *id += 1;
    }

    s
}

fn title_slide_bold(
    slide: &Slide,
    subtitle: Option<&str>,
    author: Option<&str>,
    date: Option<&str>,
    theme: &Theme,
    id: &mut u32,
) -> String {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let mut s = String::new();

    let block_h = h * 60 / 100;
    s.push_str(&filled_rect(*id, "Block", 0, 0, w, block_h, &theme.accent));
    *id += 1;

    let title_pad = 700000;
    let title_y = block_h - 1400000 - title_pad;
    let body = render_paragraph(
        &[Run::plain(&slide.title)],
        theme,
        theme.hero_size,
        &theme.on_accent,
        true,
        "l",
        false,
        &theme.title_font,
    );
    s.push_str(&text_box(
        *id,
        "Title",
        title_pad,
        title_y,
        w - 2 * title_pad,
        1400000,
        &body,
        "t",
    ));
    *id += 1;

    if let Some(sub) = subtitle {
        let sub_y = block_h + 500000;
        let sub_size = if theme.portrait { 1800 } else { 2400 };
        let sub_body = render_paragraph(
            &[Run::plain(sub)],
            theme,
            sub_size,
            &theme.title_color,
            false,
            "l",
            false,
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id,
            "Subtitle",
            title_pad,
            sub_y,
            w - 2 * title_pad,
            700000,
            &sub_body,
            "t",
        ));
        *id += 1;
    }

    s.push_str(&title_footer(
        author,
        date,
        title_pad,
        h - 700000,
        w - 2 * title_pad,
        theme,
        id,
        "l",
    ));
    s
}

fn title_footer(
    author: Option<&str>,
    date: Option<&str>,
    x: u32,
    y: u32,
    w: u32,
    theme: &Theme,
    id: &mut u32,
    algn: &str,
) -> String {
    let mut text = String::new();
    if let Some(a) = author {
        text.push_str(a);
    }
    if author.is_some() && date.is_some() {
        text.push_str("  ·  ");
    }
    if let Some(d) = date {
        text.push_str(d);
    }
    if text.is_empty() {
        return String::new();
    }
    let body = render_paragraph(
        &[Run::plain(&text)],
        theme,
        1400,
        &theme.muted_color,
        false,
        algn,
        false,
        &theme.body_font,
    );
    let s = text_box(*id, "TitleFooter", x, y, w, 400000, &body, "t");
    *id += 1;
    s
}

fn render_lines_paragraph(
    text: &str,
    theme: &Theme,
    size: u32,
    color: &str,
    bold: bool,
    algn: &str,
    font: &str,
) -> String {
    let mut s = String::new();
    for line in text.split('\n') {
        s.push_str(&render_paragraph(
            &[Run::plain(line)],
            theme,
            size,
            color,
            bold,
            algn,
            false,
            font,
        ));
    }
    s
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

fn render_section_slide(
    slide: &Slide,
    num: usize,
    total: usize,
    _deck_title: &str,
    theme: &Theme,
    layout: &Layout,
    id: &mut u32,
) -> (String, Option<String>) {
    match layout.kind {
        LayoutKind::Clean | LayoutKind::Frame => (
            section_centered(slide, theme, id),
            Some(theme.section_bg.clone()),
        ),
        LayoutKind::Studio => (section_studio(slide, num, total, theme, id), None),
        LayoutKind::Bold => (section_bold(slide, theme, id), None),
    }
}

fn section_centered(slide: &Slide, theme: &Theme, id: &mut u32) -> String {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let mut s = String::new();

    let bar_w = 1200000;
    let bar_x = (w - bar_w) / 2;
    let bar_y = h / 2 - 1000000;
    s.push_str(&filled_rect(
        *id,
        "Marker",
        bar_x,
        bar_y,
        bar_w,
        60000,
        &theme.section_text,
    ));
    *id += 1;

    let title_y = h / 2 - 700000;
    let title_w = w - 1600000;
    let title_x = (w - title_w) / 2;
    let body = render_paragraph(
        &[Run::plain(&slide.title)],
        theme,
        theme.hero_size,
        &theme.section_text,
        true,
        "ctr",
        false,
        &theme.title_font,
    );
    s.push_str(&text_box(
        *id,
        "SectionTitle",
        title_x,
        title_y,
        title_w,
        1500000,
        &body,
        "ctr",
    ));
    *id += 1;
    s
}

fn section_studio(slide: &Slide, num: usize, total: usize, theme: &Theme, id: &mut u32) -> String {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let mut s = String::new();

    s.push_str(&filled_rect(*id, "Rail", 0, 0, 90000, h, &theme.accent));
    *id += 1;

    let huge_size = if theme.portrait { 9000 } else { 13000 };
    let huge_text = format!("{:02}", num.min(99));
    let huge_x = w - 2_400_000;
    let huge_y = 380000;
    let huge = render_paragraph(
        &[Run::plain(&huge_text)],
        theme,
        huge_size,
        &theme.divider,
        true,
        "r",
        false,
        &theme.title_font,
    );
    s.push_str(&text_box(
        *id, "Numeral", huge_x, huge_y, 2_300_000, 2_400_000, &huge, "t",
    ));
    *id += 1;

    let kicker_x = 900000;
    let kicker_y = h - 2_600_000;
    let kicker_text = format!(
        "{}   ·   {}",
        letterspaced("section"),
        letterspaced(&format!("{} of {}", num, total))
    );
    let kicker_body = render_paragraph(
        &[Run::plain(&kicker_text)],
        theme,
        1100,
        &theme.muted_color,
        true,
        "l",
        false,
        &theme.body_font,
    );
    s.push_str(&text_box(
        *id,
        "Kicker",
        kicker_x,
        kicker_y,
        w - 1800000,
        350000,
        &kicker_body,
        "t",
    ));
    *id += 1;

    let title_x = kicker_x;
    let title_y = kicker_y + 400000;
    let title_w = w - title_x - 600000;
    let body = render_paragraph(
        &[Run::plain(&slide.title)],
        theme,
        theme.hero_size,
        &theme.title_color,
        false,
        "l",
        true,
        &theme.title_font,
    );
    s.push_str(&text_box(
        *id,
        "SectionTitle",
        title_x,
        title_y,
        title_w,
        1800000,
        &body,
        "t",
    ));
    *id += 1;

    s.push_str(&filled_rect(
        *id,
        "Dash",
        title_x,
        title_y + 1900000,
        600000,
        30000,
        &theme.accent,
    ));
    *id += 1;

    s
}

fn section_bold(slide: &Slide, theme: &Theme, id: &mut u32) -> String {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let mut s = String::new();

    let block_h = h * 70 / 100;
    s.push_str(&filled_rect(
        *id,
        "BoldBlock",
        0,
        0,
        w,
        block_h,
        &theme.accent,
    ));
    *id += 1;

    let pad = 700000;
    let body = render_paragraph(
        &[Run::plain(&slide.title)],
        theme,
        theme.hero_size,
        &theme.on_accent,
        true,
        "l",
        false,
        &theme.title_font,
    );
    s.push_str(&text_box(
        *id,
        "SectionTitle",
        pad,
        block_h - 1500000 - pad,
        w - 2 * pad,
        1500000,
        &body,
        "t",
    ));
    *id += 1;
    s
}

fn render_content_slide(
    slide: &Slide,
    num: usize,
    total: usize,
    deck_title: &str,
    theme: &Theme,
    layout: &Layout,
    id: &mut u32,
    imgs: &Imgs,
    rels: &mut SlideRels,
) -> String {
    let w = theme.slide_w;
    let h = theme.slide_h;
    let mut s = String::new();

    if let Some((src, alt, _)) = slide.full_page_image() {
        s.push_str(&render_full_page_image(src, alt, theme, id, imgs, rels));
        return s;
    }
    if let Some((lines, lang)) = slide.full_page_code() {
        s.push_str(&render_full_page_code(lines, lang, theme, id));
        return s;
    }

    let base_margin: u32 = 533400;
    let left_offset = layout.content_left_offset();
    let extra_left = if layout.shows_rail() { 200000 } else { 0 };
    let content_x = if left_offset > 0 {
        left_offset + 480000
    } else {
        base_margin + extra_left
    };
    let right_margin = base_margin;
    let content_w = w.saturating_sub(content_x + right_margin);

    if layout.shows_rail() {
        s.push_str(&filled_rect(
            *id,
            "Rail",
            0,
            0,
            layout.rail_width(),
            h,
            &theme.accent,
        ));
        *id += 1;
    }

    if layout.shows_sidebar() {
        let sb_w = layout.sidebar_width();
        s.push_str(&filled_rect(*id, "Sidebar", 0, 0, sb_w, h, &theme.accent));
        *id += 1;

        let pad = 300000;
        let body = render_paragraph(
            &[Run::plain(deck_title)],
            theme,
            1200,
            &theme.on_accent,
            true,
            "l",
            false,
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id,
            "SbDeck",
            pad,
            pad,
            sb_w - 2 * pad,
            600000,
            &body,
            "t",
        ));
        *id += 1;

        let num_body = render_paragraph(
            &[Run::plain(&format!("{:02} / {:02}", num, total))],
            theme,
            1100,
            &theme.on_accent,
            false,
            "l",
            false,
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id,
            "SbNum",
            pad,
            h - 600000,
            sb_w - 2 * pad,
            400000,
            &num_body,
            "t",
        ));
        *id += 1;
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

    if layout.title_block_bg() {
        let block_h = title_h + 240000;
        s.push_str(&filled_rect(
            *id,
            "TitleBlock",
            0,
            0,
            w,
            title_y + block_h,
            &theme.accent,
        ));
        *id += 1;
        let title_body = render_paragraph(
            &[Run::plain(&slide.title)],
            theme,
            theme.title_size,
            &theme.on_accent,
            true,
            "l",
            false,
            &theme.title_font,
        );
        s.push_str(&text_box(
            *id,
            "Title",
            base_margin,
            title_y,
            w - 2 * base_margin,
            title_h,
            &title_body,
            "t",
        ));
        *id += 1;
    } else {
        let title_body = render_paragraph(
            &[Run::plain(&slide.title)],
            theme,
            theme.title_size,
            &theme.title_color,
            true,
            "l",
            false,
            &theme.title_font,
        );
        s.push_str(&text_box(
            *id,
            "Title",
            content_x,
            title_y,
            content_w,
            title_h,
            &title_body,
            "t",
        ));
        *id += 1;
    }

    let underline_y = if layout.title_underline() {
        let y = title_y + title_h + 30000;
        s.push_str(&filled_rect(
            *id,
            "TitleDivider",
            content_x,
            y + 18000,
            content_w,
            14000,
            &theme.divider,
        ));
        *id += 1;
        s.push_str(&filled_rect(
            *id,
            "TitleProgress",
            content_x,
            y,
            progress_width(content_w, num, total),
            50000,
            &theme.accent,
        ));
        *id += 1;
        y
    } else {
        title_y + title_h
    };

    let content_y_start: u32 = underline_y
        + if layout.title_underline() {
            200000
        } else {
            280000
        };
    let footer_y: u32 = h - 400000;
    let content_max_y: u32 = footer_y - 100000;
    let content_h_total: u32 = content_max_y.saturating_sub(content_y_start);

    let (blocks_xml, _y) = render_blocks(
        &slide.blocks,
        theme,
        content_x,
        content_y_start,
        content_w,
        content_h_total,
        id,
        imgs,
        rels,
    );
    s.push_str(&blocks_xml);

    if !layout.shows_sidebar() {
        if let Some((key, iw, ih)) = imgs.logo() {
            let logo_h: u32 = 300000;
            let logo_w = if ih > 0 {
                ((logo_h as u64 * iw as u64) / ih as u64) as u32
            } else {
                logo_h
            };
            let pic_id = *id;
            *id += 1;
            let rid = rels.add(
                REL_IMAGE,
                &format!(
                    "../media/image{}.{}",
                    imgs.index(key).map(|i| i + 1).unwrap_or(1),
                    imgs.metas[imgs.index(key).unwrap_or(0)].ext,
                ),
            );
            s.push_str(&format!(
                r#"<p:pic>
<p:nvPicPr>
<p:cNvPr id="{pid}" name="Logo{pid}" descr="logo"/>
<p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr>
<p:nvPr/>
</p:nvPicPr>
<p:blipFill>
<a:blip r:embed="{rid}"/>
<a:stretch><a:fillRect/></a:stretch>
</p:blipFill>
<p:spPr>
<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{wcx}" cy="{hcy}"/></a:xfrm>
<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
</p:spPr>
</p:pic>"#,
                pid = pic_id,
                rid = rid,
                x = content_x,
                y = footer_y - 50000,
                wcx = logo_w,
                hcy = logo_h,
            ));
        } else {
            let foot_body = render_paragraph(
                &[Run::plain(deck_title)],
                theme,
                1000,
                &theme.muted_color,
                false,
                "l",
                false,
                &theme.body_font,
            );
            s.push_str(&text_box(
                *id,
                "DeckFooter",
                content_x,
                footer_y,
                content_w.saturating_sub(800000),
                300000,
                &foot_body,
                "t",
            ));
            *id += 1;
        }

        let num_body = render_paragraph(
            &[Run::plain(&format!("{} / {}", num, total))],
            theme,
            1000,
            &theme.muted_color,
            false,
            "r",
            false,
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id,
            "PageNum",
            w - right_margin - 800000,
            footer_y,
            800000,
            300000,
            &num_body,
            "t",
        ));
        *id += 1;
    }

    if layout.shows_corner_decoration() {
        s.push_str(&filled_rect(
            *id,
            "CornerDot",
            w - 280000,
            h - 280000,
            120000,
            120000,
            &theme.accent,
        ));
        *id += 1;
    }

    s
}

fn render_full_page_code(
    lines: &[String],
    lang: Option<&str>,
    theme: &Theme,
    id: &mut u32,
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
    let align = if math_markup { "ctr" } else { "l" };
    let italic = math_markup;
    let font = if math_markup {
        &theme.body_font
    } else {
        &theme.mono_font
    };
    let body = rendered_lines
        .iter()
        .map(|line| {
            render_paragraph(
                &[Run::plain(line)],
                theme,
                size,
                &theme.title_color,
                false,
                align,
                italic,
                font,
            )
        })
        .collect::<String>();
    let shape = text_box(*id, "FullPageText", margin_x, margin_y, w, h, &body, "t");
    *id += 1;
    shape
}

fn render_full_page_image(
    src: &str,
    alt: &str,
    theme: &Theme,
    id: &mut u32,
    imgs: &Imgs,
    rels: &mut SlideRels,
) -> String {
    let (display_w, display_h) = if let Some((iw, ih)) = imgs.dims(src) {
        fit_image(iw, ih, theme.slide_w, theme.slide_h)
    } else {
        (theme.slide_w, theme.slide_h)
    };
    let x = theme.slide_w.saturating_sub(display_w) / 2;
    let y = theme.slide_h.saturating_sub(display_h) / 2;

    let mut s = String::new();
    if let Some(idx) = imgs.index(src) {
        let rid = rels.add(
            REL_IMAGE,
            &format!("../media/image{}.{}", idx + 1, imgs.metas[idx].ext),
        );
        s.push_str(&format!(
            r#"<p:pic>
<p:nvPicPr>
<p:cNvPr id="{id}" name="Picture{id}" descr="{alt_safe}"/>
<p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr>
<p:nvPr/>
</p:nvPicPr>
<p:blipFill>
<a:blip r:embed="{rid}"/>
<a:stretch><a:fillRect/></a:stretch>
</p:blipFill>
<p:spPr>
<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{display_w}" cy="{display_h}"/></a:xfrm>
<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
</p:spPr>
</p:pic>"#,
            id = *id,
            alt_safe = escape_xml(alt),
        ));
    } else {
        s.push_str(&filled_rect(
            *id,
            "MissingImage",
            0,
            0,
            theme.slide_w,
            theme.slide_h,
            &theme.divider,
        ));
    }
    *id += 1;
    s
}

fn render_blocks(
    blocks: &[Block],
    theme: &Theme,
    x: u32,
    y_start: u32,
    w: u32,
    h_total: u32,
    id: &mut u32,
    imgs: &Imgs,
    rels: &mut SlideRels,
) -> (String, u32) {
    let mut shapes = String::new();
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

    // Vertical alignment: when the content underfills the band (scale == 1.0),
    // push it down by the slack so it sits centred / bottom-aligned. Only the
    // top-level content call (h_total == the full band) should do this, so we
    // gate on scale to avoid shifting nested column content.
    let mut y = y_start;
    if scale >= 1.0 {
        let slack = h_total.saturating_sub(estimated);
        y += match slide_valign() {
            "center" => slack / 2,
            "bottom" => slack,
            _ => 0,
        };
    }
    for block in blocks {
        let raw_h = block_height_emu(block, w, theme, imgs);
        let h = ((raw_h as f32) * scale) as u32;
        let h = h.max(200000);
        match block {
            Block::Paragraph(runs) => {
                let body = render_paragraph(
                    runs,
                    theme,
                    tscaled(theme.body_size),
                    &theme.body_color,
                    false,
                    slide_align(),
                    false,
                    &theme.body_font,
                );
                shapes.push_str(&text_box(*id, "Para", x, y, w, h, &body, "t"));
                *id += 1;
            }
            Block::Heading { level, runs } => {
                let base = match level {
                    3 => theme.title_size - 400,
                    4 => theme.title_size - 600,
                    _ => theme.title_size - 800,
                };
                let sz = tscaled(base);
                let body = render_paragraph(
                    runs,
                    theme,
                    sz,
                    &theme.title_color,
                    true,
                    slide_align(),
                    false,
                    &theme.title_font,
                );
                shapes.push_str(&text_box(*id, "Heading", x, y, w, h, &body, "t"));
                *id += 1;
            }
            Block::List(items) => {
                if items.len() > crate::theme::LONG_LIST_THRESHOLD && !theme.portrait {
                    let half = items.len().div_ceil(2);
                    let (l, r) = items.split_at(half);
                    let gap: u32 = 200000;
                    let col_w = (w.saturating_sub(gap)) / 2;
                    let body_l = render_list(l, theme);
                    shapes.push_str(&text_box(*id, "ListL", x, y, col_w, h, &body_l, "t"));
                    *id += 1;
                    let body_r = render_list(r, theme);
                    shapes.push_str(&text_box(
                        *id,
                        "ListR",
                        x + col_w + gap,
                        y,
                        col_w,
                        h,
                        &body_r,
                        "t",
                    ));
                    *id += 1;
                } else {
                    let body = render_list(items, theme);
                    shapes.push_str(&text_box(*id, "List", x, y, w, h, &body, "t"));
                    *id += 1;
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
                shapes.push_str(&render_code_block(
                    lines,
                    title.as_deref(),
                    lang.as_deref(),
                    *line_numbers,
                    *start_line,
                    theme,
                    x,
                    y,
                    w,
                    h,
                    id,
                ));
            }
            Block::Quote(paras) => {
                shapes.push_str(&filled_rect(*id, "QuoteBar", x, y, 60000, h, &theme.accent));
                *id += 1;
                let mut quote_body = String::new();
                for (i, runs) in paras.iter().enumerate() {
                    if i > 0 {
                        quote_body.push_str(&empty_paragraph());
                    }
                    quote_body.push_str(&render_runs_paragraph(
                        runs,
                        theme,
                        tscaled(theme.body_size - 100),
                        &theme.body_color,
                        false,
                        "l",
                        true,
                        &theme.body_font,
                        None,
                    ));
                }
                shapes.push_str(&text_box(
                    *id,
                    "Quote",
                    x + 180000,
                    y,
                    w.saturating_sub(180000),
                    h,
                    &quote_body,
                    "t",
                ));
                *id += 1;
            }
            Block::Callout { kind, body } => {
                let accent = callout_color(kind);
                let tint = light_tint(accent);
                let (icon, label) = callout_label(kind);
                let mut inner = render_paragraph(
                    &[Run::plain(format!("{icon} {label}"))],
                    theme,
                    tscaled(theme.body_size - 50),
                    accent,
                    true,
                    "l",
                    false,
                    &theme.body_font,
                );
                for runs in body {
                    inner.push_str(&render_paragraph(
                        runs,
                        theme,
                        tscaled(theme.body_size - 100),
                        &theme.body_color,
                        false,
                        "l",
                        false,
                        &theme.body_font,
                    ));
                }
                shapes.push_str(&filled_box(
                    *id,
                    "Callout",
                    x,
                    y,
                    w,
                    h,
                    &inner,
                    &tint,
                    Some(accent),
                    "t",
                ));
                *id += 1;
            }
            Block::Table { headers, rows, .. } => {
                shapes.push_str(&render_table(headers, rows, theme, x, y, w, h, id));
            }
            Block::Columns { left, right } => {
                let gap: u32 = 280000;
                let avail = w.saturating_sub(gap);
                let left_w = match slide_col_frac() {
                    Some(f) => (avail as f32 * f) as u32,
                    None => avail / 2,
                };
                let right_w = avail.saturating_sub(left_w);
                let (l_xml, _) = render_blocks(left, theme, x, y, left_w, h, id, imgs, rels);
                let (r_xml, _) = render_blocks(
                    right,
                    theme,
                    x + left_w + gap,
                    y,
                    right_w,
                    h,
                    id,
                    imgs,
                    rels,
                );
                shapes.push_str(&l_xml);
                shapes.push_str(&r_xml);
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
                shapes.push_str(&render_image_block(
                    src,
                    alt,
                    theme,
                    x,
                    y,
                    effective_w,
                    h,
                    id,
                    imgs,
                    rels,
                ));
            }
            Block::Footnotes(items) => {
                shapes.push_str(&render_footnotes_block(items, theme, x, y, w, h, id));
            }
            Block::Cards { cards, cols } => {
                let n = (*cols as usize).max(1);
                let gap: u32 = 180000;
                let col_w = w.saturating_sub(gap * (n as u32 - 1)) / n as u32;
                let card_border = theme.divider.clone();
                let nrows = cards.len().div_ceil(n) as u32;
                let row_h = h.saturating_sub((nrows.saturating_sub(1)) * gap) / nrows.max(1);
                for (r, row) in cards.chunks(n).enumerate() {
                    let ry = y + r as u32 * (row_h + gap);
                    for (i, card) in row.iter().enumerate() {
                        let cx = x + i as u32 * (col_w + gap);
                        let mut inner = render_paragraph(
                            &[Run::plain(card.title.clone())],
                            theme,
                            tscaled(theme.body_size),
                            &theme.title_color,
                            true,
                            "l",
                            false,
                            &theme.title_font,
                        );
                        inner.push_str(&render_paragraph(
                            &card.body,
                            theme,
                            tscaled(theme.body_size - 100),
                            &theme.body_color,
                            false,
                            "l",
                            false,
                            &theme.body_font,
                        ));
                        shapes.push_str(&filled_box(
                            *id,
                            "Card",
                            cx,
                            ry,
                            col_w,
                            row_h,
                            &inner,
                            &theme.accent_soft,
                            Some(&card_border),
                            "t",
                        ));
                        *id += 1;
                    }
                }
            }
        }
        y += h + 80000;
    }
    (shapes, y)
}

fn render_footnotes_block(
    items: &[ListItem],
    theme: &Theme,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    id: &mut u32,
) -> String {
    let size = (theme.body_size as f32 * 0.7) as u32;
    let mut body = String::new();
    for item in items {
        let p = render_paragraph(
            &item.runs,
            theme,
            size,
            &theme.muted_color,
            false,
            "l",
            false,
            &theme.body_font,
        );
        body.push_str(&p);
    }
    let shape = text_box(*id, "Footnotes", x, y, w, h, &body, "t");
    *id += 1;
    shape
}

fn chars_per_line(w: u32) -> u32 {
    (w / 120000).max(20)
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
        Block::Quote(paras) => {
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
        Block::Callout { body, .. } => {
            // Quote-style body height plus a label line and the box padding.
            body.iter()
                .map(|runs| {
                    let chars = total_chars(runs) as u32;
                    let lines = (chars / cpl.saturating_sub(4).max(12)).max(1) + 1;
                    lines * 280000
                })
                .sum::<u32>()
                + 280000 // label line
                + 320000 // top+bottom padding
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
            // 65% of slide height is a friendly default — leaves room for
            // the slide title plus surrounding content. Hardcoded EMU caps
            // here would over-shrink portrait/letter decks.
            let max_image_h = theme.slide_h * crate::theme::IMAGE_MAX_HEIGHT_FRACTION_NUM
                / crate::theme::IMAGE_MAX_HEIGHT_FRACTION_DEN;
            let placeholder_h = theme.slide_h * 30 / 100;
            let (display_w, display_h) = if let Some((iw, ih)) = imgs.dims(src) {
                fit_image_for_block(src, alt, iw, ih, w, max_image_h, theme.slide_h)
            } else {
                (w, placeholder_h)
            };
            let _ = display_w;
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
        Block::Cards { cards, cols } => {
            // Grid: rows × tallest card. Each card is title + body in a box.
            let n = (*cols as usize).max(1);
            let gap: u32 = 180000;
            let col_w = w.saturating_sub(gap * (n as u32 - 1)) / n as u32;
            let ccpl = chars_per_line(col_w).max(8);
            let tallest = cards
                .iter()
                .map(|c| {
                    let title_lines = (c.title.chars().count() as u32 / ccpl).max(1);
                    let body_lines = (total_chars(&c.body) as u32 / ccpl).max(1) + 1;
                    title_lines * 300000 + body_lines * 260000 + 280000
                })
                .max()
                .unwrap_or(0);
            let rows = cards.len().div_ceil(n) as u32;
            rows * tallest + (rows.saturating_sub(1)) * gap
        }
    }
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

fn render_image_block(
    src: &str,
    alt: &str,
    theme: &Theme,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    id: &mut u32,
    imgs: &Imgs,
    rels: &mut SlideRels,
) -> String {
    let caption_alt = crate::math::visible_image_alt(src, alt);
    let caption_h = if caption_alt.is_empty() { 0 } else { 260000 };
    let image_h = h.saturating_sub(caption_h + 80000);

    let (display_w, display_h) = if let Some((iw, ih)) = imgs.dims(src) {
        fit_image_for_block(src, alt, iw, ih, w, image_h, theme.slide_h)
    } else {
        (w, image_h)
    };

    let mut s = String::new();
    if let Some(idx) = imgs.index(src) {
        let img_x = image_x_for_block(src, alt, x, w, display_w);
        let rid = rels.add(
            REL_IMAGE,
            &format!("../media/image{}.{}", idx + 1, imgs.metas[idx].ext),
        );
        s.push_str(&format!(
            r#"<p:pic>
<p:nvPicPr>
<p:cNvPr id="{id}" name="Picture{id}" descr="{alt_safe}"/>
<p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr>
<p:nvPr/>
</p:nvPicPr>
<p:blipFill>
<a:blip r:embed="{rid}"/>
<a:stretch><a:fillRect/></a:stretch>
</p:blipFill>
<p:spPr>
<a:xfrm><a:off x="{img_x}" y="{y}"/><a:ext cx="{display_w}" cy="{display_h}"/></a:xfrm>
<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
</p:spPr>
</p:pic>"#,
            id = *id,
            alt_safe = escape_xml(caption_alt),
        ));
        *id += 1;
    } else {
        s.push_str(&filled_rect(
            *id,
            "MissingImage",
            x,
            y,
            w,
            image_h,
            &theme.divider,
        ));
        *id += 1;
    }

    if !caption_alt.is_empty() {
        let caption_y = y + display_h + 60000;
        let body = render_paragraph(
            &[Run::plain(caption_alt)],
            theme,
            1300,
            &theme.muted_color,
            false,
            "ctr",
            true,
            &theme.body_font,
        );
        s.push_str(&text_box(
            *id, "Caption", x, caption_y, w, caption_h, &body, "t",
        ));
        *id += 1;
    }
    s
}

fn total_chars(runs: &[Run]) -> usize {
    runs.iter().map(|r| r.text.chars().count()).sum()
}

fn render_paragraph(
    runs: &[Run],
    theme: &Theme,
    size_centipt: u32,
    color: &str,
    bold: bool,
    algn: &str,
    italic: bool,
    font: &str,
) -> String {
    render_runs_paragraph(
        runs,
        theme,
        size_centipt,
        color,
        bold,
        algn,
        italic,
        font,
        None,
    )
}

fn render_runs_paragraph(
    runs: &[Run],
    theme: &Theme,
    size_centipt: u32,
    color: &str,
    bold: bool,
    algn: &str,
    italic: bool,
    font: &str,
    p_pr_extra: Option<&str>,
) -> String {
    let extras = p_pr_extra.unwrap_or("");
    let ppr = format!(
        r#"<a:pPr algn="{}"{}>{}<a:buNone/></a:pPr>"#,
        algn, "", extras
    );
    let mut r_xml = String::new();
    if runs.is_empty() {
        r_xml.push_str(&run_xml(
            &Run::plain(""),
            size_centipt,
            color,
            bold,
            italic,
            font,
            theme,
        ));
    } else {
        for r in runs {
            r_xml.push_str(&run_xml(r, size_centipt, color, bold, italic, font, theme));
        }
    }
    format!("<a:p>{}{}</a:p>", ppr, r_xml)
}

fn empty_paragraph() -> String {
    String::from(r#"<a:p><a:pPr/><a:endParaRPr lang="en-US"/></a:p>"#)
}

fn run_xml(
    run: &Run,
    size_centipt: u32,
    base_color: &str,
    base_bold: bool,
    base_italic: bool,
    font: &str,
    theme: &Theme,
) -> String {
    let bold = base_bold || run.bold;
    let italic = base_italic || run.italic;
    let strike = run.strike;
    let is_link = run.link.is_some();
    let color = if is_link {
        &theme.link
    } else if run.code {
        &theme.code_accent
    } else {
        base_color
    };

    let mut attrs = format!(r#"lang="en-US" sz="{}" dirty="0""#, size_centipt);
    if bold {
        attrs.push_str(r#" b="1""#);
    }
    if italic {
        attrs.push_str(r#" i="1""#);
    }
    if strike {
        attrs.push_str(r#" strike="sngStrike""#);
    }
    if is_link {
        attrs.push_str(r#" u="sng""#);
    }

    let typeface = if run.code {
        theme.mono_font.as_str()
    } else {
        font
    };

    let mut rpr = format!("<a:rPr {}>", attrs);
    rpr.push_str(&format!(
        r#"<a:solidFill><a:srgbClr val="{}"/></a:solidFill>"#,
        color
    ));
    rpr.push_str(&format!(
        r#"<a:latin typeface="{}"/><a:ea typeface="{}"/><a:cs typeface="{}"/>"#,
        escape_xml(typeface),
        escape_xml(typeface),
        escape_xml(typeface),
    ));
    if let Some(url) = &run.link {
        let idx = LINK_BUFFER.with(|b| {
            let mut v = b.borrow_mut();
            v.push(url.clone());
            v.len() - 1
        });
        rpr.push_str(&format!(r#"<a:hlinkClick r:id="MD2LINK{}"/>"#, idx));
    }
    rpr.push_str("</a:rPr>");

    let text = if run.text.is_empty() {
        "<a:t></a:t>".to_string()
    } else {
        format!("<a:t>{}</a:t>", escape_xml(&run.text))
    };
    format!("<a:r>{}{}</a:r>", rpr, text)
}

fn render_list(items: &[ListItem], theme: &Theme) -> String {
    let mut s = String::new();
    for item in items {
        let lvl = item.level.min(8) as u32;
        let mar_l = 228600 + lvl * 285750;
        let indent = -228600_i32;
        let bullet = if item.is_task() {
            // Task items carry their own ☐/☑ marker in the runs, so suppress
            // the native bullet to avoid showing both.
            r#"<a:buNone/>"#.to_string()
        } else if item.ordered {
            r#"<a:buFont typeface="+mj-lt"/><a:buAutoNum type="arabicPeriod"/>"#.to_string()
        } else {
            let ch = match lvl {
                0 => "●",
                1 => "○",
                2 => "▪",
                _ => "·",
            };
            format!(
                r#"<a:buClr><a:srgbClr val="{}"/></a:buClr><a:buFont typeface="Arial"/><a:buChar char="{}"/>"#,
                theme.accent, ch
            )
        };

        let ppr = format!(
            r#"<a:pPr marL="{}" indent="{}" lvl="{}" algn="l">{}</a:pPr>"#,
            mar_l, indent, lvl, bullet
        );

        let mut runs_xml = String::new();
        for r in &item.runs {
            runs_xml.push_str(&run_xml(
                r,
                theme.body_size,
                &theme.body_color,
                false,
                false,
                &theme.body_font,
                theme,
            ));
        }
        if item.runs.is_empty() {
            runs_xml.push_str(&run_xml(
                &Run::plain(""),
                theme.body_size,
                &theme.body_color,
                false,
                false,
                &theme.body_font,
                theme,
            ));
        }
        s.push_str(&format!("<a:p>{}{}</a:p>", ppr, runs_xml));
    }
    s
}

fn render_code_block(
    lines: &[String],
    title: Option<&str>,
    lang: Option<&str>,
    line_numbers: bool,
    start_line: usize,
    theme: &Theme,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    id: &mut u32,
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
        s.push_str(&format!(
            r#"<p:sp>
<p:nvSpPr><p:cNvPr id="{id}" name="CodeTitle{id}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
<p:spPr>
<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{th}"/></a:xfrm>
<a:custGeom>
<a:avLst/>
<a:gdLst/>
<a:ahLst/>
<a:cxnLst/>
<a:rect l="0" t="0" r="0" b="0"/>
<a:pathLst>
<a:path w="100000" h="100000">
<a:moveTo><a:pt x="4500" y="0"/></a:moveTo>
<a:lnTo><a:pt x="95500" y="0"/></a:lnTo>
<a:cubicBezTo><a:pt x="97983" y="0"/><a:pt x="100000" y="2017"/><a:pt x="100000" y="4500"/></a:cubicBezTo>
<a:lnTo><a:pt x="100000" y="100000"/></a:lnTo>
<a:lnTo><a:pt x="0" y="100000"/></a:lnTo>
<a:lnTo><a:pt x="0" y="4500"/></a:lnTo>
<a:cubicBezTo><a:pt x="0" y="2017"/><a:pt x="2017" y="0"/><a:pt x="4500" y="0"/></a:cubicBezTo>
<a:close/>
</a:path>
</a:pathLst>
</a:custGeom>
<a:solidFill><a:srgbClr val="{title_bg}"/></a:solidFill>
<a:ln w="6350"><a:solidFill><a:srgbClr val="{divider}"/></a:solidFill></a:ln>
</p:spPr>
<p:txBody>
<a:bodyPr wrap="square" lIns="180000" tIns="50000" rIns="180000" bIns="50000" anchor="ctr"/>
<a:lstStyle/>
<a:p>
<a:pPr algn="l"><a:buNone/></a:pPr>"#,
            id = *id,
            x = x,
            y = y,
            w = w,
            th = title_h,
            title_bg = theme.divider,
            divider = theme.divider,
        ));
        *id += 1;
        if let Some(t) = title {
            s.push_str(&format!(
                r#"<a:r><a:rPr lang="en-US" sz="1200" b="1" dirty="0"><a:solidFill><a:srgbClr val="{c}"/></a:solidFill><a:latin typeface="{f}"/><a:ea typeface="{f}"/><a:cs typeface="{f}"/></a:rPr><a:t>{t}</a:t></a:r>"#,
                c = theme.title_color,
                f = escape_xml(&theme.mono_font),
                t = escape_xml(t),
            ));
            if let Some(l) = lang {
                s.push_str(&format!(
                    r#"<a:r><a:rPr lang="en-US" sz="1100" dirty="0"><a:solidFill><a:srgbClr val="{c}"/></a:solidFill><a:latin typeface="{f}"/></a:rPr><a:t>  ·  {l}</a:t></a:r>"#,
                    c = theme.muted_color,
                    f = escape_xml(&theme.body_font),
                    l = escape_xml(l),
                ));
            }
        } else if let Some(l) = lang {
            s.push_str(&format!(
                r#"<a:r><a:rPr lang="en-US" sz="1100" b="1" dirty="0"><a:solidFill><a:srgbClr val="{c}"/></a:solidFill><a:latin typeface="{f}"/></a:rPr><a:t>{l}</a:t></a:r>"#,
                c = theme.muted_color,
                f = escape_xml(&theme.body_font),
                l = escape_xml(l),
            ));
        }
        s.push_str("</a:p></p:txBody></p:sp>");
    }

    let geom = if title_h > 0 {
        r#"<a:custGeom>
<a:avLst/>
<a:gdLst/>
<a:ahLst/>
<a:cxnLst/>
<a:rect l="0" t="0" r="0" b="0"/>
<a:pathLst>
<a:path w="100000" h="100000">
<a:moveTo><a:pt x="0" y="0"/></a:moveTo>
<a:lnTo><a:pt x="100000" y="0"/></a:lnTo>
<a:lnTo><a:pt x="100000" y="95500"/></a:lnTo>
<a:cubicBezTo><a:pt x="100000" y="97983"/><a:pt x="97983" y="100000"/><a:pt x="95500" y="100000"/></a:cubicBezTo>
<a:lnTo><a:pt x="4500" y="100000"/></a:lnTo>
<a:cubicBezTo><a:pt x="2017" y="100000"/><a:pt x="0" y="97983"/><a:pt x="0" y="95500"/></a:cubicBezTo>
<a:close/>
</a:path>
</a:pathLst>
</a:custGeom>"#
            .to_string()
    } else {
        r#"<a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 4500"/></a:avLst></a:prstGeom>"#.to_string()
    };

    s.push_str(&format!(
        r#"<p:sp>
<p:nvSpPr><p:cNvPr id="{id}" name="CodeBox{id}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
<p:spPr>
<a:xfrm><a:off x="{x}" y="{cy}"/><a:ext cx="{w}" cy="{ch}"/></a:xfrm>
{geom}
<a:solidFill><a:srgbClr val="{bg}"/></a:solidFill>
<a:ln w="6350"><a:solidFill><a:srgbClr val="{divider}"/></a:solidFill></a:ln>
</p:spPr>
<p:txBody>
<a:bodyPr wrap="square" lIns="180000" tIns="120000" rIns="180000" bIns="120000" anchor="t"><a:noAutofit/></a:bodyPr>
<a:lstStyle/>"#,
        id = *id,
        x = x,
        cy = code_y,
        w = w,
        ch = code_h,
        bg = theme.code_bg,
        divider = theme.divider,
    ));
    *id += 1;

    let last_line = start_line.saturating_add(lines.len().saturating_sub(1));
    let gutter = if line_numbers {
        Some(last_line.to_string().len())
    } else {
        None
    };

    let highlighted: Vec<Vec<Token>> = syntax::tokenize(lines, lang);

    for (idx, line_tokens) in highlighted.iter().enumerate() {
        let mut runs_xml = String::new();
        if let Some(width) = gutter {
            let n = format!("{:>width$}", start_line + idx, width = width);
            runs_xml.push_str(&code_run(
                &format!("{}  ", n),
                &theme.muted_color,
                false,
                false,
                theme.code_size,
                &theme.mono_font,
            ));
        }
        if line_tokens.is_empty() || line_tokens.iter().all(|t| t.text.is_empty()) {
            runs_xml.push_str(&code_run(
                " ",
                &theme.code_text,
                false,
                false,
                theme.code_size,
                &theme.mono_font,
            ));
        } else {
            for token in line_tokens {
                if token.text.is_empty() {
                    continue;
                }
                let style = theme.syntax_style(token.kind);
                runs_xml.push_str(&code_run(
                    &token.text,
                    &style.color,
                    style.italic,
                    style.bold,
                    theme.code_size,
                    &theme.mono_font,
                ));
            }
        }
        s.push_str(&format!(
            r#"<a:p><a:pPr algn="l"><a:lnSpc><a:spcPct val="115000"/></a:lnSpc><a:spcBef><a:spcPts val="0"/></a:spcBef><a:spcAft><a:spcPts val="0"/></a:spcAft><a:buNone/></a:pPr>{runs}</a:p>"#,
            runs = runs_xml,
        ));
    }

    s.push_str("</p:txBody></p:sp>");
    s
}

fn code_run(text: &str, color: &str, italic: bool, bold: bool, size: u32, font: &str) -> String {
    let mut attrs = format!(r#"lang="en-US" sz="{}" dirty="0""#, size);
    if bold {
        attrs.push_str(r#" b="1""#);
    }
    if italic {
        attrs.push_str(r#" i="1""#);
    }
    format!(
        r#"<a:r><a:rPr {attrs}><a:solidFill><a:srgbClr val="{c}"/></a:solidFill><a:latin typeface="{f}"/><a:ea typeface="{f}"/><a:cs typeface="{f}"/></a:rPr><a:t>{t}</a:t></a:r>"#,
        attrs = attrs,
        c = color,
        f = escape_xml(font),
        t = escape_xml(text),
    )
}

fn render_table(
    headers: &[Vec<Run>],
    rows: &[Vec<Vec<Run>>],
    theme: &Theme,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    id: &mut u32,
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
    let frame_h = row_heights.iter().sum::<u32>().max(200000);

    let mut tbl = String::new();
    tbl.push_str(r#"<a:tbl><a:tblPr firstRow="1" bandRow="1"/><a:tblGrid>"#);
    for cw in &col_widths {
        tbl.push_str(&format!(r#"<a:gridCol w="{}"/>"#, cw));
    }
    tbl.push_str("</a:tblGrid>");

    tbl.push_str(&format!(r#"<a:tr h="{}">"#, row_heights[0]));
    for c in 0..cols {
        let runs = headers.get(c).map(|v| v.as_slice()).unwrap_or(&[]);
        let max_chars = table_chars_per_line(col_widths[c], table_header_size());
        tbl.push_str(&table_cell(runs, theme, true, max_chars));
    }
    tbl.push_str("</a:tr>");

    for (i, row) in rows.iter().enumerate() {
        tbl.push_str(&format!(r#"<a:tr h="{}">"#, row_heights[i + 1]));
        for c in 0..cols {
            let runs = row.get(c).map(|v| v.as_slice()).unwrap_or(&[]);
            let max_chars = table_chars_per_line(col_widths[c], table_body_size());
            tbl.push_str(&table_cell_body(runs, theme, false, i % 2 == 1, max_chars));
        }
        tbl.push_str("</a:tr>");
    }
    tbl.push_str("</a:tbl>");

    let frame_id = *id;
    *id += 1;
    format!(
        r#"<p:graphicFrame>
<p:nvGraphicFramePr>
<p:cNvPr id="{frame_id}" name="Table{frame_id}"/>
<p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr>
<p:nvPr/>
</p:nvGraphicFramePr>
<p:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{frame_h}"/></p:xfrm>
<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">{tbl}</a:graphicData></a:graphic>
</p:graphicFrame>"#
    )
}

fn table_header_size() -> u32 {
    1600
}

fn table_body_size() -> u32 {
    1400
}

fn table_cell(runs: &[Run], theme: &Theme, header: bool, max_chars: usize) -> String {
    table_cell_body(runs, theme, header, false, max_chars)
}

fn table_cell_body(
    runs: &[Run],
    theme: &Theme,
    header: bool,
    banded: bool,
    max_chars: usize,
) -> String {
    let table_band_bg = theme.table_band_bg();
    let fill = if header {
        theme.accent.clone()
    } else if banded {
        table_band_bg
    } else {
        theme.bg.clone()
    };
    let text_color = if header {
        theme.on_accent.clone()
    } else {
        theme.body_color.clone()
    };
    let size = if header {
        table_header_size()
    } else {
        table_body_size()
    };
    let bold = header;

    let mut runs_xml = String::new();
    for line in wrap_runs_for_table(runs, max_chars) {
        runs_xml.push_str(&render_runs_paragraph(
            &line,
            theme,
            size,
            &text_color,
            bold,
            "l",
            false,
            &theme.body_font,
            None,
        ));
    }

    format!(
        r#"<a:tc>
<a:txBody>
<a:bodyPr wrap="square" lIns="90000" tIns="46000" rIns="90000" bIns="46000" anchor="ctr"/>
<a:lstStyle/>
{runs_xml}
</a:txBody>
<a:tcPr lIns="0" tIns="0" rIns="0" bIns="0" anchor="ctr">
<a:lnL w="6350" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:srgbClr val="{div}"/></a:solidFill><a:prstDash val="solid"/></a:lnL>
<a:lnR w="6350" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:srgbClr val="{div}"/></a:solidFill><a:prstDash val="solid"/></a:lnR>
<a:lnT w="6350" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:srgbClr val="{div}"/></a:solidFill><a:prstDash val="solid"/></a:lnT>
<a:lnB w="6350" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:srgbClr val="{div}"/></a:solidFill><a:prstDash val="solid"/></a:lnB>
<a:solidFill><a:srgbClr val="{fill}"/></a:solidFill>
</a:tcPr>
</a:tc>"#,
        div = theme.divider,
        fill = fill,
    )
}

fn table_height_emu(headers: &[Vec<Run>], rows: &[Vec<Vec<Run>>], w: u32, theme: &Theme) -> u32 {
    let col_widths = table_column_widths(headers, rows, w);
    table_natural_row_heights(headers, rows, &col_widths, theme)
        .iter()
        .sum::<u32>()
        + 80000
}

fn table_natural_row_heights(
    headers: &[Vec<Run>],
    rows: &[Vec<Vec<Run>>],
    col_widths: &[u32],
    _theme: &Theme,
) -> Vec<u32> {
    let mut heights = Vec::with_capacity(rows.len() + 1);
    heights.push(table_row_height(
        headers,
        col_widths,
        table_header_size(),
        380000,
    ));
    for row in rows {
        heights.push(table_row_height(row, col_widths, table_body_size(), 340000));
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

fn text_box(
    id: u32,
    name: &str,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    body: &str,
    anchor: &str,
) -> String {
    format!(
        r#"<p:sp>
<p:nvSpPr>
<p:cNvPr id="{id}" name="{name}{id}"/>
<p:cNvSpPr txBox="1"/>
<p:nvPr/>
</p:nvSpPr>
<p:spPr>
<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm>
<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
<a:noFill/>
</p:spPr>
<p:txBody>
<a:bodyPr wrap="square" lIns="0" tIns="0" rIns="0" bIns="0" anchor="{anchor}"><a:noAutofit/></a:bodyPr>
<a:lstStyle/>
{body}
</p:txBody>
</p:sp>"#
    )
}

fn filled_rect(id: u32, name: &str, x: u32, y: u32, w: u32, h: u32, color: &str) -> String {
    format!(
        r#"<p:sp>
<p:nvSpPr>
<p:cNvPr id="{id}" name="{name}{id}"/>
<p:cNvSpPr/>
<p:nvPr/>
</p:nvSpPr>
<p:spPr>
<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm>
<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
<a:solidFill><a:srgbClr val="{color}"/></a:solidFill>
<a:ln><a:noFill/></a:ln>
</p:spPr>
<p:txBody>
<a:bodyPr wrap="square" anchor="ctr"/>
<a:lstStyle/>
<a:p><a:endParaRPr lang="en-US"/></a:p>
</p:txBody>
</p:sp>"#
    )
}

fn progress_width(width: u32, num: usize, total: usize) -> u32 {
    if width == 0 || total == 0 {
        return 0;
    }
    ((width as u64 * num.max(1) as u64) / total as u64)
        .min(width as u64)
        .max(1) as u32
}

/// Resolve a per-slide `bg:` token (a `#hex` or palette keyword) into a bare
/// 6-digit hex string for a PPTX `<a:srgbClr>`. Mirrors the PDF/SVG resolver.
/// Accent colour (bare hex) for a callout kind. Mirrors PDF/SVG/HTML.
fn callout_color(kind: &str) -> &'static str {
    match kind {
        "tip" => "22C55E",
        "important" => "A855F7",
        "warning" => "F59E0B",
        "caution" => "EF4444",
        _ => "3B82F6",
    }
}

/// Icon + display label for a callout kind.
fn callout_label(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "tip" => ("\u{1F4A1}", "Tip"),
        "important" => ("\u{2757}", "Important"),
        "warning" => ("\u{26A0}", "Warning"),
        "caution" => ("\u{1F6D1}", "Caution"),
        _ => ("\u{2139}", "Note"),
    }
}

/// Light background tint for a callout: accent mixed ~14% over white.
fn light_tint(hex: &str) -> String {
    let parse = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f32;
    if hex.len() != 6 {
        return "EEF2FF".to_string();
    }
    let mix = |c: f32| (255.0 - (255.0 - c) * 0.14) as u8;
    format!(
        "{:02X}{:02X}{:02X}",
        mix(parse(0)),
        mix(parse(2)),
        mix(parse(4))
    )
}

/// A filled, rounded shape containing pre-rendered paragraphs. `border` (bare
/// hex) draws a 1pt outline; `fill` is the bare-hex background.
#[allow(clippy::too_many_arguments)]
fn filled_box(
    id: u32,
    name: &str,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    body: &str,
    fill: &str,
    border: Option<&str>,
    anchor: &str,
) -> String {
    let ln = match border {
        Some(c) => {
            format!(r#"<a:ln w="12700"><a:solidFill><a:srgbClr val="{c}"/></a:solidFill></a:ln>"#)
        }
        None => String::new(),
    };
    format!(
        r#"<p:sp>
<p:nvSpPr><p:cNvPr id="{id}" name="{name}{id}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
<p:spPr>
<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm>
<a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 6000"/></a:avLst></a:prstGeom>
<a:solidFill><a:srgbClr val="{fill}"/></a:solidFill>
{ln}
</p:spPr>
<p:txBody>
<a:bodyPr wrap="square" lIns="120000" tIns="80000" rIns="120000" bIns="80000" anchor="{anchor}"><a:normAutofit/></a:bodyPr>
<a:lstStyle/>
{body}
</p:txBody>
</p:sp>"#
    )
}

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
