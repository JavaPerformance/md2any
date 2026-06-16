//! Native HTML deck renderer.
//!
//! This is separate from the `--serve` preview wrapper: the server currently
//! embeds the generated PDF for hot reload, while this module emits a
//! standalone browser deck from the same parsed/paginated IR as PPTX, ODP, and
//! PDF.

use crate::image;
use crate::ir::{Block, ColumnAlign, ListItem, Run, Slide, SlideKind};
use crate::layout::Layout;
use crate::syntax::{self, TokenKind};
use crate::theme::Theme;
use anyhow::Result;
use std::fmt::Write as _;
use std::path::Path;

pub fn write(
    slides: &[Slide],
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    direction: Option<&str>,
) -> Result<Vec<u8>> {
    write_opts(
        slides, theme, layout, deck_title, author, base_dir, logo, direction, false,
    )
}

/// As [`write`], but `editor: true` emits the deck in a continuous-scroll
/// layout (all slides stacked and visible, presenter navigation disabled) for
/// the `--serve --edit` live-DOM preview. The `--serve --edit` client morphs
/// each rebuild of this document into the iframe's live DOM, so only the
/// changed slide's nodes update; every `.slide` carries `data-line` (its source
/// line) so the caret can be mapped to the slide it sits in.
#[allow(clippy::too_many_arguments)]
pub fn write_opts(
    slides: &[Slide],
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    author: &str,
    base_dir: &Path,
    logo: Option<&Path>,
    direction: Option<&str>,
    editor: bool,
) -> Result<Vec<u8>> {
    let rtl = matches!(direction, Some("rtl"));
    let logo_uri = logo.map(|path| image_data_uri(base_dir, path.to_string_lossy().as_ref()));

    let mut out = String::new();
    out.push_str("<!doctype html>\n");
    out.push_str("<html lang=\"en\"");
    if rtl {
        out.push_str(" dir=\"rtl\"");
    }
    out.push_str(">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    write!(
        out,
        "<title>{}</title>\n",
        escape_html(if deck_title.is_empty() {
            "md2any deck"
        } else {
            deck_title
        })
    )?;
    out.push_str("<meta name=\"generator\" content=\"md2any-html-v1\">\n");
    if !author.is_empty() {
        write!(
            out,
            "<meta name=\"author\" content=\"{}\">\n",
            escape_attr(author)
        )?;
    }
    out.push_str("<style>\n");
    out.push_str(&css(theme, layout, rtl));
    if editor {
        out.push_str(EDITOR_CSS);
    }
    out.push_str("</style>\n</head>\n");
    write!(
        out,
        "<body class=\"layout-{} theme-{}{}{}\" data-slide-count=\"{}\">\n",
        layout.name(),
        escape_attr(theme.name),
        if rtl { " dir-rtl" } else { "" },
        if editor { " edit" } else { "" },
        slides.len()
    )?;
    out.push_str("<main class=\"deck\" aria-live=\"polite\">\n");
    for (idx, slide) in slides.iter().enumerate() {
        render_slide(
            &mut out,
            slide,
            idx,
            slides.len(),
            theme,
            layout,
            deck_title,
            base_dir,
            logo_uri.as_deref(),
        )?;
    }
    out.push_str("</main>\n");
    // The editor preview stacks every slide and is driven by the host shell
    // (caret-follow + morph), so the presenter controls/nav script would only
    // get in the way.
    if !editor {
        render_controls(&mut out, slides.len())?;
        out.push_str("<script>\n");
        out.push_str(SCRIPT);
        out.push_str("</script>\n");
    }
    out.push_str("</body>\n</html>\n");
    Ok(out.into_bytes())
}

fn render_slide(
    out: &mut String,
    slide: &Slide,
    idx: usize,
    total: usize,
    theme: &Theme,
    layout: &Layout,
    deck_title: &str,
    base_dir: &Path,
    logo_uri: Option<&str>,
) -> Result<()> {
    let kind = match slide.kind {
        SlideKind::Title { .. } => "title",
        SlideKind::Section => "section",
        SlideKind::Content => "content",
    };
    let full_page_image = slide.full_page_image();
    let full_page_code = slide.full_page_code();
    let full_page_class = if full_page_image.is_some() {
        " image-full"
    } else if full_page_code.is_some() {
        " text-full"
    } else {
        ""
    };
    let bg_style = if let Some(src) = slide.bg_image.as_ref() {
        format!(
            " style=\"background-image:url('{}')\"",
            image_data_uri(base_dir, src)
        )
    } else if let Some(tok) = slide.bg_color() {
        format!(" style=\"background:#{}\"", resolve_bg_color(tok, theme))
    } else {
        String::new()
    };
    write!(
        out,
        "<section class=\"slide slide-{}{}{}\" data-slide=\"{}\" data-line=\"{}\" aria-label=\"Slide {} of {}\"{}>\n",
        kind,
        full_page_class,
        if idx == 0 { " active" } else { "" },
        idx + 1,
        slide.source_line,
        idx + 1,
        total,
        bg_style
    )?;

    if let Some((src, alt, _)) = full_page_image {
        write!(
            out,
            "<img class=\"full-page-image\" src=\"{}\" alt=\"{}\">\n",
            escape_attr(&image_data_uri(base_dir, src)),
            escape_attr(alt)
        )?;
        if let Some(notes) = &slide.notes {
            write!(
                out,
                "<aside class=\"speaker-notes\"><h2>Speaker notes</h2><p>{}</p></aside>\n",
                escape_html(notes).replace('\n', "<br>")
            )?;
        }
        out.push_str("</section>\n");
        return Ok(());
    }
    if let Some((lines, lang)) = full_page_code {
        let math_markup = crate::math::is_markup_text_language(lang);
        let class = if math_markup {
            "full-page-text math"
        } else {
            "full-page-text"
        };
        let rendered_lines = crate::math::translate_markup_lines(lines, lang);
        write!(out, "<pre class=\"{}\">", class)?;
        out.push_str(&escape_html(&rendered_lines.join("\n")));
        out.push_str("</pre>\n");
        if let Some(notes) = &slide.notes {
            write!(
                out,
                "<aside class=\"speaker-notes\"><h2>Speaker notes</h2><p>{}</p></aside>\n",
                escape_html(notes).replace('\n', "<br>")
            )?;
        }
        out.push_str("</section>\n");
        return Ok(());
    }

    if layout.shows_rail() {
        out.push_str("<div class=\"rail\" aria-hidden=\"true\"></div>\n");
    }
    if layout.shows_sidebar() {
        write!(
            out,
            "<aside class=\"sidebar\"><div>{}</div><div>{:02}/{:02}</div></aside>\n",
            escape_html(deck_title),
            idx + 1,
            total
        )?;
    }

    match &slide.kind {
        SlideKind::Title {
            subtitle,
            author,
            date,
        } => {
            out.push_str("<div class=\"slide-inner hero\">\n");
            write!(out, "<h1>{}</h1>\n", escape_html(&slide.title))?;
            if let Some(subtitle) = subtitle {
                write!(out, "<p class=\"subtitle\">{}</p>\n", escape_html(subtitle))?;
            }
            if author.is_some() || date.is_some() {
                out.push_str("<p class=\"byline\">");
                if let Some(author) = author {
                    out.push_str(&escape_html(author));
                }
                if author.is_some() && date.is_some() {
                    out.push_str(" &middot; ");
                }
                if let Some(date) = date {
                    out.push_str(&escape_html(date));
                }
                out.push_str("</p>\n");
            }
            out.push_str("</div>\n");
        }
        SlideKind::Section => {
            out.push_str("<div class=\"slide-inner section-title\">\n");
            write!(out, "<h1>{}</h1>\n", escape_html(&slide.title))?;
            out.push_str("</div>\n");
        }
        SlideKind::Content => {
            let valign_class = match slide.valign() {
                "center" => " valign-center",
                "bottom" => " valign-bottom",
                _ => "",
            };
            let mut style_props: Vec<String> = Vec::new();
            if let Some(f) = slide.col_frac() {
                style_props.push(format!("--cols: {:.3}fr {:.3}fr", f, 1.0 - f));
            }
            let scale = slide.text_scale();
            if (scale - 1.0).abs() > f32::EPSILON {
                style_props.push(format!("--text-scale: {scale:.3}"));
            }
            let col_style = if style_props.is_empty() {
                String::new()
            } else {
                format!(" style=\"{}\"", style_props.join("; "))
            };
            let align_class = match slide.text_align() {
                "center" => " align-center",
                "right" => " align-right",
                _ => "",
            };
            write!(
                out,
                "<div class=\"slide-inner content{valign_class}{align_class}\"{col_style}>\n"
            )?;
            render_content_title(out, slide, idx + 1, total, layout, theme.title_center)?;
            out.push_str("<div class=\"blocks\">\n");
            render_blocks(out, &slide.blocks, theme, base_dir)?;
            out.push_str("</div>\n");
            if let Some(logo_uri) = logo_uri {
                write!(
                    out,
                    "<img class=\"logo\" src=\"{}\" alt=\"Logo\">\n",
                    escape_attr(logo_uri)
                )?;
            }
            out.push_str("</div>\n");
        }
    }

    if let Some(notes) = &slide.notes {
        write!(
            out,
            "<aside class=\"speaker-notes\"><h2>Speaker notes</h2><p>{}</p></aside>\n",
            escape_html(notes).replace('\n', "<br>")
        )?;
    }
    out.push_str("</section>\n");
    Ok(())
}

fn render_content_title(
    out: &mut String,
    slide: &Slide,
    num: usize,
    total: usize,
    layout: &Layout,
    title_center: bool,
) -> Result<()> {
    let mut class = if layout.title_block_bg() {
        "slide-title title-block".to_string()
    } else if layout.title_underline() {
        "slide-title title-underline".to_string()
    } else {
        "slide-title".to_string()
    };
    if title_center {
        class.push_str(" title-center");
    }
    let progress = if total == 0 {
        0.0
    } else {
        (num.max(1) as f32 / total as f32 * 100.0)
            .min(100.0)
            .max(0.1)
    };
    write!(
        out,
        "<header class=\"{}\" style=\"--slide-progress:{:.3}%\"><h1>{}</h1></header>\n",
        class,
        progress,
        escape_html(&slide.title)
    )?;
    Ok(())
}

fn render_blocks(out: &mut String, blocks: &[Block], theme: &Theme, base_dir: &Path) -> Result<()> {
    for block in blocks {
        render_block(out, block, theme, base_dir)?;
    }
    Ok(())
}

fn render_block(out: &mut String, block: &Block, theme: &Theme, base_dir: &Path) -> Result<()> {
    match block {
        Block::Paragraph(runs) => {
            out.push_str("<p>");
            render_runs(out, runs)?;
            out.push_str("</p>\n");
        }
        Block::Heading { level, runs } => {
            let tag = (*level).clamp(3, 6);
            write!(out, "<h{}>", tag)?;
            render_runs(out, runs)?;
            write!(out, "</h{}>\n", tag)?;
        }
        Block::List(items) => render_list(out, items)?,
        Block::CodeBlock {
            lang,
            title,
            lines,
            line_numbers,
            start_line,
            ..
        } => render_code_block(
            out,
            theme,
            lang.as_deref(),
            title.as_deref(),
            lines,
            *line_numbers,
            *start_line,
        )?,
        Block::Quote(paragraphs) => {
            out.push_str("<blockquote>\n");
            for runs in paragraphs {
                out.push_str("<p>");
                render_runs(out, runs)?;
                out.push_str("</p>\n");
            }
            out.push_str("</blockquote>\n");
        }
        Block::Callout { kind, body } => {
            let (icon, label) = callout_label(kind);
            write!(
                out,
                "<aside class=\"callout callout-{}\">\n<div class=\"callout-title\">{} {}</div>\n",
                escape_attr(kind),
                icon,
                label
            )?;
            for runs in body {
                out.push_str("<p>");
                render_runs(out, runs)?;
                out.push_str("</p>\n");
            }
            out.push_str("</aside>\n");
        }
        Block::Table {
            headers,
            rows,
            aligns,
        } => render_table(out, headers, rows, aligns)?,
        Block::Cards { cols, cards } => {
            write!(out, "<div class=\"cards\" style=\"--n: {}\">\n", cols)?;
            for card in cards {
                out.push_str("<div class=\"card\">\n");
                write!(out, "<h3>{}</h3>\n", escape_html(&card.title))?;
                if !card.body.is_empty() {
                    out.push_str("<p>");
                    render_runs(out, &card.body)?;
                    out.push_str("</p>\n");
                }
                out.push_str("</div>\n");
            }
            out.push_str("</div>\n");
        }
        Block::ColumnBreak => {}
        Block::Columns { left, right } => {
            out.push_str("<div class=\"columns\">\n<div>\n");
            render_blocks(out, left, theme, base_dir)?;
            out.push_str("</div>\n<div>\n");
            render_blocks(out, right, theme, base_dir)?;
            out.push_str("</div>\n</div>\n");
        }
        Block::Image {
            src,
            alt,
            width_pct,
            fit,
            rounded,
        } => {
            let math_meta = crate::math::math_image_meta(src, alt);
            let mut style_parts = Vec::new();
            if let Some(pct) = width_pct {
                style_parts.push(format!("--image-width:{}%", (*pct).clamp(1, 100)));
            }
            if let Some(meta) = math_meta {
                let (left, right) = match meta.align {
                    crate::math::MathBlockAlign::Left => ("0", "auto"),
                    crate::math::MathBlockAlign::Center => ("auto", "auto"),
                    crate::math::MathBlockAlign::Right => ("auto", "0"),
                };
                style_parts.push(format!("--math-margin-left:{left}"));
                style_parts.push(format!("--math-margin-right:{right}"));
                if let Some(max_height) = meta.max_height_px {
                    style_parts.push(format!("--math-max-height:{max_height}px"));
                }
            }
            let style = if style_parts.is_empty() {
                String::new()
            } else {
                format!(" style=\"{}\"", escape_attr(&style_parts.join(";")))
            };
            let class = if math_meta.is_some() {
                "image math-image"
            } else {
                "image"
            };
            let visible_alt = crate::math::visible_image_alt(src, alt);
            let mut img_style = Vec::new();
            if let Some(f) = fit {
                img_style.push(format!("object-fit:{f}"));
            }
            if *rounded {
                img_style.push("border-radius:.6em".to_string());
            }
            let img_style = if img_style.is_empty() {
                String::new()
            } else {
                format!(" style=\"{}\"", img_style.join(";"))
            };
            write!(
                out,
                "<figure class=\"{}\"{}><img src=\"{}\" alt=\"{}\"{}></figure>\n",
                class,
                style,
                escape_attr(&image_data_uri(base_dir, src)),
                escape_attr(visible_alt),
                img_style
            )?;
        }
        Block::Footnotes(items) => {
            out.push_str("<div class=\"footnotes\">\n");
            render_list(out, items)?;
            out.push_str("</div>\n");
        }
    }
    Ok(())
}

fn render_runs(out: &mut String, runs: &[Run]) -> Result<()> {
    for run in runs {
        let mut text = escape_html(&run.text).replace('\n', "<br>");
        if run.code {
            text = format!("<code class=\"inline-code\">{text}</code>");
        }
        if run.bold {
            text = format!("<strong>{text}</strong>");
        }
        if run.italic {
            text = format!("<em>{text}</em>");
        }
        if run.strike {
            text = format!("<s>{text}</s>");
        }
        if let Some(url) = &run.link {
            text = format!("<a href=\"{}\">{text}</a>", escape_attr(url));
        }
        out.push_str(&text);
    }
    Ok(())
}

fn render_list(out: &mut String, items: &[ListItem]) -> Result<()> {
    let mut counters = vec![0usize; 10];
    out.push_str("<div class=\"list\">\n");
    for item in items {
        let level = usize::from(item.level).min(9);
        if item.ordered {
            counters[level] += 1;
        }
        for counter in counters.iter_mut().skip(level + 1) {
            *counter = 0;
        }
        // Task-list items carry their own ☐/☑ marker in the runs, so leave the
        // bullet slot empty to avoid showing both a bullet and a checkbox.
        let marker = if item.is_task() {
            String::new()
        } else if item.ordered {
            format!("{}.", counters[level].max(1))
        } else {
            "&bull;".to_string()
        };
        write!(
            out,
            "<div class=\"list-item\" style=\"--level:{}\"><span class=\"marker\">{}</span><span>",
            level, marker
        )?;
        render_runs(out, &item.runs)?;
        out.push_str("</span></div>\n");
    }
    out.push_str("</div>\n");
    Ok(())
}

fn render_code_block(
    out: &mut String,
    theme: &Theme,
    lang: Option<&str>,
    title: Option<&str>,
    lines: &[String],
    line_numbers: bool,
    start_line: usize,
) -> Result<()> {
    out.push_str("<figure class=\"code-block\">\n");
    if title.is_some() || lang.is_some() {
        out.push_str("<figcaption>");
        if let Some(title) = title {
            out.push_str(&escape_html(title));
        }
        if title.is_some() && lang.is_some() {
            out.push_str("<span class=\"code-lang\">");
        }
        if let Some(lang) = lang {
            if title.is_some() {
                out.push_str(&escape_html(lang));
                out.push_str("</span>");
            } else {
                out.push_str(&escape_html(lang));
            }
        }
        out.push_str("</figcaption>\n");
    }
    out.push_str("<pre><code>\n");
    let token_lines = syntax::tokenize(lines, lang);
    for (idx, tokens) in token_lines.iter().enumerate() {
        if line_numbers {
            out.push_str("<span class=\"code-line\">");
        } else {
            out.push_str("<span class=\"code-line no-gutter\">");
        }
        if line_numbers {
            write!(out, "<span class=\"line-no\">{}</span>", start_line + idx)?;
        }
        out.push_str("<span class=\"code-text\">");
        if tokens.is_empty() {
            out.push('\n');
        } else {
            for token in tokens {
                write!(
                    out,
                    "<span class=\"{}\" style=\"color:#{}{}{}\">{}</span>",
                    token_class(token.kind),
                    theme.syntax_style(token.kind).color,
                    if theme.syntax_style(token.kind).italic {
                        ";font-style:italic"
                    } else {
                        ""
                    },
                    if theme.syntax_style(token.kind).bold {
                        ";font-weight:700"
                    } else {
                        ""
                    },
                    escape_html(&token.text)
                )?;
            }
        }
        out.push_str("</span></span>\n");
    }
    out.push_str("</code></pre>\n</figure>\n");
    Ok(())
}

fn render_table(
    out: &mut String,
    headers: &[Vec<Run>],
    rows: &[Vec<Vec<Run>>],
    aligns: &[ColumnAlign],
) -> Result<()> {
    let align_attr = |col: usize| -> &'static str {
        match aligns.get(col) {
            Some(ColumnAlign::Center) => " style=\"text-align:center\"",
            Some(ColumnAlign::Right) => " style=\"text-align:right\"",
            _ => "",
        }
    };
    out.push_str("<div class=\"table-wrap\"><table>\n<thead><tr>");
    for (col, cell) in headers.iter().enumerate() {
        write!(out, "<th{}>", align_attr(col))?;
        render_runs(out, cell)?;
        out.push_str("</th>");
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for row in rows {
        out.push_str("<tr>");
        for (col, cell) in row.iter().enumerate() {
            write!(out, "<td{}>", align_attr(col))?;
            render_runs(out, cell)?;
            out.push_str("</td>");
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody></table></div>\n");
    Ok(())
}

fn render_controls(out: &mut String, total: usize) -> Result<()> {
    write!(
        out,
        "<nav class=\"controls\" aria-label=\"Slide controls\">\n\
         <button type=\"button\" data-prev aria-label=\"Previous slide\">&lsaquo;</button>\n\
         <span><b data-current>1</b>/<span data-total>{}</span></span>\n\
         <button type=\"button\" data-next aria-label=\"Next slide\">&rsaquo;</button>\n\
         </nav>\n",
        total
    )?;
    Ok(())
}

fn image_data_uri(base_dir: &Path, src: &str) -> String {
    let meta = image::load_any_or_placeholder(base_dir, src);
    data_uri(&meta)
}

fn data_uri(meta: &image::ImageMeta) -> String {
    let mime = match meta.ext {
        "jpeg" | "jpg" => "image/jpeg",
        "png" => "image/png",
        _ => "application/octet-stream",
    };
    format!("data:{mime};base64,{}", base64(&meta.bytes))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn token_class(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Default => "tok-default",
        TokenKind::Keyword => "tok-keyword",
        TokenKind::String => "tok-string",
        TokenKind::Number => "tok-number",
        TokenKind::Comment => "tok-comment",
        TokenKind::Function => "tok-function",
        TokenKind::Type => "tok-type",
        TokenKind::Attribute => "tok-attribute",
    }
}

fn css(theme: &Theme, layout: &Layout, rtl: bool) -> String {
    let frame_offset = if layout.shows_sidebar() { "18%" } else { "0px" };
    format!(
        r#":root {{
  --slide-w: {slide_w};
  --slide-h: {slide_h};
  --bg: #{bg};
  --title: #{title};
  --body: #{body};
  --muted: #{muted};
  --accent: #{accent};
  --accent-soft: #{accent_soft};
  --divider: #{divider};
  --table-band-bg: #{table_band_bg};
  --code-bg: #{code_bg};
  --code-text: #{code_text};
  --link: #{link};
  --section-bg: #{section_bg};
  --section-text: #{section_text};
  --title-font: "{title_font}", system-ui, sans-serif;
  --body-font: "{body_font}", system-ui, sans-serif;
  --mono-font: "{mono_font}", ui-monospace, "SFMono-Regular", Consolas, monospace;
  --title-size: {title_size}pt;
  --body-size: {body_size}pt;
  --code-size: {code_size}pt;
  --hero-size: {hero_size}pt;
}}
* {{ box-sizing: border-box; }}
html, body {{ margin: 0; min-height: 100%; background: #111827; color: var(--body); }}
body {{ font-family: var(--body-font); overflow: hidden; overflow-wrap: anywhere; }}
.deck {{ min-height: 100vh; display: grid; place-items: center; padding: 2.25vh 2.25vw; }}
.slide {{
  display: none;
  position: relative;
  width: min(96vw, calc(96vh * var(--slide-w) / var(--slide-h)));
  aspect-ratio: var(--slide-w) / var(--slide-h);
  overflow: hidden;
  background: var(--bg);
  color: var(--body);
  box-shadow: 0 18px 48px rgba(0,0,0,.32);
  background-size: cover;
  background-position: center;
}}
.slide::before {{
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, rgba(255,255,255,.84), rgba(255,255,255,.74));
  opacity: 0;
  pointer-events: none;
}}
.theme-dark .slide::before {{ background: linear-gradient(90deg, rgba(11,18,32,.86), rgba(11,18,32,.72)); }}
.slide[style*="background-image"]::before {{ opacity: 1; }}
.slide.active {{ display: block; }}
.slide-inner {{ position: relative; z-index: 1; height: 100%; padding: 6.2% 7%; margin-left: {frame_offset}; }}
.hero, .section-title {{ display: flex; flex-direction: column; justify-content: center; }}
.hero h1, .section-title h1 {{ margin: 0; font-family: var(--title-font); font-size: clamp(30px, 5.2vw, var(--hero-size)); line-height: 1.04; color: var(--title); }}
.subtitle {{ margin: 1.2rem 0 0; max-width: 78%; color: var(--body); font-size: clamp(18px, 2.2vw, 30px); line-height: 1.22; }}
.byline {{ margin-top: 2rem; color: var(--muted); font-size: clamp(14px, 1.3vw, 18px); }}
.section-title {{ background: var(--section-bg); color: var(--section-text); margin-left: 0; }}
.section-title h1 {{ color: var(--section-text); }}
.slide-title {{ margin-bottom: 2.2%; padding-bottom: 1.1%; }}
.title-center, .title-center h1 {{ text-align: center; }}
.slide-title h1 {{ margin: 0; font-family: var(--title-font); color: var(--title); font-size: clamp(22px, 3.0vw, var(--title-size)); line-height: 1.1; }}
.title-underline {{ position: relative; }}
.title-underline::before, .title-underline::after {{ content: ""; position: absolute; left: 0; bottom: 0; display: block; }}
.title-underline::before {{ width: 100%; height: .07rem; bottom: .055rem; background: var(--divider); }}
.title-underline::after {{ width: var(--slide-progress); height: .18rem; background: var(--accent); }}
.dir-rtl .title-underline::before, .dir-rtl .title-underline::after {{ left: auto; right: 0; }}
.title-block {{ margin: -6.2% -7% 2.8%; padding: 4.2% 7% 2.6%; background: var(--accent); }}
.title-block h1 {{ color: white; }}
.slide.image-full {{ padding: 0; background: #fff; }}
.full-page-image {{ position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; display: block; }}
.slide.text-full {{ padding: 0; background: #fff; }}
.full-page-text {{ position: absolute; inset: 3.2%; margin: 0; overflow: hidden; color: var(--title); background: transparent; font: clamp(5px, .72vw, 8.5px)/1.18 var(--mono-font); white-space: pre; }}
.full-page-text.math {{ inset: 2.7%; text-align: center; font-family: var(--body-font); font-style: italic; font-size: clamp(5px, .66vw, 8.5px); line-height: 1.1; }}
.blocks {{ font-size: calc(clamp(15px, 1.45vw, var(--body-size)) * var(--text-scale, 1)); line-height: 1.34; }}
p {{ margin: .5em 0 .85em; }}
h3, h4, h5, h6 {{ margin: .9em 0 .32em; color: var(--title); font-family: var(--title-font); line-height: 1.15; }}
a {{ color: var(--link); text-decoration-thickness: .08em; text-underline-offset: .16em; }}
.inline-code {{ font-family: var(--mono-font); background: var(--code-bg); color: var(--code-text); padding: .06em .28em; border-radius: .22em; font-size: .92em; }}
.list {{ margin: .45em 0 .85em; }}
.list-item {{ display: grid; grid-template-columns: 2.1em 1fr; gap: .35em; margin: .26em 0 .26em calc(var(--level) * 1.45em); }}
.marker {{ color: var(--accent); font-weight: 700; text-align: end; }}
blockquote {{ margin: .85em 0; padding: .2em 0 .2em 1em; border-left: .22em solid var(--accent); color: var(--title); }}
.callout {{ margin: .8em 0; padding: .65em .95em; border-left: .3em solid var(--accent); background: var(--accent-soft); border-radius: .35em; }}
.callout-title {{ font-weight: 700; color: var(--title); margin-bottom: .25em; font-size: .92em; letter-spacing: .01em; }}
.callout p {{ margin: .2em 0; }}
.callout-note {{ border-color: #3b82f6; background: color-mix(in srgb, #3b82f6 13%, var(--bg)); }}
.callout-tip {{ border-color: #22c55e; background: color-mix(in srgb, #22c55e 13%, var(--bg)); }}
.callout-important {{ border-color: #a855f7; background: color-mix(in srgb, #a855f7 13%, var(--bg)); }}
.callout-warning {{ border-color: #f59e0b; background: color-mix(in srgb, #f59e0b 15%, var(--bg)); }}
.callout-caution {{ border-color: #ef4444; background: color-mix(in srgb, #ef4444 13%, var(--bg)); }}
.columns {{ display: grid; grid-template-columns: var(--cols, minmax(0, 1fr) minmax(0, 1fr)); gap: 4%; align-items: start; }}
.valign-center .columns {{ align-items: center; }}
.valign-bottom .columns {{ align-items: end; }}
.slide-inner.content.valign-center, .slide-inner.content.valign-bottom {{ display: flex; flex-direction: column; }}
.valign-center .blocks {{ margin-top: auto; margin-bottom: auto; }}
.valign-bottom .blocks {{ margin-top: auto; }}
.align-center {{ text-align: center; }}
.align-center .list-item {{ justify-content: center; }}
.align-right {{ text-align: right; }}
.cards {{ display: grid; grid-template-columns: repeat(var(--n, 3), minmax(0, 1fr)); gap: 3%; margin: .8em 0; }}
.card {{ background: var(--accent-soft); border: 1px solid var(--divider); border-radius: .5em; padding: .9em 1em; }}
.card h3 {{ margin: 0 0 .35em; color: var(--accent); font-size: 1.02em; }}
.card p {{ margin: 0; font-size: .94em; }}
.image {{ margin: .8em 0; width: var(--image-width, 100%); max-width: 100%; }}
.image img {{ display: block; width: 100%; max-height: 47vh; object-fit: contain; }}
.math-image {{ width: fit-content; margin-left: var(--math-margin-left, auto); margin-right: var(--math-margin-right, auto); }}
.math-image img {{ width: auto; max-width: 100%; max-height: var(--math-max-height, 32vh); }}
.table-wrap {{ margin: .75em 0; overflow: hidden; border: 1px solid var(--divider); }}
table {{ width: 100%; border-collapse: collapse; font-size: .82em; }}
th, td {{ padding: .42em .55em; border-bottom: 1px solid var(--divider); vertical-align: top; text-align: {text_align}; }}
th {{ background: var(--accent-soft); color: var(--title); font-weight: 700; }}
tbody tr:nth-child(even) {{ background: var(--table-band-bg); }}
.code-block {{ margin: .75em 0; background: var(--code-bg); border: 1px solid var(--divider); color: var(--code-text); overflow: hidden; }}
.code-block figcaption {{ display: flex; justify-content: space-between; gap: 1em; padding: .45em .75em; color: var(--muted); border-bottom: 1px solid var(--divider); font: 600 .72em var(--body-font); }}
.code-lang {{ margin-left: auto; text-transform: uppercase; letter-spacing: .08em; }}
pre {{ margin: 0; padding: .65em .75em; overflow: hidden; font: clamp(11px, 1.15vw, var(--code-size))/1.35 var(--mono-font); white-space: normal; }}
code {{ font-family: inherit; }}
.code-line {{ display: grid; grid-template-columns: auto 1fr; gap: .75em; min-height: 1.35em; }}
.code-line.no-gutter {{ grid-template-columns: 1fr; }}
.line-no {{ color: var(--muted); user-select: none; text-align: right; min-width: 2.2em; }}
.code-text {{ white-space: pre; overflow: hidden; text-overflow: clip; }}
.footnotes {{ position: absolute; left: 7%; right: 7%; bottom: 4.2%; color: var(--muted); font-size: .58em; }}
.speaker-notes {{ display: none; position: absolute; z-index: 4; left: 7%; right: 7%; bottom: 5%; max-height: 28%; overflow: auto; padding: 1rem; background: color-mix(in srgb, var(--bg) 92%, transparent); border: 1px solid var(--divider); box-shadow: 0 12px 36px rgba(0,0,0,.2); }}
.speaker-notes h2 {{ margin: 0 0 .5rem; font-size: .8rem; color: var(--muted); text-transform: uppercase; letter-spacing: .08em; }}
.speaker-notes p {{ margin: 0; }}
body.show-notes .slide.active .speaker-notes {{ display: block; }}
.logo {{ position: absolute; z-index: 2; right: 4.2%; bottom: 3.8%; max-width: 8%; max-height: 7%; object-fit: contain; }}
.rail {{ position: absolute; z-index: 2; inset: 0 auto 0 0; width: 1.2%; background: var(--accent); }}
.sidebar {{ position: absolute; z-index: 2; inset: 0 auto 0 0; width: 18%; padding: 5% 2.2%; background: var(--accent); color: white; display: flex; flex-direction: column; justify-content: space-between; font: 700 .82rem var(--title-font); }}
.controls {{ position: fixed; z-index: 10; right: 1rem; bottom: 1rem; display: flex; align-items: center; gap: .55rem; padding: .45rem .55rem; background: rgba(17,24,39,.82); color: white; font: 13px system-ui, sans-serif; border-radius: .45rem; }}
.controls button {{ width: 2rem; height: 2rem; border: 0; border-radius: .35rem; background: rgba(255,255,255,.12); color: white; font-size: 1.25rem; cursor: pointer; }}
.controls button:hover {{ background: rgba(255,255,255,.22); }}
.dir-rtl .slide-inner {{ margin-left: 0; margin-right: {frame_offset}; }}
.dir-rtl .rail {{ left: auto; right: 0; }}
.dir-rtl .sidebar {{ left: auto; right: 0; }}
.dir-rtl .logo {{ right: auto; left: 4.2%; }}
.dir-rtl blockquote {{ border-left: 0; border-right: .22em solid var(--accent); padding-left: 0; padding-right: 1em; }}
@media print {{
  body {{ overflow: visible; background: white; }}
  .deck {{ display: block; padding: 0; }}
  .slide {{ display: block; width: 100vw; height: 100vh; aspect-ratio: auto; box-shadow: none; page-break-after: always; break-after: page; }}
  .controls {{ display: none; }}
}}
"#,
        slide_w = theme.slide_w,
        slide_h = theme.slide_h,
        bg = theme.bg,
        title = theme.title_color,
        body = theme.body_color,
        muted = theme.muted_color,
        accent = theme.accent,
        accent_soft = theme.accent_soft,
        divider = theme.divider,
        table_band_bg = theme.table_band_bg(),
        code_bg = theme.code_bg,
        code_text = theme.code_text,
        link = theme.link,
        section_bg = theme.section_bg,
        section_text = theme.section_text,
        title_font = css_string(&theme.title_font),
        body_font = css_string(&theme.body_font),
        mono_font = css_string(&theme.mono_font),
        // Theme sizes are centipoints (e.g. 1500 = 15pt); CSS wants points.
        title_size = theme.title_size as f32 / 100.0,
        body_size = theme.body_size as f32 / 100.0,
        code_size = theme.code_size as f32 / 100.0,
        hero_size = theme.hero_size as f32 / 100.0,
        frame_offset = frame_offset,
        text_align = if rtl { "right" } else { "left" },
    )
}

fn css_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    escape_html(s)
}

/// Resolve a per-slide background token (`#hex` or palette keyword) to a hex
/// string without the leading `#`.
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

/// Icon + display label for a callout kind. Unknown kinds get a generic note.
fn callout_label(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "tip" => ("\u{1F4A1}", "Tip"),
        "important" => ("\u{2757}", "Important"),
        "warning" => ("\u{26A0}", "Warning"),
        "caution" => ("\u{1F6D1}", "Caution"),
        _ => ("\u{2139}", "Note"),
    }
}

/// Extra rules for the `--serve --edit` continuous-scroll preview: unhide every
/// slide, stack them vertically in a scrolling page, hide presenter chrome, and
/// give the caret-tracked slide a highlight ring the host shell toggles.
const EDITOR_CSS: &str = r#"
html, body.edit { overflow: auto; height: auto; }
body.edit { background: #1a1a1a; }
body.edit .deck { display: block; min-height: 0; padding: 18px 14px; }
body.edit .slide { display: block !important; margin: 0 auto 20px; scroll-margin: 14px; }
body.edit .controls { display: none !important; }
body.edit .slide { border-radius: 8px; transition: box-shadow .22s ease; }
body.edit .slide.caret { box-shadow: 0 0 0 2px var(--accent), 0 0 0 9px color-mix(in srgb, var(--accent) 18%, transparent), 0 14px 40px rgba(0,0,0,.45); }
"#;

const SCRIPT: &str = r#"(function () {
  const slides = Array.from(document.querySelectorAll('.slide'));
  const current = document.querySelector('[data-current]');
  const total = document.querySelector('[data-total]');
  let index = 0;
  if (total) total.textContent = String(slides.length);
  function show(next) {
    if (!slides.length) return;
    index = Math.max(0, Math.min(slides.length - 1, next));
    slides.forEach((slide, i) => slide.classList.toggle('active', i === index));
    if (current) current.textContent = String(index + 1);
    location.hash = String(index + 1);
  }
  function fromHash() {
    const n = Number(location.hash.replace('#', ''));
    if (Number.isFinite(n) && n > 0) show(n - 1);
  }
  document.querySelector('[data-prev]')?.addEventListener('click', () => show(index - 1));
  document.querySelector('[data-next]')?.addEventListener('click', () => show(index + 1));
  window.addEventListener('hashchange', fromHash);
  window.addEventListener('keydown', (event) => {
    if (event.target && /^(INPUT|TEXTAREA|SELECT)$/.test(event.target.tagName)) return;
    if (['ArrowRight', 'PageDown', ' ', 'Enter'].includes(event.key)) {
      event.preventDefault();
      show(index + 1);
    } else if (['ArrowLeft', 'PageUp', 'Backspace'].includes(event.key)) {
      event.preventDefault();
      show(index - 1);
    } else if (event.key === 'Home') {
      event.preventDefault();
      show(0);
    } else if (event.key === 'End') {
      event.preventDefault();
      show(slides.length - 1);
    } else if (event.key.toLowerCase() === 'n') {
      document.body.classList.toggle('show-notes');
    }
  });
  fromHash();
  show(index);
}());"#;
