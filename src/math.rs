//! Tiny LaTeX-flavoured math preprocessor. Not a real TeX engine — just
//! enough to make `$E = mc^2$` and `\sum_{i=1}^n f(x_i)` look reasonable in a
//! slide deck without pulling in a runtime renderer.
//!
//! The translator runs as a *preprocessor* pass before pulldown-cmark sees
//! the markdown. In Unicode mode it replaces `$...$` and `$$...$$` spans
//! (outside fenced code blocks) with italicised Unicode. In SVG mode it keeps
//! inline math as source and turns display spans into generated local image
//! data. Anything we can't translate is passed through verbatim so the user can
//! spot it and adjust.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathMode {
    /// Translate `$...$` and `$$...$$` into italicized Unicode text.
    Unicode,
    /// Leave math source untouched.
    Source,
    /// Render display `$$...$$` spans as generated SVG images.
    Svg,
}

impl Default for MathMode {
    fn default() -> Self {
        MathMode::Unicode
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathBlockAlign {
    Left,
    Center,
    Right,
}

impl Default for MathBlockAlign {
    fn default() -> Self {
        MathBlockAlign::Center
    }
}

impl MathBlockAlign {
    pub fn name(self) -> &'static str {
        match self {
            MathBlockAlign::Left => "left",
            MathBlockAlign::Center => "center",
            MathBlockAlign::Right => "right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathSvgOptions {
    /// Display math scale as a percentage. 100 means the built-in default.
    pub scale_percent: u16,
    /// Maximum display-math image height in logical CSS/SVG pixels.
    pub max_height_px: Option<u16>,
    pub block_align: MathBlockAlign,
}

impl Default for MathSvgOptions {
    fn default() -> Self {
        Self {
            scale_percent: 100,
            max_height_px: None,
            block_align: MathBlockAlign::Center,
        }
    }
}

impl MathSvgOptions {
    pub fn scale_factor(self) -> f32 {
        (self.scale_percent as f32 / 100.0).clamp(0.25, 4.0)
    }

    fn image_alt(self) -> String {
        let mut alt = format!(
            "math;scale={};align={}",
            self.scale_percent,
            self.block_align.name()
        );
        if let Some(max_height) = self.max_height_px {
            alt.push_str(&format!(";maxh={max_height}"));
        }
        alt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathImageMeta {
    pub align: MathBlockAlign,
    pub max_height_px: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathOptions {
    pub mode: MathMode,
    pub macros: Vec<(String, String)>,
    pub svg: MathSvgOptions,
}

impl MathOptions {
    pub fn new(mode: MathMode) -> Self {
        Self {
            mode,
            macros: Vec::new(),
            svg: MathSvgOptions::default(),
        }
    }
}

impl Default for MathOptions {
    fn default() -> Self {
        MathOptions::new(MathMode::Unicode)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathDiagnostic {
    pub line: usize,
    pub kind: &'static str,
    pub detail: String,
    pub source: String,
}

/// Walk the source string and replace math spans. Skips fenced code blocks
/// (` ``` ` / `~~~`) so dollar signs inside code are left alone.
pub fn translate(input: &str) -> String {
    translate_with_mode(input, MathMode::Unicode)
}

pub fn translate_with_mode(input: &str, mode: MathMode) -> String {
    translate_with_options(input, &MathOptions::new(mode))
}

pub fn translate_with_options(input: &str, options: &MathOptions) -> String {
    if matches!(options.mode, MathMode::Source) {
        return input.to_string();
    }
    if matches!(options.mode, MathMode::Svg) {
        return translate_svg_mode(input, options);
    }
    let mut out = String::with_capacity(input.len());
    let mut in_code = false;
    for line in input.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            out.push_str(line);
            continue;
        }
        if in_code {
            out.push_str(line);
            continue;
        }
        translate_line(line, &mut out, &options.macros);
    }
    out
}

pub fn is_generated_math_image(src: &str, alt: &str) -> bool {
    math_image_meta(src, alt).is_some()
}

pub fn math_image_meta(src: &str, alt: &str) -> Option<MathImageMeta> {
    if !src.starts_with("data:image/svg+xml;base64,") {
        return None;
    }
    let mut parts = alt.split(';');
    if parts.next()?.trim() != "math" {
        return None;
    }
    let mut meta = MathImageMeta {
        align: MathBlockAlign::Center,
        max_height_px: None,
    };
    for part in parts {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        match key {
            "align" => {
                meta.align = match value {
                    "left" => MathBlockAlign::Left,
                    "right" => MathBlockAlign::Right,
                    _ => MathBlockAlign::Center,
                };
            }
            "maxh" => {
                meta.max_height_px = value.parse::<u16>().ok();
            }
            _ => {}
        }
    }
    Some(meta)
}

pub fn visible_image_alt<'a>(src: &str, alt: &'a str) -> &'a str {
    if is_generated_math_image(src, alt) {
        ""
    } else {
        alt
    }
}

pub fn is_markup_text_language(lang: Option<&str>) -> bool {
    matches!(
        lang.map(|lang| lang.trim().to_ascii_lowercase()),
        Some(lang)
            if matches!(
                lang.as_str(),
                "math" | "md2any-math" | "math-unicode" | "unicode-math"
            )
    )
}

pub fn translate_markup_text(src: &str) -> String {
    translate_math(src)
}

pub fn translate_markup_lines(lines: &[String], lang: Option<&str>) -> Vec<String> {
    if is_markup_text_language(lang) {
        lines
            .iter()
            .map(|line| translate_markup_text(line))
            .collect()
    } else {
        lines.to_vec()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MathTextLayout {
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
    pub draws: Vec<MathLayoutDraw>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MathLayoutDraw {
    Text {
        x: f32,
        y: f32,
        size: f32,
        text: String,
        bold: bool,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke_width: f32,
    },
    Polyline {
        points: Vec<(f32, f32)>,
        stroke_width: f32,
    },
    Delimiter {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        token: String,
        stroke_width: f32,
    },
}

pub fn layout_markup_text(src: &str, scale_percent: u16) -> MathTextLayout {
    layout_markup_text_with(src, scale_percent, &DejaVuMetrics)
}

/// Lay out a markup-math line, measuring glyph advances against `metrics`.
/// Renderers that typeset math in a face other than the bundled DejaVu Sans
/// (e.g. the PDF writer under `--font`) pass a provider backed by that face
/// so the reserved widths match the glyphs actually drawn.
pub fn layout_markup_text_with(
    src: &str,
    scale_percent: u16,
    metrics: &dyn GlyphMetrics,
) -> MathTextLayout {
    let expanded = apply_user_macros(src.trim(), &[]);
    let node = MathParser::new(&expanded).parse_root();
    let base_size = 28.0 * (scale_percent as f32 / 100.0).clamp(0.25, 4.0);
    let ctx = LayoutCtx {
        metrics,
        bold: false,
    };
    publish_math_box(ctx.layout_math(&node, base_size))
}

pub fn decode_generated_math_svg(src: &str) -> Option<String> {
    let data = src.strip_prefix("data:image/svg+xml;base64,")?;
    let bytes = decode_base64(data)?;
    String::from_utf8(bytes).ok()
}

fn translate_svg_mode(input: &str, options: &MathOptions) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_code = false;
    let mut in_display = false;
    let mut display = String::new();

    for line in input.split_inclusive('\n') {
        if in_display {
            if let Some(end) = find_display_close(line) {
                display.push_str(&line[..end]);
                emit_math_svg_image(&display, &mut out, options);
                display.clear();
                in_display = false;
                translate_line_svg(&line[end + 2..], &mut out, options);
            } else {
                display.push_str(line);
            }
            continue;
        }

        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            out.push_str(line);
            continue;
        }
        if in_code {
            out.push_str(line);
            continue;
        }

        if let Some(open) = find_unclosed_display_open(line) {
            translate_line_svg(&line[..open], &mut out, options);
            display.push_str(&line[open + 2..]);
            in_display = true;
        } else {
            translate_line_svg(line, &mut out, options);
        }
    }

    if in_display {
        out.push_str("$$");
        out.push_str(&display);
    }
    out
}

/// Inspect math spans for constructs the Unicode translator cannot render
/// faithfully. This is intentionally conservative: it catches common rich
/// TeX features before they silently degrade into literal source text.
pub fn diagnose(input: &str) -> Vec<MathDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut in_code = false;
    for (idx, line) in input.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if !in_code {
            diagnose_line(line, idx + 1, &mut diagnostics);
        }
    }
    diagnostics
}

fn translate_line(line: &str, out: &mut String, macros: &[(String, String)]) {
    // Scan over byte indices but always slice on UTF-8 boundaries so we
    // never split a multi-byte char. The `$`, `\`, and `` ` `` markers we
    // care about are pure ASCII, so byte indices align with character
    // boundaries.
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Pass-through for inline code spans. CommonMark: a run of N
        // backticks opens a code span that closes at the next run of
        // exactly N backticks. Math syntax inside is literal — copy it
        // verbatim so `$x$` and `x^2` in the docs survive intact.
        if bytes[i] == b'`' {
            let mut n = 0;
            while i + n < bytes.len() && bytes[i + n] == b'`' {
                n += 1;
            }
            if let Some(close) = find_code_span_close(bytes, i + n, n) {
                let end = close + n;
                out.push_str(&line[i..end]);
                i = end;
                continue;
            }
            // No matching close — treat the run as literal text.
            out.push_str(&line[i..i + n]);
            i += n;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'$' {
                if let Some(end) = find_close(bytes, i + 2, b"$$") {
                    let inner = &line[i + 2..end];
                    out.push_str(&render_math(inner, true, macros));
                    i = end + 2;
                    continue;
                }
            } else if let Some(end) = find_close(bytes, i + 1, b"$") {
                let inner = &line[i + 1..end];
                if !inner.contains('\n') {
                    out.push_str(&render_math(inner, false, macros));
                    i = end + 1;
                    continue;
                }
            }
        }
        // Copy one full UTF-8 character at a time.
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&line[i..ch_end]);
        i = ch_end;
    }
}

fn translate_line_svg(line: &str, out: &mut String, options: &MathOptions) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let mut n = 0;
            while i + n < bytes.len() && bytes[i + n] == b'`' {
                n += 1;
            }
            if let Some(close) = find_code_span_close(bytes, i + n, n) {
                let end = close + n;
                out.push_str(&line[i..end]);
                i = end;
                continue;
            }
            out.push_str(&line[i..i + n]);
            i += n;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            continue;
        }
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            if let Some(end) = find_close(bytes, i + 2, b"$$") {
                let inner = &line[i + 2..end];
                emit_math_svg_image(inner, out, options);
                i = end + 2;
                continue;
            }
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&line[i..ch_end]);
        i = ch_end;
    }
}

fn emit_math_svg_image(src: &str, out: &mut String, options: &MathOptions) {
    out.push_str("\n\n![");
    out.push_str(&options.svg.image_alt());
    out.push_str("](");
    out.push_str(&render_math_svg_data_uri(src, &options.macros, options.svg));
    out.push_str(")\n\n");
}

fn find_display_close(line: &str) -> Option<usize> {
    find_close(line.as_bytes(), 0, b"$$")
}

fn find_unclosed_display_open(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let mut n = 0;
            while i + n < bytes.len() && bytes[i + n] == b'`' {
                n += 1;
            }
            if let Some(close) = find_code_span_close(bytes, i + n, n) {
                i = close + n;
                continue;
            }
            i += n;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            i += 2;
            continue;
        }
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            if let Some(close) = find_close(bytes, i + 2, b"$$") {
                i = close + 2;
                continue;
            }
            return Some(i);
        }
        i = utf8_char_end(bytes, i);
    }
    None
}

fn render_math_svg_data_uri(
    src: &str,
    macros: &[(String, String)],
    options: MathSvgOptions,
) -> String {
    let svg = render_math_svg(src, macros, options);
    format!("data:image/svg+xml;base64,{}", base64(svg.as_bytes()))
}

fn render_math_svg(src: &str, macros: &[(String, String)], options: MathSvgOptions) -> String {
    let expanded = apply_user_macros(src.trim(), macros);
    let node = MathParser::new(&expanded).parse_root();
    let base_size = 28.0 * options.scale_factor();
    // The SVG-image path always renders math in the bundled DejaVu face
    // (see the <g font-family> below), so DejaVu metrics are exact here.
    let layout = LayoutCtx {
        metrics: &DejaVuMetrics,
        bold: false,
    }
    .layout_math(&node, base_size);
    let pad_x = base_size;
    let pad_y = base_size * 0.85;
    let natural_width = (layout.width + pad_x * 2.0).ceil().clamp(96.0, 2200.0);
    let natural_height = (layout.height + pad_y * 2.0).ceil().clamp(48.0, 1600.0);
    let (width, height) = if let Some(max_height) = options.max_height_px {
        let max_height = f32::from(max_height).max(24.0);
        if natural_height > max_height {
            let scale = max_height / natural_height;
            ((natural_width * scale).ceil().max(96.0), max_height)
        } else {
            (natural_width, natural_height)
        }
    } else {
        (natural_width, natural_height)
    };
    let mut body = String::new();
    render_box(&layout, pad_x, pad_y, &mut body);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {natural_width} {natural_height}" role="img" aria-label="math">
  <rect width="{natural_width}" height="{natural_height}" rx="12" fill="#ffffff" fill-opacity="0"/>
  <g font-family="DejaVu Sans, DejaVu Sans Mono, sans-serif" fill="#111827" stroke="#111827" stroke-linecap="round" stroke-linejoin="round">
{body}  </g>
</svg>"##
    )
}

#[derive(Debug, Clone)]
enum MathNode {
    Row(Vec<MathNode>),
    Text(String),
    Fraction(Box<MathNode>, Box<MathNode>),
    Sqrt(Box<MathNode>),
    Delimited {
        left: String,
        body: Box<MathNode>,
        right: String,
    },
    Script {
        base: Box<MathNode>,
        sub: Option<Box<MathNode>>,
        sup: Option<Box<MathNode>>,
    },
    Matrix {
        rows: Vec<Vec<MathNode>>,
        left: &'static str,
        right: &'static str,
    },
    Cases(Vec<(MathNode, Option<MathNode>)>),
    Lines(Vec<MathNode>),
    /// Bold weight applied to everything inside (`\mathbf`, `\boldsymbol`, …).
    Bold(Box<MathNode>),
    /// An accent (`\bar`, `\hat`, `\vec`, …) drawn centred over `base`. `mark`
    /// is the Unicode combining char that names the accent; the layout engine
    /// renders it as geometry rather than a font combining glyph, because
    /// zero-advance combining marks land at the base's right edge instead of
    /// centred over it (and differently in every font).
    Accent {
        base: Box<MathNode>,
        mark: char,
    },
}

struct MathParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> MathParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn parse_root(mut self) -> MathNode {
        let rows = split_top_level_math_rows(self.src);
        if rows.len() > 1 {
            return MathNode::Lines(
                rows.into_iter()
                    .map(|row| MathParser::new(&row).parse_row(None))
                    .collect(),
            );
        }
        self.parse_row(None)
    }

    fn parse_row(&mut self, stop: Option<u8>) -> MathNode {
        let mut nodes = Vec::new();
        while self.pos < self.bytes.len() {
            if stop.is_some() && Some(self.bytes[self.pos]) == stop {
                break;
            }
            let atom = self.parse_atom(stop);
            if let Some(atom) = atom {
                let atom = self.parse_scripts(atom, stop);
                push_text_node(&mut nodes, atom);
            } else {
                break;
            }
        }
        match nodes.len() {
            0 => MathNode::Text(String::new()),
            1 => nodes.remove(0),
            _ => MathNode::Row(nodes),
        }
    }

    fn parse_atom(&mut self, stop: Option<u8>) -> Option<MathNode> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let b = self.bytes[self.pos];
        if stop.is_some() && Some(b) == stop {
            return None;
        }
        match b {
            b'{' => {
                self.pos += 1;
                let group = self.parse_row(Some(b'}'));
                if self.pos < self.bytes.len() && self.bytes[self.pos] == b'}' {
                    self.pos += 1;
                }
                Some(group)
            }
            b'}' => None,
            b'\\' => Some(self.parse_command()),
            b'^' | b'_' => {
                self.pos += 1;
                Some(MathNode::Text(char::from(b).to_string()))
            }
            b'&' => {
                self.pos += 1;
                Some(MathNode::Text(" ".into()))
            }
            _ if b.is_ascii_whitespace() => {
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                    self.pos += 1;
                }
                Some(MathNode::Text(" ".into()))
            }
            _ => Some(self.parse_text_run(stop)),
        }
    }

    fn parse_command(&mut self) -> MathNode {
        let start = self.pos;
        self.pos += 1;
        if self.pos >= self.bytes.len() {
            return MathNode::Text("\\".into());
        }
        let next = self.bytes[self.pos];
        if next == b'\\' {
            self.pos += 1;
            return MathNode::Text("\n".into());
        }
        if !char::from(next).is_ascii_alphabetic() {
            self.pos += 1;
            return MathNode::Text(match next {
                b',' | b':' | b';' | b' ' => " ".into(),
                b'!' => String::new(),
                b'|' => "‖".into(),
                b'{' => "{".into(),
                b'}' => "}".into(),
                b'(' => "(".into(),
                b')' => ")".into(),
                b'[' => "[".into(),
                b']' => "]".into(),
                b'_' => "_".into(),
                _ => char::from(next).to_string(),
            });
        }
        let name_start = self.pos;
        while self.pos < self.bytes.len() && char::from(self.bytes[self.pos]).is_ascii_alphabetic()
        {
            self.pos += 1;
        }
        let name = &self.src[name_start..self.pos];
        let starred = if self.pos < self.bytes.len() && self.bytes[self.pos] == b'*' {
            self.pos += 1;
            true
        } else {
            false
        };
        match name {
            // `\dfrac`/`\tfrac`/`\cfrac` differ from `\frac` only in display
            // style, which this engine already approximates per nesting depth.
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.parse_required_group_node();
                let den = self.parse_required_group_node();
                MathNode::Fraction(Box::new(num), Box::new(den))
            }
            "sqrt" => {
                self.skip_optional_group();
                MathNode::Sqrt(Box::new(self.parse_required_group_node()))
            }
            "binom" => {
                let top = self.parse_required_group_node();
                let bottom = self.parse_required_group_node();
                MathNode::Matrix {
                    rows: vec![vec![top], vec![bottom]],
                    left: "(",
                    right: ")",
                }
            }
            "begin" => self.parse_environment(),
            "left" => self.parse_left_right(),
            "right" => {
                let _ = self.parse_delimiter_token();
                MathNode::Text(String::new())
            }
            "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl"
            | "biggr" | "Biggl" | "Biggr" => MathNode::Text(self.parse_delimiter_token()),
            "limits" | "nolimits" => MathNode::Text(String::new()),
            "mathbf" | "textbf" | "boldsymbol" | "bm" | "pmb" | "mathbfit" => {
                MathNode::Bold(Box::new(self.parse_required_group_node()))
            }
            "text" | "textrm" | "textit" | "mathrm" | "mathit" | "mathsf" | "mathtt" => {
                MathNode::Text(self.parse_required_group_raw())
            }
            "operatorname" => {
                let mut name = self.parse_required_group_raw();
                if starred && !name.is_empty() {
                    name.push(' ');
                }
                MathNode::Text(name)
            }
            "mathbb" => MathNode::Text(map_math_alphabet(
                &self.parse_required_group_raw(),
                blackboard_char,
            )),
            "mathcal" => MathNode::Text(map_math_alphabet(
                &self.parse_required_group_raw(),
                script_char,
            )),
            _ if accent_mark(name).is_some() => {
                let mark = accent_mark(name).unwrap_or('\u{0302}');
                let arg = self.parse_required_group_node();
                MathNode::Accent {
                    base: Box::new(arg),
                    mark,
                }
            }
            _ => {
                if let Some(rep) = greek_or_symbol(name) {
                    MathNode::Text(rep.into())
                } else {
                    MathNode::Text(self.src[start..self.pos].into())
                }
            }
        }
    }

    fn parse_environment(&mut self) -> MathNode {
        let name = self.parse_required_group_raw();
        let marker = format!("\\end{{{name}}}");
        let Some(rel_end) = self.src[self.pos..].find(&marker) else {
            return MathNode::Text(format!("\\begin{{{name}}}"));
        };
        let inner_start = self.pos;
        let inner_end = self.pos + rel_end;
        self.pos = inner_end + marker.len();
        let inner = &self.src[inner_start..inner_end];
        match name.as_str() {
            "matrix" | "pmatrix" | "bmatrix" | "Bmatrix" | "vmatrix" | "Vmatrix"
            | "smallmatrix" => {
                let (left, right) = match name.as_str() {
                    "pmatrix" => ("(", ")"),
                    "bmatrix" => ("[", "]"),
                    "Bmatrix" => ("{", "}"),
                    "vmatrix" => ("|", "|"),
                    "Vmatrix" => ("‖", "‖"),
                    _ => ("", ""),
                };
                MathNode::Matrix {
                    rows: parse_matrix_rows(inner),
                    left,
                    right,
                }
            }
            "cases" => MathNode::Cases(parse_case_rows(inner)),
            "aligned" | "align" | "alignat" | "gather" | "equation" | "split" => MathNode::Matrix {
                rows: parse_matrix_rows(inner),
                left: "",
                right: "",
            },
            "array" => MathNode::Matrix {
                rows: parse_matrix_rows(strip_array_spec(inner)),
                left: "",
                right: "",
            },
            _ => MathNode::Text(inner.into()),
        }
    }

    fn parse_left_right(&mut self) -> MathNode {
        let left = self.parse_delimiter_token();
        let Some((inner_start, inner_end, right)) = self.find_matching_right() else {
            return MathNode::Text(left);
        };
        let body = MathParser::new(&self.src[inner_start..inner_end]).parse_root();
        MathNode::Delimited {
            left,
            body: Box::new(body),
            right,
        }
    }

    fn find_matching_right(&mut self) -> Option<(usize, usize, String)> {
        let inner_start = self.pos;
        let mut i = self.pos;
        let mut brace_depth = 0usize;
        let mut left_depth = 0usize;
        while i < self.bytes.len() {
            match self.bytes[i] {
                b'{' => {
                    brace_depth += 1;
                    i += 1;
                }
                b'}' => {
                    brace_depth = brace_depth.saturating_sub(1);
                    i += 1;
                }
                b'\\' if brace_depth == 0 && starts_math_command(self.src, i, "left") => {
                    left_depth += 1;
                    i += "\\left".len();
                }
                b'\\' if brace_depth == 0 && starts_math_command(self.src, i, "right") => {
                    if left_depth == 0 {
                        let inner_end = i;
                        self.pos = i + "\\right".len();
                        let right = self.parse_delimiter_token();
                        return Some((inner_start, inner_end, right));
                    }
                    left_depth = left_depth.saturating_sub(1);
                    i += "\\right".len();
                }
                _ => i = utf8_char_end(self.bytes, i),
            }
        }
        None
    }

    fn parse_delimiter_token(&mut self) -> String {
        self.skip_spaces();
        if self.pos >= self.bytes.len() {
            return String::new();
        }
        if self.bytes[self.pos] == b'.' {
            self.pos += 1;
            return String::new();
        }
        if self.bytes[self.pos] == b'\\' {
            let start = self.pos;
            self.pos += 1;
            if self.pos < self.bytes.len() && (self.bytes[self.pos] as char).is_ascii_alphabetic() {
                let name_start = self.pos;
                while self.pos < self.bytes.len()
                    && (self.bytes[self.pos] as char).is_ascii_alphabetic()
                {
                    self.pos += 1;
                }
                return delimiter_name(&self.src[name_start..self.pos]).to_string();
            }
            if self.pos < self.bytes.len() {
                let ch_end = utf8_char_end(self.bytes, self.pos);
                let token = &self.src[start..ch_end];
                self.pos = ch_end;
                return delimiter_name(token.trim_start_matches('\\')).to_string();
            }
            return "\\".into();
        }
        let ch_end = utf8_char_end(self.bytes, self.pos);
        let token = self.src[self.pos..ch_end].to_string();
        self.pos = ch_end;
        delimiter_name(&token).to_string()
    }

    fn parse_scripts(&mut self, base: MathNode, stop: Option<u8>) -> MathNode {
        let mut sub = None;
        let mut sup = None;
        loop {
            if self.pos >= self.bytes.len() {
                break;
            }
            if stop.is_some() && Some(self.bytes[self.pos]) == stop {
                break;
            }
            match self.bytes[self.pos] {
                b'_' => {
                    self.pos += 1;
                    sub = Some(Box::new(self.parse_script_arg(stop)));
                }
                b'^' => {
                    self.pos += 1;
                    sup = Some(Box::new(self.parse_script_arg(stop)));
                }
                _ => break,
            }
        }
        if sub.is_none() && sup.is_none() {
            base
        } else {
            MathNode::Script {
                base: Box::new(base),
                sub,
                sup,
            }
        }
    }

    fn parse_script_arg(&mut self, stop: Option<u8>) -> MathNode {
        if self.pos < self.bytes.len() && self.bytes[self.pos] == b'{' {
            self.pos += 1;
            let group = self.parse_row(Some(b'}'));
            if self.pos < self.bytes.len() && self.bytes[self.pos] == b'}' {
                self.pos += 1;
            }
            group
        } else if self.pos < self.bytes.len() && self.bytes[self.pos] == b'\\' {
            self.parse_command()
        } else if self.pos < self.bytes.len() {
            if stop.is_some() && Some(self.bytes[self.pos]) == stop {
                return MathNode::Text(String::new());
            }
            let end = utf8_char_end(self.bytes, self.pos);
            let text = self.src[self.pos..end].to_string();
            self.pos = end;
            MathNode::Text(text)
        } else {
            MathNode::Text(String::new())
        }
    }

    fn parse_required_group_node(&mut self) -> MathNode {
        self.skip_spaces();
        if self.pos >= self.bytes.len() || self.bytes[self.pos] != b'{' {
            return MathNode::Text(String::new());
        }
        self.pos += 1;
        let group = self.parse_row(Some(b'}'));
        if self.pos < self.bytes.len() && self.bytes[self.pos] == b'}' {
            self.pos += 1;
        }
        group
    }

    fn parse_required_group_raw(&mut self) -> String {
        self.skip_spaces();
        if self.pos >= self.bytes.len() || self.bytes[self.pos] != b'{' {
            return String::new();
        }
        let open = self.pos;
        let (end, inner) = read_group(self.bytes, open);
        if let Some(end) = end {
            self.pos = end + 1;
        }
        inner
    }

    fn skip_optional_group(&mut self) {
        self.skip_spaces();
        if self.pos >= self.bytes.len() || self.bytes[self.pos] != b'[' {
            return;
        }
        self.pos += 1;
        let mut depth = 1;
        while self.pos < self.bytes.len() && depth > 0 {
            match self.bytes[self.pos] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn skip_spaces(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn parse_text_run(&mut self, stop: Option<u8>) -> MathNode {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if stop.is_some() && Some(b) == stop {
                break;
            }
            if matches!(b, b'\\' | b'{' | b'}' | b'^' | b'_' | b'&') || b.is_ascii_whitespace() {
                break;
            }
            self.pos = utf8_char_end(self.bytes, self.pos);
        }
        if self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b'^' | b'_') {
            let run = &self.src[start..self.pos];
            if let Some((last_offset, _)) = run.char_indices().last() {
                if last_offset > 0 {
                    self.pos = start + last_offset;
                }
            }
        }
        MathNode::Text(self.src[start..self.pos].into())
    }
}

fn push_text_node(nodes: &mut Vec<MathNode>, node: MathNode) {
    match node {
        MathNode::Text(text) if text.is_empty() => {}
        MathNode::Text(text) => {
            if let Some(MathNode::Text(prev)) = nodes.last_mut() {
                prev.push_str(&text);
            } else {
                nodes.push(MathNode::Text(text));
            }
        }
        other => nodes.push(other),
    }
}

fn split_top_level_math_rows(src: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut brace_depth = 0usize;
    let mut env_depth = 0usize;
    let mut left_depth = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            brace_depth += 1;
            current.push('{');
            i += 1;
            continue;
        }
        if bytes[i] == b'}' {
            brace_depth = brace_depth.saturating_sub(1);
            current.push('}');
            i += 1;
            continue;
        }
        if src[i..].starts_with("\\begin") {
            env_depth += 1;
        } else if src[i..].starts_with("\\end") {
            env_depth = env_depth.saturating_sub(1);
        }
        if brace_depth == 0 && starts_math_command(src, i, "left") {
            left_depth += 1;
        } else if brace_depth == 0 && starts_math_command(src, i, "right") {
            left_depth = left_depth.saturating_sub(1);
        }
        if brace_depth == 0
            && env_depth == 0
            && left_depth == 0
            && i + 1 < bytes.len()
            && &bytes[i..i + 2] == b"\\\\"
        {
            rows.push(current.trim().to_string());
            current.clear();
            i += 2;
            continue;
        }
        if brace_depth == 0 && env_depth == 0 && left_depth == 0 && bytes[i] == b'\n' {
            if !current.trim().is_empty() {
                rows.push(current.trim().to_string());
                current.clear();
            }
            i += 1;
            continue;
        }
        let end = utf8_char_end(bytes, i);
        current.push_str(&src[i..end]);
        i = end;
    }
    if !current.trim().is_empty() || rows.is_empty() {
        rows.push(current.trim().to_string());
    }
    rows
}

fn starts_math_command(src: &str, pos: usize, name: &str) -> bool {
    let rest = &src[pos..];
    let needle = format!("\\{name}");
    if !rest.starts_with(&needle) {
        return false;
    }
    rest.as_bytes()
        .get(needle.len())
        .map_or(true, |b| !(*b as char).is_ascii_alphabetic())
}

fn parse_matrix_rows(inner: &str) -> Vec<Vec<MathNode>> {
    split_math_rows(inner)
        .into_iter()
        .map(|row| {
            row.split('&')
                .map(|cell| MathParser::new(cell.trim()).parse_root())
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
        .collect()
}

fn parse_case_rows(inner: &str) -> Vec<(MathNode, Option<MathNode>)> {
    split_math_rows(inner)
        .into_iter()
        .filter_map(|row| {
            if row.trim().is_empty() {
                return None;
            }
            let cells = row
                .split('&')
                .map(|cell| MathParser::new(cell.trim()).parse_root())
                .collect::<Vec<_>>();
            let expr = cells
                .get(0)
                .cloned()
                .unwrap_or_else(|| MathNode::Text(String::new()));
            let condition = cells.get(1).cloned();
            Some((expr, condition))
        })
        .collect()
}

fn strip_array_spec(inner: &str) -> &str {
    let trimmed = inner.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.first() != Some(&b'{') {
        return inner;
    }
    let (end, _) = read_group(bytes, 0);
    if let Some(end) = end {
        &trimmed[end + 1..]
    } else {
        inner
    }
}

fn delimiter_name(token: &str) -> &str {
    match token {
        "lbrace" | "{" => "{",
        "rbrace" | "}" => "}",
        "langle" => "⟨",
        "rangle" => "⟩",
        "vert" | "lvert" | "rvert" | "mid" | "|" => "|",
        "Vert" | "lVert" | "rVert" | "parallel" => "‖",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "lbrack" | "[" => "[",
        "rbrack" | "]" => "]",
        "(" | ")" => token,
        "" | "." => "",
        _ => token,
    }
}

#[derive(Debug, Clone)]
struct MathBox {
    width: f32,
    height: f32,
    baseline: f32,
    draws: Vec<Draw>,
}

#[derive(Debug, Clone)]
enum Draw {
    Text {
        x: f32,
        y: f32,
        size: f32,
        text: String,
        bold: bool,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke_width: f32,
    },
    Polyline {
        points: Vec<(f32, f32)>,
        stroke_width: f32,
    },
    Delimiter {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        token: String,
        stroke_width: f32,
    },
}

/// Source of glyph advance widths for the layout engine. A provider reports
/// each glyph's advance as a fraction of the em **in the face that will
/// actually render it**. Because the layout reserves horizontal space here
/// while the renderer advances glyphs with the font's own metrics, the two
/// only stay aligned when the provider matches the rendering face. The
/// SVG/PNG paths always typeset math in the bundled DejaVu Sans and so use
/// [`DejaVuMetrics`]; the PDF path supplies a provider backed by its embedded
/// face, so a `--font` replacement stays pixel-aligned instead of drifting.
pub trait GlyphMetrics {
    /// Advance width of `ch` as a fraction of the em, at the given weight.
    /// `bold` selects the bold face's metrics — bold glyphs are ~12% wider
    /// in DejaVu, so a bold run reserves its own space.
    fn advance_em(&self, ch: char, bold: bool) -> f32;
}

/// Built-in metrics for the bundled DejaVu Sans face. Also the sensible
/// fallback for any glyph a custom face cannot supply, since unmapped glyphs
/// degrade to DejaVu (or a fallback font) at render time anyway.
pub struct DejaVuMetrics;

/// Mean advance ratio of DejaVu Sans **Bold** over Regular (measured across
/// the math glyph set). Bold runs in the SVG/PNG paths — which always render
/// in DejaVu — reserve width with this scale; the PDF path measures the real
/// bold face instead, so this approximation only affects short SVG bold runs.
const DEJAVU_BOLD_WIDTH_SCALE: f32 = 1.12;

impl GlyphMetrics for DejaVuMetrics {
    fn advance_em(&self, ch: char, bold: bool) -> f32 {
        let w = char_width_factor(ch);
        if bold {
            w * DEJAVU_BOLD_WIDTH_SCALE
        } else {
            w
        }
    }
}

/// Carries the active [`GlyphMetrics`] and weight through the recursive
/// layout pass so every `layout_*` step measures text against the same face.
struct LayoutCtx<'m> {
    metrics: &'m dyn GlyphMetrics,
    bold: bool,
}

impl<'m> LayoutCtx<'m> {
    /// A child context with bold weight enabled for everything it lays out.
    fn bolded(&self) -> LayoutCtx<'m> {
        LayoutCtx {
            metrics: self.metrics,
            bold: true,
        }
    }

    fn layout_math(&self, node: &MathNode, size: f32) -> MathBox {
        match node {
            MathNode::Text(text) => self.layout_text(text, size),
            MathNode::Row(nodes) => self.layout_row(nodes, size),
            MathNode::Fraction(num, den) => self.layout_fraction(num, den, size),
            MathNode::Sqrt(inner) => self.layout_sqrt(inner, size),
            MathNode::Delimited { left, body, right } => {
                self.layout_delimited(left, body, right, size)
            }
            MathNode::Script { base, sub, sup } => {
                self.layout_script(base, sub.as_deref(), sup.as_deref(), size)
            }
            MathNode::Matrix { rows, left, right } => self.layout_matrix(rows, left, right, size),
            MathNode::Cases(rows) => self.layout_cases(rows, size),
            MathNode::Lines(lines) => self.layout_lines(lines, size),
            MathNode::Bold(inner) => self.bolded().layout_math(inner, size),
            MathNode::Accent { base, mark } => self.layout_accent(base, *mark, size),
        }
    }

    /// Lay out an accent (`\bar`, `\hat`, …) by drawing it as geometry centred
    /// over the base. This sidesteps font combining-mark placement, which puts
    /// zero-advance marks at the base's right edge rather than over it.
    fn layout_accent(&self, base: &MathNode, mark: char, size: f32) -> MathBox {
        let base_box = self.layout_math(base, size);
        let stroke = (size * 0.05).max(1.2);
        // Vertical room added above the base for the accent.
        let pad = size * 0.20;
        let w = base_box.width;
        let cx = w / 2.0;
        // Accent baseline within the padding band, a little above the base ink.
        let ay = pad * 0.45;
        let mut draws = Vec::new();
        append_draws(&mut draws, &base_box, 0.0, pad);
        match mark {
            // bar / overline — a horizontal rule spanning the base.
            '\u{0304}' | '\u{0305}' => {
                let half = (w * 0.5 - size * 0.03).max(size * 0.14);
                draws.push(Draw::Line {
                    x1: cx - half,
                    y1: ay,
                    x2: cx + half,
                    y2: ay,
                    stroke_width: stroke,
                });
            }
            // hat — a caret.
            '\u{0302}' => {
                let hw = (w * 0.32).clamp(size * 0.14, size * 0.30);
                draws.push(Draw::Polyline {
                    points: vec![
                        (cx - hw, ay + size * 0.10),
                        (cx, ay - size * 0.03),
                        (cx + hw, ay + size * 0.10),
                    ],
                    stroke_width: stroke,
                });
            }
            // tilde — a shallow wave.
            '\u{0303}' => {
                let hw = (w * 0.32).clamp(size * 0.14, size * 0.30);
                let a = size * 0.05;
                draws.push(Draw::Polyline {
                    points: vec![
                        (cx - hw, ay + a),
                        (cx - hw * 0.3, ay - a),
                        (cx + hw * 0.3, ay + a),
                        (cx + hw, ay - a),
                    ],
                    stroke_width: stroke,
                });
            }
            // vec — a right arrow.
            '\u{20d7}' => {
                let half = (w * 0.5).max(size * 0.16);
                draws.push(Draw::Line {
                    x1: cx - half,
                    y1: ay,
                    x2: cx + half,
                    y2: ay,
                    stroke_width: stroke,
                });
                let hh = size * 0.07;
                draws.push(Draw::Polyline {
                    points: vec![
                        (cx + half - size * 0.11, ay - hh),
                        (cx + half, ay),
                        (cx + half - size * 0.11, ay + hh),
                    ],
                    stroke_width: stroke,
                });
            }
            // dot / ddot — round caps make a short stroke read as a dot.
            '\u{0307}' => {
                draws.push(dot_draw(cx, ay, size));
            }
            '\u{0308}' => {
                let dx = size * 0.13;
                draws.push(dot_draw(cx - dx, ay, size));
                draws.push(dot_draw(cx + dx, ay, size));
            }
            _ => {}
        }
        MathBox {
            width: w,
            height: pad + base_box.height,
            baseline: pad + base_box.baseline,
            draws,
        }
    }

    fn layout_text(&self, text: &str, size: f32) -> MathBox {
        if text.is_empty() {
            return MathBox {
                width: 0.0,
                height: size * 1.15,
                baseline: size * 0.82,
                draws: Vec::new(),
            };
        }
        let width = text
            .chars()
            .map(|ch| self.metrics.advance_em(ch, self.bold))
            .sum::<f32>()
            * size;
        let height = size * 1.15;
        let baseline = size * 0.82;
        MathBox {
            width,
            height,
            baseline,
            draws: vec![Draw::Text {
                x: 0.0,
                y: baseline,
                size,
                text: text.into(),
                bold: self.bold,
            }],
        }
    }
}

/// Advance width of `ch` as a fraction of the em, taken from the real glyph
/// metrics of **DejaVu Sans** — the face both the PDF and the SVG/PNG paths
/// use to draw math text. The layout engine reserves horizontal space using
/// these factors while the renderer advances the glyphs with the font's own
/// widths, so the two only stay in lockstep if the factors match the font.
/// Hand-rounded guesses (the old table) drifted by up to 0.28 em on `+`/`-`,
/// which a 90-term Lagrangian — built almost entirely of those signs —
/// accumulates into visible overlap. Values below are `advanceWidth /
/// unitsPerEm` straight from `DejaVuSans.ttf` (unitsPerEm = 2048).
// 0.318 (the comma/period/`·` advance) is close to 1/π, which trips the
// `approx_constant` lint — but these are real DejaVu advance widths, not the
// constant.
#[allow(clippy::approx_constant)]
fn char_width_factor(ch: char) -> f32 {
    // Combining diacritics (\bar, \hat, …) stack over the previous glyph and
    // therefore advance nothing.
    if ('\u{0300}'..='\u{036f}').contains(&ch) {
        return 0.0;
    }
    match ch {
        ' ' | '\t' | '\n' | '\r' => 0.32,
        '0'..='9' => 0.636,

        // ASCII lowercase
        'a' => 0.613,
        'b' => 0.635,
        'c' => 0.550,
        'd' => 0.635,
        'e' => 0.615,
        'f' => 0.352,
        'g' => 0.635,
        'h' => 0.634,
        'i' => 0.278,
        'j' => 0.278,
        'k' => 0.579,
        'l' => 0.278,
        'm' => 0.974,
        'n' => 0.634,
        'o' => 0.612,
        'p' => 0.635,
        'q' => 0.635,
        'r' => 0.411,
        's' => 0.521,
        't' => 0.392,
        'u' => 0.634,
        'v' => 0.592,
        'w' => 0.818,
        'x' => 0.592,
        'y' => 0.592,
        'z' => 0.525,

        // ASCII uppercase
        'A' => 0.684,
        'B' => 0.686,
        'C' => 0.698,
        'D' => 0.770,
        'E' => 0.632,
        'F' => 0.575,
        'G' => 0.775,
        'H' => 0.752,
        'I' => 0.295,
        'J' => 0.295,
        'K' => 0.656,
        'L' => 0.557,
        'M' => 0.863,
        'N' => 0.748,
        'O' => 0.787,
        'P' => 0.603,
        'Q' => 0.787,
        'R' => 0.695,
        'S' => 0.635,
        'T' => 0.611,
        'U' => 0.732,
        'V' => 0.684,
        'W' => 0.989,
        'X' => 0.685,
        'Y' => 0.611,
        'Z' => 0.685,

        // ASCII operators, relations and punctuation
        '+' | '=' | '<' | '>' | '~' | '#' => 0.838,
        '-' => 0.361,
        '*' => 0.500,
        '/' | '\\' | ':' | ';' | '|' => 0.337,
        '(' | ')' | '[' | ']' => 0.390,
        '{' | '}' => 0.636,
        ',' | '.' => 0.318,
        '!' => 0.401,
        '?' => 0.531,
        '\'' => 0.275,
        '"' => 0.460,
        '&' => 0.780,
        '%' => 0.950,
        '@' => 1.000,

        // Greek lowercase
        'α' => 0.659,
        'β' => 0.638,
        'γ' => 0.592,
        'δ' => 0.612,
        'ε' => 0.541,
        'ζ' => 0.544,
        'η' => 0.634,
        'θ' => 0.612,
        'ϑ' => 0.619,
        'ι' => 0.338,
        'κ' => 0.589,
        'λ' => 0.592,
        'μ' => 0.636,
        'ν' => 0.559,
        'ξ' => 0.558,
        'π' => 0.602,
        'ϖ' => 0.837,
        'ρ' => 0.635,
        'ϱ' => 0.635,
        'σ' => 0.634,
        'ς' => 0.587,
        'τ' => 0.602,
        'υ' => 0.579,
        'φ' => 0.660,
        'ϕ' => 0.660,
        'χ' => 0.578,
        'ψ' => 0.660,
        'ω' => 0.837,

        // Greek uppercase
        'Α' => 0.684,
        'Β' => 0.686,
        'Γ' => 0.557,
        'Δ' => 0.684,
        'Ε' => 0.632,
        'Ζ' => 0.685,
        'Η' => 0.752,
        'Θ' => 0.787,
        'Ι' => 0.295,
        'Κ' => 0.656,
        'Λ' => 0.684,
        'Μ' => 0.863,
        'Ν' => 0.748,
        'Ξ' => 0.632,
        'Π' => 0.752,
        'Ρ' => 0.603,
        'Σ' => 0.632,
        'Τ' => 0.611,
        'Υ' => 0.611,
        'Φ' => 0.787,
        'Χ' => 0.685,
        'Ψ' => 0.787,
        'Ω' => 0.764,

        // Big operators, relations, arrows and symbols
        '∑' => 0.674,
        '∏' => 0.757,
        '∫' | '∮' => 0.521,
        '⋃' | '⋂' => 0.820,
        '≤' | '≥' | '≠' | '≈' | '≡' | '∼' | '≃' | '≅' | '≐' | '≺' | '≻' | '≼' | '≽' => {
            0.838
        }
        '≪' | '≫' => 1.047,
        '∝' => 0.714,
        '→' | '←' | '⇒' | '⇐' | '↔' | '⇔' | '↦' | '↑' | '↓' | '⇑' | '⇓' => {
            0.838
        }
        '⟶' | '⟵' | '⟹' | '⟸' | '⟷' | '⟺' => 1.434,
        '±' | '∓' | '×' | '÷' | '⊕' | '⊗' | '∗' | '¬' => 0.838,
        '·' => 0.318,
        '∘' => 0.626,
        '•' => 0.590,
        '⋆' => 0.626,
        '∧' | '∨' | '∪' | '∩' => 0.732,
        '∖' => 0.637,
        '∀' => 0.684,
        '∃' | '∄' => 0.632,
        '∈' | '∉' | '∌' | '∅' => 0.871,
        '⊂' | '⊃' | '⊆' | '⊇' | '⊈' | '⊉' => 0.838,
        '∴' | '∵' => 0.636,
        '∞' => 0.833,
        '∂' => 0.517,
        '∇' => 0.669,
        'ℏ' => 0.634,
        'ℓ' => 0.413,
        'ℜ' => 0.814,
        'ℑ' => 0.697,
        'ℵ' => 0.745,
        '∠' => 0.896,
        '△' => 0.769,
        '°' => 0.500,
        '′' => 0.227,
        '†' => 0.500,
        '⟨' | '⟩' | '⌊' | '⌋' | '⌈' | '⌉' => 0.390,
        '‖' => 0.500,
        '…' | '⋯' | '⋮' | '⋱' => 1.000,

        // Anything else: blackboard/script letters, CJK, etc. A middling
        // advance is the safest guess and rarely appears in equations.
        _ => 0.620,
    }
}

impl<'m> LayoutCtx<'m> {
    fn layout_row(&self, nodes: &[MathNode], size: f32) -> MathBox {
        let children = nodes
            .iter()
            .map(|node| self.layout_math(node, size))
            .collect::<Vec<_>>();
        let baseline = children
            .iter()
            .map(|b| b.baseline)
            .fold(size * 0.82, f32::max);
        let below = children
            .iter()
            .map(|b| b.height - b.baseline)
            .fold(size * 0.33, f32::max);
        let mut draws = Vec::new();
        let mut x = 0.0;
        for child in children {
            append_draws(&mut draws, &child, x, baseline - child.baseline);
            x += child.width;
        }
        MathBox {
            width: x,
            height: baseline + below,
            baseline,
            draws,
        }
    }

    fn layout_fraction(&self, num: &MathNode, den: &MathNode, size: f32) -> MathBox {
        let child_size = (size * 0.82).max(12.0);
        let num_box = self.layout_math(num, child_size);
        let den_box = self.layout_math(den, child_size);
        let pad = size * 0.28;
        let gap = size * 0.16;
        let line_y = num_box.height + gap;
        let width = num_box.width.max(den_box.width) + pad * 2.0;
        let mut draws = Vec::new();
        append_draws(&mut draws, &num_box, (width - num_box.width) / 2.0, 0.0);
        draws.push(Draw::Line {
            x1: 0.0,
            y1: line_y,
            x2: width,
            y2: line_y,
            stroke_width: (size * 0.045).max(1.2),
        });
        let den_y = line_y + gap;
        append_draws(&mut draws, &den_box, (width - den_box.width) / 2.0, den_y);
        MathBox {
            width,
            height: den_y + den_box.height,
            baseline: line_y + gap + den_box.baseline,
            draws,
        }
    }

    fn layout_sqrt(&self, inner: &MathNode, size: f32) -> MathBox {
        let inner_box = self.layout_math(inner, size * 0.94);
        let left = size * 0.62;
        let top = size * 0.14;
        let pad = size * 0.18;
        let width = left + inner_box.width + pad;
        let height = inner_box.height + top + size * 0.10;
        let baseline = top + inner_box.baseline;
        let mut draws = Vec::new();
        let y_mid = top + inner_box.height * 0.58;
        let y_base = top + inner_box.height * 0.86;
        draws.push(Draw::Polyline {
            points: vec![
                (size * 0.08, y_mid),
                (size * 0.22, y_base),
                (size * 0.42, top + inner_box.height),
                (left * 0.92, top + size * 0.06),
                (width, top + size * 0.06),
            ],
            stroke_width: (size * 0.045).max(1.2),
        });
        append_draws(&mut draws, &inner_box, left, top);
        MathBox {
            width,
            height,
            baseline,
            draws,
        }
    }

    fn layout_delimited(&self, left: &str, body: &MathNode, right: &str, size: f32) -> MathBox {
        let body_box = self.layout_math(body, size);
        self.layout_delimited_box(left, body_box, right, size)
    }

    fn layout_delimited_box(
        &self,
        left: &str,
        body_box: MathBox,
        right: &str,
        size: f32,
    ) -> MathBox {
        let delimiter_height = body_box.height.max(size * 1.25).min(size * 6.0);
        let left_box = self.layout_delimiter(left, delimiter_height, size);
        let right_box = self.layout_delimiter(right, delimiter_height, size);
        let gap = if left.is_empty() && right.is_empty() {
            0.0
        } else {
            size * 0.12
        };
        let body_y = (delimiter_height - body_box.height) / 2.0;
        let baseline = body_y + body_box.baseline;
        let mut draws = Vec::new();
        let mut x = 0.0;
        if !left.is_empty() {
            append_draws(&mut draws, &left_box, x, 0.0);
            x += left_box.width + gap;
        }
        append_draws(&mut draws, &body_box, x, body_y);
        x += body_box.width;
        if !right.is_empty() {
            x += gap;
            append_draws(&mut draws, &right_box, x, 0.0);
            x += right_box.width;
        }
        MathBox {
            width: x,
            height: delimiter_height,
            baseline,
            draws,
        }
    }

    fn layout_delimiter(&self, token: &str, height: f32, size: f32) -> MathBox {
        if token.is_empty() {
            return MathBox {
                width: 0.0,
                height,
                baseline: height * 0.5,
                draws: Vec::new(),
            };
        }
        if scalable_delimiter(token) {
            let width = delimiter_width(token, height, size);
            return MathBox {
                width,
                height,
                baseline: height * 0.5,
                draws: vec![Draw::Delimiter {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height,
                    token: token.to_string(),
                    stroke_width: (size * 0.055).max(1.35),
                }],
            };
        }
        let font_size = height.min(size * 3.2).max(size * 1.15);
        self.layout_text(token, font_size)
    }
}

fn scalable_delimiter(token: &str) -> bool {
    matches!(
        token,
        "(" | ")" | "[" | "]" | "{" | "}" | "|" | "‖" | "⟨" | "⟩"
    )
}

fn delimiter_width(token: &str, height: f32, size: f32) -> f32 {
    match token {
        "|" => size * 0.18,
        "‖" => size * 0.34,
        "⟨" | "⟩" => (height * 0.18).clamp(size * 0.34, size * 0.72),
        _ => (height * 0.16).clamp(size * 0.32, size * 0.74),
    }
}

impl<'m> LayoutCtx<'m> {
    fn layout_script(
        &self,
        base: &MathNode,
        sub: Option<&MathNode>,
        sup: Option<&MathNode>,
        size: f32,
    ) -> MathBox {
        let base_box = self.layout_math(base, size);
        let script_size = (size * 0.62).max(10.0);
        let sup_box = sup.map(|node| self.layout_math(node, script_size));
        let sub_box = sub.map(|node| self.layout_math(node, script_size));
        let gap = size * 0.08;
        let above = sup_box
            .as_ref()
            .map(|b| (b.height * 0.72).max(size * 0.15))
            .unwrap_or(0.0);
        let below = sub_box
            .as_ref()
            .map(|b| (b.height * 0.72).max(size * 0.15))
            .unwrap_or(0.0);
        let baseline = above + base_box.baseline;
        let height = above + base_box.height + below;
        let script_width = sup_box
            .as_ref()
            .map(|b| b.width)
            .unwrap_or(0.0)
            .max(sub_box.as_ref().map(|b| b.width).unwrap_or(0.0));
        let mut draws = Vec::new();
        append_draws(&mut draws, &base_box, 0.0, above);
        if let Some(sup_box) = sup_box.as_ref() {
            append_draws(
                &mut draws,
                sup_box,
                base_box.width + gap,
                (above - sup_box.height * 0.70).max(0.0),
            );
        }
        if let Some(sub_box) = sub_box.as_ref() {
            append_draws(
                &mut draws,
                sub_box,
                base_box.width + gap,
                baseline + size * 0.10,
            );
        }
        MathBox {
            width: base_box.width + gap + script_width,
            height,
            baseline,
            draws,
        }
    }

    fn layout_lines(&self, lines: &[MathNode], size: f32) -> MathBox {
        let rows = lines
            .iter()
            .map(|line| self.layout_math(line, size))
            .collect::<Vec<_>>();
        stack_rows(&rows, size * 0.38, false)
    }

    fn layout_matrix(&self, rows: &[Vec<MathNode>], left: &str, right: &str, size: f32) -> MathBox {
        let cell_size = (size * 0.86).max(12.0);
        let laid_rows = rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| self.layout_math(cell, cell_size))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let cols = laid_rows.iter().map(|row| row.len()).max().unwrap_or(0);
        let mut col_widths = vec![0.0; cols];
        for row in &laid_rows {
            for (idx, cell) in row.iter().enumerate() {
                if cell.width > col_widths[idx] {
                    col_widths[idx] = cell.width;
                }
            }
        }
        let col_gap = size * 0.65;
        let row_gap = size * 0.28;
        let body_width = col_widths.iter().sum::<f32>() + col_gap * cols.saturating_sub(1) as f32;
        let mut y = 0.0;
        let mut draws = Vec::new();
        for row in &laid_rows {
            let baseline = row
                .iter()
                .map(|cell| cell.baseline)
                .fold(cell_size * 0.82, f32::max);
            let below = row
                .iter()
                .map(|cell| cell.height - cell.baseline)
                .fold(cell_size * 0.33, f32::max);
            let mut x = 0.0;
            for (idx, cell) in row.iter().enumerate() {
                let cell_x = x + (col_widths[idx] - cell.width) / 2.0;
                append_draws(&mut draws, cell, cell_x, y + baseline - cell.baseline);
                x += col_widths[idx] + col_gap;
            }
            y += baseline + below + row_gap;
        }
        let height = (y - row_gap).max(size * 1.2);
        let baseline = height / 2.0 + size * 0.28;
        let body = MathBox {
            width: body_width,
            height,
            baseline,
            draws,
        };
        if left.is_empty() && right.is_empty() {
            body
        } else {
            self.layout_delimited_box(left, body, right, size)
        }
    }

    fn layout_cases(&self, rows: &[(MathNode, Option<MathNode>)], size: f32) -> MathBox {
        let matrix_rows = rows
            .iter()
            .map(|(expr, condition)| {
                let mut row = vec![expr.clone()];
                if let Some(condition) = condition {
                    row.push(MathNode::Text("if ".into()));
                    row.push(condition.clone());
                }
                row
            })
            .collect::<Vec<_>>();
        self.layout_matrix(&matrix_rows, "{", "", size)
    }
}

fn stack_rows(rows: &[MathBox], gap: f32, center: bool) -> MathBox {
    let width = rows.iter().map(|row| row.width).fold(0.0, f32::max);
    let height =
        rows.iter().map(|row| row.height).sum::<f32>() + gap * rows.len().saturating_sub(1) as f32;
    let baseline = rows.first().map(|row| row.baseline).unwrap_or(height * 0.5);
    let mut draws = Vec::new();
    let mut y = 0.0;
    for row in rows {
        let x = if center {
            (width - row.width) / 2.0
        } else {
            0.0
        };
        append_draws(&mut draws, row, x, y);
        y += row.height + gap;
    }
    MathBox {
        width,
        height,
        baseline,
        draws,
    }
}

/// A dot accent: a near-zero-length stroke whose round line cap renders as a
/// filled disc of diameter ≈ the stroke width.
fn dot_draw(cx: f32, cy: f32, size: f32) -> Draw {
    let r = (size * 0.06).max(1.4);
    Draw::Line {
        x1: cx,
        y1: cy,
        x2: cx + 0.01,
        y2: cy,
        stroke_width: r * 2.0,
    }
}

fn append_draws(draws: &mut Vec<Draw>, child: &MathBox, dx: f32, dy: f32) {
    for draw in &child.draws {
        draws.push(offset_draw(draw, dx, dy));
    }
}

fn publish_math_box(layout: MathBox) -> MathTextLayout {
    MathTextLayout {
        width: layout.width,
        height: layout.height,
        baseline: layout.baseline,
        draws: layout.draws.into_iter().map(publish_draw).collect(),
    }
}

fn publish_draw(draw: Draw) -> MathLayoutDraw {
    match draw {
        Draw::Text {
            x,
            y,
            size,
            text,
            bold,
        } => MathLayoutDraw::Text {
            x,
            y,
            size,
            text,
            bold,
        },
        Draw::Line {
            x1,
            y1,
            x2,
            y2,
            stroke_width,
        } => MathLayoutDraw::Line {
            x1,
            y1,
            x2,
            y2,
            stroke_width,
        },
        Draw::Polyline {
            points,
            stroke_width,
        } => MathLayoutDraw::Polyline {
            points,
            stroke_width,
        },
        Draw::Delimiter {
            x,
            y,
            width,
            height,
            token,
            stroke_width,
        } => MathLayoutDraw::Delimiter {
            x,
            y,
            width,
            height,
            token,
            stroke_width,
        },
    }
}

fn offset_draw(draw: &Draw, dx: f32, dy: f32) -> Draw {
    match draw {
        Draw::Text {
            x,
            y,
            size,
            text,
            bold,
        } => Draw::Text {
            x: x + dx,
            y: y + dy,
            size: *size,
            text: text.clone(),
            bold: *bold,
        },
        Draw::Line {
            x1,
            y1,
            x2,
            y2,
            stroke_width,
        } => Draw::Line {
            x1: x1 + dx,
            y1: y1 + dy,
            x2: x2 + dx,
            y2: y2 + dy,
            stroke_width: *stroke_width,
        },
        Draw::Polyline {
            points,
            stroke_width,
        } => Draw::Polyline {
            points: points.iter().map(|(x, y)| (x + dx, y + dy)).collect(),
            stroke_width: *stroke_width,
        },
        Draw::Delimiter {
            x,
            y,
            width,
            height,
            token,
            stroke_width,
        } => Draw::Delimiter {
            x: x + dx,
            y: y + dy,
            width: *width,
            height: *height,
            token: token.clone(),
            stroke_width: *stroke_width,
        },
    }
}

fn render_box(layout: &MathBox, dx: f32, dy: f32, out: &mut String) {
    for draw in &layout.draws {
        match draw {
            Draw::Text {
                x,
                y,
                size,
                text,
                bold,
            } => {
                if text.is_empty() {
                    continue;
                }
                let weight = if *bold { r#" font-weight="bold""# } else { "" };
                out.push_str(&format!(
                    r#"    <text x="{:.1}" y="{:.1}" font-size="{:.1}"{weight} stroke="none">{}</text>"#,
                    x + dx,
                    y + dy,
                    size,
                    escape_xml(text)
                ));
                out.push('\n');
            }
            Draw::Line {
                x1,
                y1,
                x2,
                y2,
                stroke_width,
            } => {
                out.push_str(&format!(
                    r#"    <line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke-width="{:.1}"/>"#,
                    x1 + dx,
                    y1 + dy,
                    x2 + dx,
                    y2 + dy,
                    stroke_width
                ));
                out.push('\n');
            }
            Draw::Polyline {
                points,
                stroke_width,
            } => {
                let points = points
                    .iter()
                    .map(|(x, y)| format!("{:.1},{:.1}", x + dx, y + dy))
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push_str(&format!(
                    r#"    <polyline points="{points}" fill="none" stroke-width="{:.1}"/>"#,
                    stroke_width
                ));
                out.push('\n');
            }
            Draw::Delimiter {
                x,
                y,
                width,
                height,
                token,
                stroke_width,
            } => {
                render_delimiter(*x + dx, *y + dy, *width, *height, token, *stroke_width, out);
            }
        }
    }
}

fn render_delimiter(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    token: &str,
    stroke_width: f32,
    out: &mut String,
) {
    let d = match token {
        "(" => format!(
            "M {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
            x + width * 0.86,
            y,
            x + width * 0.10,
            y + height * 0.20,
            x + width * 0.10,
            y + height * 0.80,
            x + width * 0.86,
            y + height
        ),
        ")" => format!(
            "M {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
            x + width * 0.14,
            y,
            x + width * 0.90,
            y + height * 0.20,
            x + width * 0.90,
            y + height * 0.80,
            x + width * 0.14,
            y + height
        ),
        "[" => format!(
            "M {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1}",
            x + width,
            y,
            x,
            y,
            x,
            y + height,
            x + width,
            y + height
        ),
        "]" => format!(
            "M {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1}",
            x,
            y,
            x + width,
            y,
            x + width,
            y + height,
            x,
            y + height
        ),
        "{" => format!(
            "M {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
            x + width,
            y,
            x + width * 0.18,
            y,
            x + width * 0.20,
            y + height * 0.28,
            x + width * 0.56,
            y + height * 0.38,
            x + width * 0.82,
            y + height * 0.46,
            x + width * 0.18,
            y + height * 0.45,
            x + width * 0.18,
            y + height * 0.50,
            x + width * 0.18,
            y + height * 0.55,
            x + width * 0.82,
            y + height * 0.54,
            x + width * 0.56,
            y + height * 0.62,
            x + width * 0.20,
            y + height * 0.72,
            x + width * 0.18,
            y + height,
            x + width,
            y + height
        ),
        "}" => format!(
            "M {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
            x,
            y,
            x + width * 0.82,
            y,
            x + width * 0.80,
            y + height * 0.28,
            x + width * 0.44,
            y + height * 0.38,
            x + width * 0.18,
            y + height * 0.46,
            x + width * 0.82,
            y + height * 0.45,
            x + width * 0.82,
            y + height * 0.50,
            x + width * 0.82,
            y + height * 0.55,
            x + width * 0.18,
            y + height * 0.54,
            x + width * 0.44,
            y + height * 0.62,
            x + width * 0.80,
            y + height * 0.72,
            x + width * 0.82,
            y + height,
            x,
            y + height
        ),
        "|" => format!(
            "M {:.1} {:.1} L {:.1} {:.1}",
            x + width * 0.5,
            y,
            x + width * 0.5,
            y + height
        ),
        "‖" => format!(
            "M {:.1} {:.1} L {:.1} {:.1} M {:.1} {:.1} L {:.1} {:.1}",
            x + width * 0.35,
            y,
            x + width * 0.35,
            y + height,
            x + width * 0.65,
            y,
            x + width * 0.65,
            y + height
        ),
        "⟨" => format!(
            "M {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1}",
            x + width,
            y,
            x,
            y + height * 0.5,
            x + width,
            y + height
        ),
        "⟩" => format!(
            "M {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1}",
            x,
            y,
            x + width,
            y + height * 0.5,
            x,
            y + height
        ),
        _ => return,
    };
    out.push_str(&format!(
        r#"    <path d="{d}" fill="none" stroke-width="{stroke_width:.1}"/>"#
    ));
    out.push('\n');
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    let mut seen_padding = false;
    for b in input.bytes().filter(|b| !b.is_ascii_whitespace()) {
        if b == b'=' {
            seen_padding = true;
            continue;
        }
        if seen_padding {
            return None;
        }
        let value = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        buf = (buf << 6) | value;
        bits += 6;
        while bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
            if bits == 0 {
                buf = 0;
            } else {
                buf &= (1 << bits) - 1;
            }
        }
    }
    if bits == 6 {
        return None;
    }
    Some(out)
}

fn diagnose_line(line: &str, line_no: usize, diagnostics: &mut Vec<MathDiagnostic>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let mut n = 0;
            while i + n < bytes.len() && bytes[i + n] == b'`' {
                n += 1;
            }
            if let Some(close) = find_code_span_close(bytes, i + n, n) {
                i = close + n;
                continue;
            }
            i += n;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'$' {
                if let Some(end) = find_close(bytes, i + 2, b"$$") {
                    diagnose_span(&line[i + 2..end], line_no, "display", diagnostics);
                    i = end + 2;
                    continue;
                }
                diagnostics.push(MathDiagnostic {
                    line: line_no,
                    kind: "unterminated-display-math",
                    detail: "display math starts with `$$` but has no closing `$$` on this line"
                        .to_string(),
                    source: excerpt(&line[i..]),
                });
                break;
            } else if let Some(end) = find_close(bytes, i + 1, b"$") {
                diagnose_span(&line[i + 1..end], line_no, "inline", diagnostics);
                i = end + 1;
                continue;
            } else {
                diagnostics.push(MathDiagnostic {
                    line: line_no,
                    kind: "unterminated-inline-math",
                    detail: "inline math starts with `$` but has no closing `$` on this line"
                        .to_string(),
                    source: excerpt(&line[i..]),
                });
                break;
            }
        }
        i = utf8_char_end(bytes, i);
    }
}

fn diagnose_span(
    src: &str,
    line_no: usize,
    math_kind: &str,
    diagnostics: &mut Vec<MathDiagnostic>,
) {
    let mut seen_macros: Vec<String> = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i = utf8_char_end(bytes, i);
            continue;
        }
        if i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
            diagnostics.push(MathDiagnostic {
                line: line_no,
                kind: "unsupported-math-linebreak",
                detail: format!("{math_kind} math uses `\\\\`; aligned or multi-line equations need a rich math renderer"),
                source: excerpt(src),
            });
            i += 2;
            continue;
        }
        if i + 1 < bytes.len() && matches!(bytes[i + 1], b',' | b':' | b';' | b'!') {
            i += 2;
            continue;
        }
        if i + 1 >= bytes.len() || !(bytes[i + 1] as char).is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && (bytes[j] as char).is_ascii_alphabetic() {
            j += 1;
        }
        let name = &src[i + 1..j];
        if name == "begin" {
            if let Some(env) = read_command_group_name(bytes, j) {
                if unsupported_environment(&env) {
                    diagnostics.push(MathDiagnostic {
                        line: line_no,
                        kind: "unsupported-math-environment",
                        detail: format!("`{env}` environments need a rich math renderer"),
                        source: excerpt(src),
                    });
                }
            }
        } else if !supported_macro(name) && !seen_macros.iter().any(|m| m == name) {
            seen_macros.push(name.to_string());
            diagnostics.push(MathDiagnostic {
                line: line_no,
                kind: "unsupported-math-macro",
                detail: format!("`\\{name}` is not translated by `--math unicode`"),
                source: excerpt(src),
            });
        }
        i = j;
    }
}

/// Find the start of a closing backtick run of exactly `n` backticks. Returns
/// the byte index of the first backtick of the closing run, or `None` if
/// there is no matching close on this line.
fn find_code_span_close(bytes: &[u8], from: usize, n: usize) -> Option<usize> {
    let mut j = from;
    while j < bytes.len() {
        if bytes[j] != b'`' {
            j += 1;
            continue;
        }
        let mut run = 0;
        while j + run < bytes.len() && bytes[j + run] == b'`' {
            run += 1;
        }
        if run == n {
            return Some(j);
        }
        j += run;
    }
    None
}

fn utf8_char_end(bytes: &[u8], start: usize) -> usize {
    let b = bytes[start];
    let len = if b < 0x80 {
        1
    } else if b < 0xC0 {
        1 // continuation byte — shouldn't be a start, but be defensive
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    };
    (start + len).min(bytes.len())
}

fn find_close(bytes: &[u8], from: usize, marker: &[u8]) -> Option<usize> {
    let n = marker.len();
    let mut j = from;
    while j + n <= bytes.len() {
        if &bytes[j..j + n] == marker {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn render_math(src: &str, display: bool, macros: &[(String, String)]) -> String {
    let expanded = apply_user_macros(src, macros);
    let body = translate_math(&expanded);
    if display {
        // Display math goes on its own line, italicised by the renderer via
        // surrounding underscores (markdown italics).
        format!("\n\n*{}*\n\n", body)
    } else {
        format!("*{}*", body)
    }
}

fn apply_user_macros(src: &str, macros: &[(String, String)]) -> String {
    if macros.is_empty() {
        return src.to_string();
    }
    let mut ordered = macros.to_vec();
    ordered.sort_by(|(left, _), (right, _)| {
        right.len().cmp(&left.len()).then_with(|| left.cmp(right))
    });
    let mut out = src.to_string();
    for (from, to) in ordered {
        if !from.is_empty() {
            out = out.replace(&from, &to);
        }
    }
    out
}

fn translate_math(src: &str) -> String {
    // Order matters here. We need `^{...}` and `_{...}` to keep their
    // braces long enough for `apply_super_sub` to read the full group;
    // otherwise stripping `{` and `}` early collapses `x^{n+1}` to `x^n+1`
    // and the super-sub pass would only catch the first character.
    let normalized = normalize_math_source(src.trim());
    let with_environments = expand_simple_environments(&normalized);
    let with_groups = expand_group_commands(&with_environments);
    let with_macros = expand_macros(&with_groups);
    let with_frac = expand_frac(&with_macros);
    let with_binom = expand_binom(&with_frac);
    let with_sqrt = expand_sqrt(&with_binom);
    let with_super = apply_super_sub(&with_sqrt);
    // Anything still wrapped in braces was either malformed or a group we
    // don't recognise; flatten it now so the user doesn't see literal `{}`.
    with_super.replace('{', "").replace('}', "")
}

fn normalize_math_source(src: &str) -> String {
    src.replace("\\left.", "")
        .replace("\\right.", "")
        .replace("\\limits", "")
        .replace("\\nolimits", "")
}

fn expand_simple_environments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if s[i..].starts_with("\\begin") {
            let j = i + "\\begin".len();
            if let Some((after_name, name)) = read_math_group_arg(bytes, j) {
                let marker = format!("\\end{{{name}}}");
                if let Some(rel_end) = s[after_name..].find(&marker) {
                    let inner_start = after_name;
                    let inner_end = after_name + rel_end;
                    out.push_str(&render_simple_environment(
                        &name,
                        &s[inner_start..inner_end],
                    ));
                    i = inner_end + marker.len();
                    continue;
                }
            }
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn render_simple_environment(name: &str, inner: &str) -> String {
    match name {
        "matrix" | "pmatrix" | "bmatrix" | "Bmatrix" | "vmatrix" | "Vmatrix" | "smallmatrix" => {
            render_matrix_environment(name, inner)
        }
        "cases" => render_cases_environment(inner),
        "aligned" | "align" | "alignat" | "gather" | "equation" | "split" => {
            render_aligned_environment(inner)
        }
        _ => inner.to_string(),
    }
}

fn render_matrix_environment(name: &str, inner: &str) -> String {
    let (open, close) = match name {
        "pmatrix" => ("(", ")"),
        "bmatrix" => ("[", "]"),
        "Bmatrix" => ("{", "}"),
        "vmatrix" => ("|", "|"),
        "Vmatrix" => ("‖", "‖"),
        _ => ("", ""),
    };
    let rows = split_math_rows(inner)
        .into_iter()
        .map(|row| {
            row.split('&')
                .map(|cell| translate_math(cell.trim()))
                .collect::<Vec<_>>()
                .join("  ")
        })
        .filter(|row| !row.trim().is_empty())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return String::new();
    }
    let body = rows.join("\n");
    if open.is_empty() {
        body
    } else {
        format!("{open} {body} {close}")
    }
}

fn render_cases_environment(inner: &str) -> String {
    split_math_rows(inner)
        .into_iter()
        .map(|row| {
            let cells = row
                .split('&')
                .map(|cell| translate_math(cell.trim()))
                .filter(|cell| !cell.trim().is_empty())
                .collect::<Vec<_>>();
            match cells.as_slice() {
                [] => String::new(),
                [expr] => expr.clone(),
                [expr, condition, ..] => format!("{expr}  if {condition}"),
            }
        })
        .filter(|row| !row.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_aligned_environment(inner: &str) -> String {
    split_math_rows(inner)
        .into_iter()
        .map(|row| {
            row.split('&')
                .map(|cell| translate_math(cell.trim()))
                .filter(|cell| !cell.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|row| !row.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn split_math_rows(inner: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && &bytes[i..i + 2] == b"\\\\" {
            rows.push(current.trim().to_string());
            current.clear();
            i += 2;
            continue;
        }
        if bytes[i] == b'\n' {
            if !current.ends_with(' ') {
                current.push(' ');
            }
            i += 1;
            continue;
        }
        let end = utf8_char_end(bytes, i);
        current.push_str(&inner[i..end]);
        i = end;
    }
    rows.push(current.trim().to_string());
    rows
}

fn expand_group_commands(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_alphabetic()
        {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] as char).is_ascii_alphabetic() {
                j += 1;
            }
            let name = &s[i + 1..j];
            let arg_start = if name == "operatorname" && j < bytes.len() && bytes[j] == b'*' {
                j + 1
            } else {
                j
            };
            if let Some((end, arg)) = read_math_group_arg(bytes, arg_start) {
                if is_text_like_group_command(name) {
                    out.push_str(&arg);
                    i = end;
                    continue;
                }
                if name == "mathbb" {
                    out.push_str(&map_math_alphabet(&arg, blackboard_char));
                    i = end;
                    continue;
                }
                if name == "mathcal" {
                    out.push_str(&map_math_alphabet(&arg, script_char));
                    i = end;
                    continue;
                }
                if let Some(mark) = accent_mark(name) {
                    out.push_str(&apply_combining_mark(&translate_math(&arg), mark));
                    i = end;
                    continue;
                }
            }
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn read_math_group_arg(bytes: &[u8], from: usize) -> Option<(usize, String)> {
    let mut j = from;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    if j >= bytes.len() || bytes[j] != b'{' {
        return None;
    }
    let (end, inner) = read_group(bytes, j);
    end.map(|idx| (idx + 1, inner))
}

fn is_text_like_group_command(name: &str) -> bool {
    matches!(
        name,
        "text"
            | "textrm"
            | "textit"
            | "textbf"
            | "mathrm"
            | "mathbf"
            | "mathit"
            | "mathsf"
            | "mathtt"
            | "operatorname"
            | "boldsymbol"
            | "bm"
            | "pmb"
            | "mathbfit"
    )
}

fn accent_mark(name: &str) -> Option<char> {
    match name {
        "vec" => Some('\u{20d7}'),
        "bar" => Some('\u{0304}'),
        "overline" => Some('\u{0305}'),
        "hat" => Some('\u{0302}'),
        "tilde" => Some('\u{0303}'),
        "dot" => Some('\u{0307}'),
        "ddot" => Some('\u{0308}'),
        _ => None,
    }
}

fn apply_combining_mark(arg: &str, mark: char) -> String {
    let mut out = String::with_capacity(arg.len() + 4);
    for ch in arg.chars() {
        out.push(ch);
        if !ch.is_whitespace() {
            out.push(mark);
        }
    }
    out
}

fn map_math_alphabet(arg: &str, map: fn(char) -> Option<char>) -> String {
    arg.chars().map(|ch| map(ch).unwrap_or(ch)).collect()
}

fn blackboard_char(ch: char) -> Option<char> {
    Some(match ch {
        'A' => '𝔸',
        'B' => '𝔹',
        'C' => 'ℂ',
        'D' => '𝔻',
        'E' => '𝔼',
        'F' => '𝔽',
        'G' => '𝔾',
        'H' => 'ℍ',
        'I' => '𝕀',
        'J' => '𝕁',
        'K' => '𝕂',
        'L' => '𝕃',
        'M' => '𝕄',
        'N' => 'ℕ',
        'O' => '𝕆',
        'P' => 'ℙ',
        'Q' => 'ℚ',
        'R' => 'ℝ',
        'S' => '𝕊',
        'T' => '𝕋',
        'U' => '𝕌',
        'V' => '𝕍',
        'W' => '𝕎',
        'X' => '𝕏',
        'Y' => '𝕐',
        'Z' => 'ℤ',
        _ => return None,
    })
}

fn script_char(ch: char) -> Option<char> {
    Some(match ch {
        'A' => '𝒜',
        'B' => 'ℬ',
        'C' => '𝒞',
        'D' => '𝒟',
        'E' => 'ℰ',
        'F' => 'ℱ',
        'G' => '𝒢',
        'H' => 'ℋ',
        'I' => 'ℐ',
        'J' => '𝒥',
        'K' => '𝒦',
        'L' => 'ℒ',
        'M' => 'ℳ',
        'N' => '𝒩',
        'O' => '𝒪',
        'P' => '𝒫',
        'Q' => '𝒬',
        'R' => 'ℛ',
        'S' => '𝒮',
        'T' => '𝒯',
        'U' => '𝒰',
        'V' => '𝒱',
        'W' => '𝒲',
        'X' => '𝒳',
        'Y' => '𝒴',
        'Z' => '𝒵',
        _ => return None,
    })
}

/// Replace `\alpha`-style commands with their Unicode equivalents.
fn expand_macros(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Single-character spacing macros: `\,` (thin), `\:` (medium),
        // `\;` (thick), `\!` (negative thin). LaTeX uses these for
        // fine-grained spacing; we just collapse them to a plain space
        // (or nothing for the negative one) since our renderer has no
        // typographic spacing primitives.
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            let replacement: Option<&str> = match next {
                b',' | b':' | b';' => Some(" "),
                b'!' => Some(""),
                b'|' => Some("‖"),
                b'{' => Some("{"),
                b'}' => Some("}"),
                b'(' => Some("("),
                b')' => Some(")"),
                b'[' => Some("["),
                b']' => Some("]"),
                b'_' => Some("_"),
                b' ' => Some(" "),
                _ => None,
            };
            if let Some(rep) = replacement {
                out.push_str(rep);
                i += 2;
                continue;
            }
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_alphabetic()
        {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] as char).is_ascii_alphabetic() {
                j += 1;
            }
            let name = &s[i + 1..j];
            if let Some(rep) = greek_or_symbol(name) {
                out.push_str(rep);
                i = j;
                // LaTeX swallows a single trailing space after a macro so
                // `\alpha k` reads as `αk`. That's the right behaviour for
                // single-glyph replacements (Greek letters, symbols), but
                // operator names like `\log n` need to keep the space:
                // "log n" is the desired reading, not "logn". Detect the
                // word-shaped case by the last char of the replacement.
                let rep_ends_in_alpha = rep
                    .chars()
                    .last()
                    .map_or(false, |c| c.is_ascii_alphabetic());
                if !rep_ends_in_alpha && i < bytes.len() && bytes[i] == b' ' {
                    i += 1;
                }
                continue;
            }
            out.push_str(&s[i..j]);
            i = j;
            continue;
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn expand_frac(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 5 <= bytes.len() && &bytes[i..i + 5] == b"\\frac" && bytes[i] == b'\\' {
            let mut j = i + 5;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'{' {
                if let (Some(end1), num_text) = read_group(bytes, j) {
                    let mut k = end1 + 1;
                    while k < bytes.len() && bytes[k] == b' ' {
                        k += 1;
                    }
                    if k < bytes.len() && bytes[k] == b'{' {
                        if let (Some(end2), den_text) = read_group(bytes, k) {
                            out.push('(');
                            out.push_str(&num_text);
                            out.push_str(")/(");
                            out.push_str(&den_text);
                            out.push(')');
                            i = end2 + 1;
                            continue;
                        }
                    }
                }
            }
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

/// `\binom{n}{k}` → `C(n, k)`. Real LaTeX renders it as a column of two
/// values inside parentheses; the closest unambiguous ASCII analogue is the
/// `C(n, k)` (combinations) shorthand used in combinatorics textbooks.
fn expand_binom(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 6 <= bytes.len() && &bytes[i..i + 6] == b"\\binom" && bytes[i] == b'\\' {
            let mut j = i + 6;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'{' {
                if let (Some(end1), top) = read_group(bytes, j) {
                    let mut k = end1 + 1;
                    while k < bytes.len() && bytes[k] == b' ' {
                        k += 1;
                    }
                    if k < bytes.len() && bytes[k] == b'{' {
                        if let (Some(end2), bot) = read_group(bytes, k) {
                            out.push_str("C(");
                            out.push_str(&top);
                            out.push_str(", ");
                            out.push_str(&bot);
                            out.push(')');
                            i = end2 + 1;
                            continue;
                        }
                    }
                }
            }
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn expand_sqrt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 5 <= bytes.len() && &bytes[i..i + 5] == b"\\sqrt" {
            let mut j = i + 5;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'{' {
                if let (Some(end), inner) = read_group(bytes, j) {
                    out.push('√');
                    out.push('(');
                    out.push_str(&inner);
                    out.push(')');
                    i = end + 1;
                    continue;
                }
            }
            out.push('√');
            i = j;
            continue;
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn read_group(bytes: &[u8], open: usize) -> (Option<usize>, String) {
    debug_assert_eq!(bytes[open], b'{');
    let mut depth = 0;
    let mut j = open;
    while j < bytes.len() {
        match bytes[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let inner = std::str::from_utf8(&bytes[open + 1..j])
                        .unwrap_or("")
                        .to_string();
                    return (Some(j), inner);
                }
            }
            _ => {}
        }
        j += 1;
    }
    (None, String::new())
}

fn read_command_group_name(bytes: &[u8], open: usize) -> Option<String> {
    let mut j = open;
    while j < bytes.len() && bytes[j] == b' ' {
        j += 1;
    }
    if j >= bytes.len() || bytes[j] != b'{' {
        return None;
    }
    let (end, inner) = read_group(bytes, j);
    end.map(|_| inner)
}

fn unsupported_environment(name: &str) -> bool {
    matches!(
        name,
        "matrix"
            | "pmatrix"
            | "bmatrix"
            | "Bmatrix"
            | "vmatrix"
            | "Vmatrix"
            | "smallmatrix"
            | "array"
            | "cases"
            | "aligned"
            | "align"
            | "alignat"
            | "gather"
            | "equation"
            | "split"
    )
}

fn supported_macro(name: &str) -> bool {
    matches!(
        name,
        "frac"
            | "dfrac"
            | "tfrac"
            | "cfrac"
            | "sqrt"
            | "binom"
            | "begin"
            | "end"
            | "limits"
            | "nolimits"
            | "mathbb"
            | "mathcal"
            | "text"
            | "textrm"
            | "textit"
            | "textbf"
            | "mathrm"
            | "mathbf"
            | "mathit"
            | "mathsf"
            | "mathtt"
            | "operatorname"
            | "boldsymbol"
            | "bm"
            | "pmb"
            | "mathbfit"
            | "vec"
            | "bar"
            | "overline"
            | "hat"
            | "tilde"
            | "dot"
            | "ddot"
            | "dagger"
    ) || greek_or_symbol(name).is_some()
}

fn excerpt(src: &str) -> String {
    let mut out: String = src.chars().take(96).collect();
    if src.chars().count() > 96 {
        out.push('…');
    }
    out
}

/// Convert `x^2`, `x^{2n}`, `a_i`, `a_{ij}` to Unicode super/subscript when
/// every character in the exponent maps cleanly. Falls back to `x^(2n)`
/// notation otherwise.
fn apply_super_sub(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c < (0x80 as char) && (c == '^' || c == '_') && !out.is_empty() {
            let map = if c == '^' {
                superscript_char
            } else {
                subscript_char
            };
            let (end, group) = if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                read_group(bytes, i + 1)
            } else if i + 1 < bytes.len() {
                let end = utf8_char_end(bytes, i + 1);
                (
                    Some(end.saturating_sub(1)),
                    std::str::from_utf8(&bytes[i + 1..end])
                        .unwrap_or("")
                        .to_string(),
                )
            } else {
                (None, String::new())
            };
            if let Some(end_idx) = end {
                let mapped: Option<String> = group.chars().map(map).collect::<Option<String>>();
                if let Some(text) = mapped {
                    out.push_str(&text);
                } else {
                    out.push(c);
                    if group.chars().count() > 1 {
                        out.push('(');
                        out.push_str(&group);
                        out.push(')');
                    } else {
                        out.push_str(&group);
                    }
                }
                i = end_idx + 1;
                continue;
            }
        }
        let ch_end = utf8_char_end(bytes, i);
        out.push_str(&s[i..ch_end]);
        i = ch_end;
    }
    out
}

fn superscript_char(c: char) -> Option<String> {
    let m = match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        ' ' => ' ',
        // Lowercase Latin (Unicode "Modifier Letter Small …" range).
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        // Uppercase Latin where Unicode has a modifier-letter form. Missing
        // glyphs: C F Q S X Y Z (no equivalent codepoint exists, so callers
        // fall back to literal `^X` notation for those).
        'A' => 'ᴬ',
        'B' => 'ᴮ',
        'D' => 'ᴰ',
        'E' => 'ᴱ',
        'G' => 'ᴳ',
        'H' => 'ᴴ',
        'I' => 'ᴵ',
        'J' => 'ᴶ',
        'K' => 'ᴷ',
        'L' => 'ᴸ',
        'M' => 'ᴹ',
        'N' => 'ᴺ',
        'O' => 'ᴼ',
        'P' => 'ᴾ',
        'R' => 'ᴿ',
        'T' => 'ᵀ',
        'U' => 'ᵁ',
        'V' => 'ⱽ',
        'W' => 'ᵂ',
        _ => return None,
    };
    Some(m.to_string())
}

fn subscript_char(c: char) -> Option<String> {
    let m = match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => return None,
    };
    Some(m.to_string())
}

fn greek_or_symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        // Greek lowercase
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ε",
        "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "varpi" => "ϖ",
        "rho" => "ρ",
        "varrho" => "ϱ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "φ",
        "varphi" => "ϕ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        // Greek uppercase
        "Alpha" => "Α",
        "Beta" => "Β",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Epsilon" => "Ε",
        "Zeta" => "Ζ",
        "Eta" => "Η",
        "Theta" => "Θ",
        "Iota" => "Ι",
        "Kappa" => "Κ",
        "Lambda" => "Λ",
        "Mu" => "Μ",
        "Nu" => "Ν",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Rho" => "Ρ",
        "Sigma" => "Σ",
        "Tau" => "Τ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Chi" => "Χ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        // Large operators
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "oint" => "∮",
        "bigcup" => "⋃",
        "bigcap" => "⋂",
        // Relations
        "leq" => "≤",
        "le" => "≤",
        "geq" => "≥",
        "ge" => "≥",
        "neq" => "≠",
        "ne" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "simeq" => "≃",
        "cong" => "≅",
        "doteq" => "≐",
        "ll" => "≪",
        "gg" => "≫",
        "prec" => "≺",
        "succ" => "≻",
        "preceq" => "≼",
        "succeq" => "≽",
        "propto" => "∝",
        // Arrows
        "to" => "→",
        "rightarrow" => "→",
        "leftarrow" => "←",
        "longrightarrow" => "⟶",
        "longleftarrow" => "⟵",
        "Rightarrow" => "⇒",
        "Leftarrow" => "⇐",
        "Longrightarrow" => "⟹",
        "Longleftarrow" => "⟸",
        "leftrightarrow" => "↔",
        "Leftrightarrow" => "⇔",
        "longleftrightarrow" => "⟷",
        "Longleftrightarrow" => "⟺",
        "mapsto" => "↦",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "Uparrow" => "⇑",
        "Downarrow" => "⇓",
        // Operators
        "pm" => "±",
        "mp" => "∓",
        "times" => "×",
        "cdot" => "·",
        "circ" => "∘",
        "bullet" => "•",
        "oplus" => "⊕",
        "otimes" => "⊗",
        "wedge" => "∧",
        "vee" => "∨",
        "div" => "÷",
        "cup" => "∪",
        "cap" => "∩",
        "setminus" => "∖",
        "ast" => "∗",
        "star" => "⋆",
        // Logic / set
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "in" => "∈",
        "notin" => "∉",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "nsubseteq" => "⊈",
        "nsupseteq" => "⊉",
        "includenot" => "∌",
        "land" => "∧",
        "lor" => "∨",
        "lnot" => "¬",
        "neg" => "¬",
        "therefore" => "∴",
        "because" => "∵",
        // Misc
        "infty" => "∞",
        "emptyset" => "∅",
        "varnothing" => "∅",
        "partial" => "∂",
        "nabla" => "∇",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",
        "angle" => "∠",
        "triangle" => "△",
        "degree" => "°",
        "prime" => "′",
        "dagger" => "†",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lvert" => "|",
        "rvert" => "|",
        "mid" => "|",
        "vert" => "|",
        "Vert" => "‖",
        "lVert" => "‖",
        "rVert" => "‖",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "ldots" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "ddots" => "⋱",
        // Delimiter sizing macros — real LaTeX auto-scales the following
        // delimiter to match the height of its content. We have no stacking
        // engine, so we just collapse them to empty and let the explicit
        // delimiter character that follows render normally.
        "left" => "",
        "right" => "",
        "big" => "",
        "Big" => "",
        "bigg" => "",
        "Bigg" => "",
        // Spacing macros. Real LaTeX has fine-grained spacing primitives;
        // map them to plain ASCII spaces of roughly the same visual width.
        "quad" => "  ",
        "qquad" => "    ",
        // Operator names. In real LaTeX these are typeset in roman; we just
        // emit the literal text so `\log n` reads as `log n`. Without these
        // entries the backslash + name would leak into the output verbatim.
        "log" => "log",
        "ln" => "ln",
        "lg" => "lg",
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "csc" => "csc",
        "sec" => "sec",
        "cot" => "cot",
        "arcsin" => "arcsin",
        "arccos" => "arccos",
        "arctan" => "arctan",
        "sinh" => "sinh",
        "cosh" => "cosh",
        "tanh" => "tanh",
        "exp" => "exp",
        "min" => "min",
        "max" => "max",
        "sup" => "sup",
        "inf" => "inf",
        "lim" => "lim",
        "liminf" => "lim inf",
        "limsup" => "lim sup",
        "argmax" => "arg max",
        "argmin" => "arg min",
        "deg" => "deg",
        "det" => "det",
        "dim" => "dim",
        "gcd" => "gcd",
        "hom" => "hom",
        "ker" => "ker",
        "mod" => "mod",
        "rank" => "rank",
        "trace" => "trace",
        "tr" => "tr",
        "span" => "span",
        "Pr" => "Pr",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_layout_draws_fraction_and_sqrt_geometry() {
        let svg = render_math_svg(r"\frac{a+1}{\sqrt{x}}", &[], MathSvgOptions::default());

        assert!(svg.contains("<line "), "{svg}");
        assert!(svg.contains("<polyline "), "{svg}");
        assert!(svg.contains(">a+1</text>"), "{svg}");
        assert!(svg.contains(">x</text>"), "{svg}");
    }

    #[test]
    fn svg_layout_aligns_matrix_cells() {
        let svg = render_math_svg(
            r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
            &[],
            MathSvgOptions::default(),
        );

        assert!(svg.contains(">a</text>"), "{svg}");
        assert!(svg.contains(">b</text>"), "{svg}");
        assert!(svg.contains(">c</text>"), "{svg}");
        assert!(svg.contains(">d</text>"), "{svg}");
        assert!(svg.contains("<path "), "{svg}");
    }

    #[test]
    fn math_options_apply_front_matter_style_macros() {
        let options = MathOptions {
            mode: MathMode::Unicode,
            macros: vec![(r"\RR".into(), r"\mathbb{R}".into())],
            svg: MathSvgOptions::default(),
        };
        let rendered = translate_with_options(r"$x \in \RR^n$", &options);

        assert!(rendered.contains("ℝⁿ"), "{rendered}");
    }

    #[test]
    fn markup_scripts_bind_to_last_plain_atom() {
        let layout = layout_markup_text("MZ^0_\\mu", 100);
        let texts = layout
            .draws
            .iter()
            .filter_map(|draw| match draw {
                MathLayoutDraw::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(texts.contains(&"M"), "{texts:?}");
        assert!(texts.contains(&"Z"), "{texts:?}");
        assert!(!texts.contains(&"MZ"), "{texts:?}");
    }

    #[test]
    fn markup_width_model_leaves_room_for_wide_fields() {
        let wide = layout_markup_text(r"W^+_\mu W^{-\mu} MZ^0_\mu", 100);
        let narrow = layout_markup_text(r"i^+_\mu i^{-\mu} lZ^0_\mu", 100);

        assert!(
            wide.width > narrow.width * 1.35,
            "wide glyphs should reserve more horizontal layout room: wide={}, narrow={}",
            wide.width,
            narrow.width
        );
    }

    #[test]
    fn layout_reserves_width_from_the_supplied_metrics() {
        // A provider reporting a fixed advance per glyph must drive the
        // reserved width directly — proving the layout measures the active
        // face rather than a baked-in table. This is what keeps PDF `--font`
        // output aligned.
        struct Fixed(f32);
        impl GlyphMetrics for Fixed {
            fn advance_em(&self, _ch: char, _bold: bool) -> f32 {
                self.0
            }
        }

        let wide = layout_markup_text_with("abcd", 100, &Fixed(1.0));
        let narrow = layout_markup_text_with("abcd", 100, &Fixed(0.5));

        // 4 glyphs × 1.0 em × 28 px base size.
        assert!((wide.width - 112.0).abs() < 0.5, "wide={}", wide.width);
        assert!(
            (wide.width - narrow.width * 2.0).abs() < 0.5,
            "halving the advance must halve the width: wide={}, narrow={}",
            wide.width,
            narrow.width
        );
    }

    #[test]
    fn accents_render_as_centred_geometry_not_combining_glyphs() {
        // `\bar{X}` must draw a horizontal rule over the base and keep the
        // base text as a bare "X" — never an "X\u{0304}" combining run, whose
        // zero-advance mark the renderer would mis-place at the base's edge
        // (font-dependently). This is what makes the accent font-independent.
        let layout = layout_markup_text(r"\bar{X}", 100);
        let has_rule = layout
            .draws
            .iter()
            .any(|d| matches!(d, MathLayoutDraw::Line { .. }));
        assert!(has_rule, "\\bar must draw a rule: {:?}", layout.draws);
        for draw in &layout.draws {
            if let MathLayoutDraw::Text { text, .. } = draw {
                assert!(
                    !text.chars().any(|c| ('\u{0300}'..='\u{036f}').contains(&c)),
                    "base text must not carry a combining mark: {text:?}"
                );
            }
        }
    }

    #[test]
    fn bold_command_marks_text_runs_bold_and_widens_them() {
        let layout = layout_markup_text(r"a\mathbf{b}\boldsymbol{c}", 100);
        let bold_texts: Vec<&str> = layout
            .draws
            .iter()
            .filter_map(|draw| match draw {
                MathLayoutDraw::Text {
                    text, bold: true, ..
                } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        // `b` (\mathbf) and `c` (\boldsymbol) are bold; the leading `a` is not.
        assert!(bold_texts.contains(&"b"), "{bold_texts:?}");
        assert!(bold_texts.contains(&"c"), "{bold_texts:?}");
        assert!(!bold_texts.contains(&"a"), "{bold_texts:?}");

        // Bold reserves more width than the same glyph in regular weight.
        let regular = layout_markup_text("x", 100);
        let bold = layout_markup_text(r"\mathbf{x}", 100);
        assert!(
            bold.width > regular.width,
            "bold should be wider: bold={}, regular={}",
            bold.width,
            regular.width
        );
    }
}
