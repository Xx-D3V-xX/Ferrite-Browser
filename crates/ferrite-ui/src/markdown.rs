//! Renders what the model wrote the way it meant it to be read.
//!
//! The agent's answers arrive as Markdown (headings, `**bold**`, lists, code
//! fences, tables, links), sometimes as bare JSON, sometimes as plain prose.
//! This module turns any of them into a tidy block layout. The parser is pure
//! (`parse` below, no UI types) so every shape of answer is covered by plain
//! unit tests; only `view` touches Iced.
//!
//! Answers are text from a model that has read untrusted pages, so nothing here
//! acts on its own: no HTML is interpreted, no image is fetched, and a link
//! only opens when the person clicks it, always in a new tab and always shown
//! with the site it really goes to.

use iced::widget::{column, container, row};
use iced::{Background, Border, Color, Element, Font, Length};
use iced_widget::{button, rich_text, span, text};

use crate::tokens::{
    card_style, tint, RADIUS_MD, RADIUS_SM, SP_MD, SP_SM, SP_XS, TEXT_CAPTION, TEXT_SMALL,
};
use crate::{font_weight, FerriteBrowserMessage, Palette};

/// A stretch of inline text with one style.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    /// An `http(s)` address this text links to.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Marker {
    Bullet,
    Number(u32),
    /// A task-list box, checked or not.
    Task(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Block {
    Heading {
        level: u8,
        runs: Vec<Run>,
    },
    Paragraph(Vec<Run>),
    Item {
        depth: u8,
        marker: Marker,
        runs: Vec<Run>,
    },
    Code {
        lang: String,
        body: String,
    },
    Quote(Vec<Run>),
    Rule,
    Table {
        header: Vec<Vec<Run>>,
        rows: Vec<Vec<Vec<Run>>>,
    },
}

// ---------------------------------------------------------------------------
// Parsing: blocks
// ---------------------------------------------------------------------------

/// The deepest list nesting shown; deeper items sit at this depth.
const MAX_DEPTH: u8 = 4;

/// Parses a model answer into blocks. Never fails: whatever is not recognised
/// is a paragraph of plain text.
pub(crate) fn parse(source: &str) -> Vec<Block> {
    let source = source.replace("\r\n", "\n");
    if let Some(pretty) = pretty_json(&source) {
        return vec![Block::Code {
            lang: "json".into(),
            body: pretty,
        }];
    }
    let lines: Vec<&str> = source.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        // Fenced code.
        if let Some((fence, lang)) = fence_open(trimmed) {
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(&fence) {
                body.push(lines[i]);
                i += 1;
            }
            i += 1; // the closing fence (or the end)
            blocks.push(Block::Code {
                lang,
                body: dedent(&body).join("\n"),
            });
            continue;
        }
        if let Some((level, rest)) = heading(trimmed) {
            blocks.push(Block::Heading {
                level,
                runs: inline(rest),
            });
            i += 1;
            continue;
        }
        if is_rule(trimmed) {
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }
        // A table: a header row, then the `---|---` separator.
        if trimmed.contains('|') && i + 1 < lines.len() && is_table_separator(lines[i + 1]) {
            let header = cells(trimmed);
            let columns = header.len();
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                let mut row = cells(lines[i].trim());
                row.resize(columns, String::new());
                rows.push(row.iter().map(|c| inline(c)).collect());
                i += 1;
            }
            blocks.push(Block::Table {
                header: header.iter().map(|c| inline(c)).collect(),
                rows,
            });
            continue;
        }
        if trimmed.starts_with('>') {
            let mut quoted = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                quoted.push(
                    lines[i]
                        .trim_start()
                        .trim_start_matches('>')
                        .trim_start()
                        .to_string(),
                );
                i += 1;
            }
            blocks.push(Block::Quote(inline(&quoted.join("\n"))));
            continue;
        }
        if let Some(item) = list_item(line) {
            let (depth, marker, first) = item;
            let mut body = first.to_string();
            i += 1;
            // Lines indented under the item continue it.
            while i < lines.len() {
                let next = lines[i];
                if next.trim().is_empty() || list_item(next).is_some() || next.trim_start() == next
                {
                    break;
                }
                body.push('\n');
                body.push_str(next.trim());
                i += 1;
            }
            blocks.push(Block::Item {
                depth,
                marker,
                runs: inline(&body),
            });
            continue;
        }
        // A paragraph: lines up to the next blank line or block start.
        let mut para = vec![trimmed.to_string()];
        i += 1;
        while i < lines.len() {
            let next = lines[i].trim();
            if next.is_empty()
                || fence_open(next).is_some()
                || heading(next).is_some()
                || is_rule(next)
                || next.starts_with('>')
                || list_item(lines[i]).is_some()
                || (next.contains('|') && i + 1 < lines.len() && is_table_separator(lines[i + 1]))
            {
                break;
            }
            para.push(next.to_string());
            i += 1;
        }
        blocks.push(Block::Paragraph(inline(&para.join("\n"))));
    }
    blocks
}

/// The whole answer is one JSON value: show it formatted, as code.
fn pretty_json(source: &str) -> Option<String> {
    let t = source.trim();
    if !(t.starts_with('{') && t.ends_with('}') || t.starts_with('[') && t.ends_with(']')) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(t).ok()?;
    // A bare list of numbers like `[1]` is more likely prose than data.
    if matches!(&value, serde_json::Value::Array(a) if a.iter().all(|v| v.is_number()))
        && t.len() < 12
    {
        return None;
    }
    serde_json::to_string_pretty(&value).ok()
}

fn fence_open(trimmed: &str) -> Option<(String, String)> {
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let n = trimmed.chars().take_while(|c| *c == ch).count();
    if n < 3 {
        return None;
    }
    let lang = trimmed[n..].split_whitespace().next().unwrap_or("");
    Some((ch.to_string().repeat(n), lang.to_string()))
}

fn dedent<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| if l.len() >= indent { &l[indent..] } else { *l })
        .collect()
}

fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = trimmed[hashes..].strip_prefix(' ')?;
    Some((hashes as u8, rest.trim().trim_end_matches('#').trim_end()))
}

fn is_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|m| compact.chars().all(|c| m.starts_with(c)))
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
        && t.trim_matches(|c| c == '|' || c == ' ').contains('-')
}

fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|')
        .map(|c| c.trim().replace("<br>", "\n"))
        .collect()
}

/// `(depth, marker, text)` when `line` starts a list item.
fn list_item(line: &str) -> Option<(u8, Marker, &str)> {
    let stripped = line.trim_start();
    let indent = line.len() - stripped.len();
    let depth = ((indent / 2) as u8).min(MAX_DEPTH);
    let (marker, rest) = if let Some(rest) = ["- ", "* ", "+ ", "• "]
        .iter()
        .find_map(|m| stripped.strip_prefix(m))
    {
        (Marker::Bullet, rest)
    } else {
        let digits = stripped.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let after = &stripped[digits..];
        let rest = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))?;
        (Marker::Number(stripped[..digits].parse().ok()?), rest)
    };
    let rest = rest.trim_start();
    if let Some(r) = rest.strip_prefix("[ ] ") {
        return Some((depth, Marker::Task(false), r));
    }
    if let Some(r) = rest
        .strip_prefix("[x] ")
        .or_else(|| rest.strip_prefix("[X] "))
    {
        return Some((depth, Marker::Task(true), r));
    }
    // `**Bold**` or `---` at the start of a line is not a bullet.
    if matches!(marker, Marker::Bullet) && (stripped.starts_with("**") || is_rule(stripped.trim()))
    {
        return None;
    }
    Some((depth, marker, rest))
}

// ---------------------------------------------------------------------------
// Parsing: inline
// ---------------------------------------------------------------------------

/// Parses inline Markdown into styled runs.
pub(crate) fn inline(source: &str) -> Vec<Run> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    inline_into(&chars, &Run::default(), &mut out);
    merge(out)
}

fn push(out: &mut Vec<Run>, style: &Run, text: String) {
    if text.is_empty() {
        return;
    }
    out.push(Run {
        text,
        ..style.clone()
    });
}

fn inline_into(chars: &[char], style: &Run, out: &mut Vec<Run>) {
    let mut buf = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev = if i == 0 { ' ' } else { chars[i - 1] };
        // Backslash escape.
        if c == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        // Inline code: the first matching run of backticks closes it.
        if c == '`' {
            let n = chars[i..].iter().take_while(|c| **c == '`').count();
            if let Some(end) = find_run(chars, i + n, '`', n) {
                push(out, style, std::mem::take(&mut buf));
                let code: String = chars[i + n..end].iter().collect();
                out.push(Run {
                    text: code.trim().to_string(),
                    code: true,
                    ..style.clone()
                });
                i = end + n;
                continue;
            }
        }
        // `[label](url)`
        if c == '[' {
            if let Some((label_end, url, after)) = link_at(chars, i) {
                push(out, style, std::mem::take(&mut buf));
                let label = &chars[i + 1..label_end];
                let link_style = Run {
                    link: safe_url(&url),
                    ..style.clone()
                };
                inline_into(label, &link_style, out);
                i = after;
                continue;
            }
        }
        // A bare address.
        if (c == 'h') && (prev == ' ' || prev == '(' || prev == '\n' || i == 0) {
            if let Some(end) = bare_url_end(chars, i) {
                push(out, style, std::mem::take(&mut buf));
                let url: String = chars[i..end].iter().collect();
                out.push(Run {
                    text: url.clone(),
                    link: safe_url(&url),
                    ..style.clone()
                });
                i = end;
                continue;
            }
        }
        // Emphasis: `**x**`, `__x__`, `~~x~~`, `*x*`, `_x_`.
        if matches!(c, '*' | '_' | '~') {
            let n = chars[i..].iter().take_while(|x| **x == c).count();
            let width = if n >= 2 { 2 } else { 1 };
            let tilde_ok = c != '~' || width == 2;
            let word_ok = c != '_' || !prev.is_alphanumeric();
            let next = chars.get(i + width).copied().unwrap_or(' ');
            if tilde_ok && word_ok && !next.is_whitespace() {
                if let Some(end) = find_closer(chars, i + width, c, width) {
                    push(out, style, std::mem::take(&mut buf));
                    let mut inner = style.clone();
                    match (c, width) {
                        ('~', _) => inner.strike = true,
                        (_, 2) => inner.bold = true,
                        _ => inner.italic = true,
                    }
                    inline_into(&chars[i + width..end], &inner, out);
                    i = end + width;
                    continue;
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    push(out, style, buf);
}

/// Index of a run of exactly `n` of `ch` at or after `from`.
fn find_run(chars: &[char], from: usize, ch: char, n: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == ch {
            let len = chars[i..].iter().take_while(|c| **c == ch).count();
            if len == n {
                return Some(i);
            }
            i += len;
        } else {
            i += 1;
        }
    }
    None
}

/// The closing delimiter of an emphasis opened at `from - width`: `width` of
/// `ch`, preceded by something that is not a space.
fn find_closer(chars: &[char], from: usize, ch: char, width: usize) -> Option<usize> {
    let mut i = from;
    while i + width <= chars.len() {
        if chars[i] == '`' {
            // Skip over code so its contents never close an emphasis.
            let n = chars[i..].iter().take_while(|c| **c == '`').count();
            i = find_run(chars, i + n, '`', n).map_or(i + n, |e| e + n);
            continue;
        }
        let run = chars[i..].iter().take_while(|c| **c == ch).count();
        if run >= width && i > from && !chars[i - 1].is_whitespace() {
            let after = chars.get(i + width).copied().unwrap_or(' ');
            // `_` closes only at a word end.
            if ch != '_' || !after.is_alphanumeric() {
                return Some(i + run - width);
            }
        }
        i += run.max(1);
    }
    None
}

/// `[label](url)` at `at`: `(index of ']', url, index after ')')`.
fn link_at(chars: &[char], at: usize) -> Option<(usize, String, usize)> {
    let mut depth = 0;
    let mut close = None;
    for (j, c) in chars.iter().enumerate().skip(at) {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(j);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = chars[close + 2..].iter().position(|c| *c == ')')? + close + 2;
    let url: String = chars[close + 2..end].iter().collect();
    // `[x](url "title")`: keep the address only.
    let url = url.split_whitespace().next().unwrap_or("").to_string();
    Some((close, url, end + 1))
}

fn bare_url_end(chars: &[char], at: usize) -> Option<usize> {
    let rest: String = chars[at..].iter().take(8).collect();
    if !(rest.starts_with("http://") || rest.starts_with("https://")) {
        return None;
    }
    let mut end = at;
    while end < chars.len() && !chars[end].is_whitespace() && !matches!(chars[end], '<' | '>' | '"')
    {
        end += 1;
    }
    // Trailing punctuation belongs to the sentence, not the address.
    while end > at
        && matches!(
            chars[end - 1],
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']'
        )
    {
        end -= 1;
    }
    (end > at + 8).then_some(end)
}

/// Only `http(s)` addresses become links; `javascript:`, `data:`, `file:` and
/// the rest stay plain text.
fn safe_url(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")).then(|| url.trim().to_string())
}

fn merge(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for run in runs {
        match out.last_mut() {
            Some(last)
                if last.bold == run.bold
                    && last.italic == run.italic
                    && last.strike == run.strike
                    && last.code == run.code
                    && last.link == run.link =>
            {
                last.text.push_str(&run.text);
            }
            _ => out.push(run),
        }
    }
    out
}

/// The site a link really goes to, for showing beside its label.
pub(crate) fn host_of(url: &str) -> String {
    let rest = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    rest.split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_string()
}

/// The plain text of some runs.
#[cfg(test)]
pub(crate) fn plain(runs: &[Run]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

type Msg = FerriteBrowserMessage;

/// The answer as a column of blocks. `size` is the body text size.
pub(crate) fn view<'a>(
    source: &str,
    palette: &'static Palette,
    size: f32,
    alpha: f32,
) -> Element<'a, Msg> {
    let fade = move |c: Color| Color {
        a: c.a * alpha,
        ..c
    };
    let blocks = parse(source);
    let mut col = column![].spacing(SP_SM).width(Length::Fill);
    let mut previous_was_item = false;
    for block in blocks {
        let is_item = matches!(block, Block::Item { .. });
        let element = render_block(block, palette, size, fade);
        // List items sit closer together than separate blocks.
        col = col.push(if is_item && previous_was_item {
            container(element).padding(iced::Padding {
                top: -(SP_XS + 1.0),
                ..iced::Padding::ZERO
            })
        } else {
            container(element)
        });
        previous_was_item = is_item;
    }
    col.into()
}

fn render_block<'a>(
    block: Block,
    palette: &'static Palette,
    size: f32,
    fade: impl Fn(Color) -> Color + Copy + 'a,
) -> Element<'a, Msg> {
    match block {
        Block::Heading { level, runs } => {
            let scale = match level {
                1 => 1.45,
                2 => 1.28,
                3 => 1.14,
                _ => 1.04,
            };
            let runs: Vec<Run> = runs.into_iter().map(|r| Run { bold: true, ..r }).collect();
            container(rich(runs, size * scale, palette, fade))
                .padding(iced::Padding {
                    top: if level <= 2 { SP_XS } else { 0.0 },
                    ..iced::Padding::ZERO
                })
                .into()
        }
        Block::Paragraph(runs) => rich(runs, size, palette, fade),
        Block::Item {
            depth,
            marker,
            runs,
        } => {
            let glyph = match marker {
                Marker::Bullet => match depth {
                    0 => "•".to_string(),
                    1 => "◦".to_string(),
                    _ => "▪".to_string(),
                },
                Marker::Number(n) => format!("{n}."),
                Marker::Task(true) => "☑".to_string(),
                Marker::Task(false) => "☐".to_string(),
            };
            let marker_color = match marker {
                Marker::Task(true) => palette.safe,
                _ => palette.text_dim,
            };
            row![
                text(glyph).size(size).color(fade(marker_color)).width(
                    if matches!(marker, Marker::Number(_)) {
                        22.0
                    } else {
                        14.0
                    }
                ),
                rich(runs, size, palette, fade),
            ]
            .spacing(SP_XS + 2.0)
            .padding(iced::Padding {
                left: 4.0 + f32::from(depth) * SP_MD,
                ..iced::Padding::ZERO
            })
            .into()
        }
        Block::Code { lang, body } => code_block(lang, body, palette, size, fade),
        Block::Quote(runs) => container(rich_dim(runs, size, palette, fade))
            .padding([SP_XS + 2.0, SP_MD])
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(fade(tint(palette.accent, 0.10)))),
                border: Border {
                    radius: RADIUS_SM.into(),
                    width: 1.0,
                    color: fade(tint(palette.accent, 0.35)),
                },
                ..container::Style::default()
            })
            .into(),
        Block::Rule => container(iced::widget::Space::new(Length::Fill, 1.0))
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(fade(palette.divider))),
                ..container::Style::default()
            })
            .into(),
        Block::Table { header, rows } => table(header, rows, palette, size, fade),
    }
}

fn run_font(run: &Run, bold: bool) -> Font {
    let mut font = if run.code {
        Font::MONOSPACE
    } else if bold {
        font_weight(iced::font::Weight::Semibold)
    } else {
        Font::with_name(crate::FONT_FAMILY)
    };
    if run.italic {
        font.style = iced::font::Style::Italic;
    }
    font
}

fn spans_for<'a>(
    runs: &[Run],
    size: f32,
    palette: &'static Palette,
    base: Color,
    fade: impl Fn(Color) -> Color + Copy,
) -> Vec<iced_widget::text::Span<'a, String, Font>> {
    let mut spans = Vec::new();
    for run in runs {
        let mut s = span(run.text.clone())
            .font(run_font(run, run.bold))
            .size(if run.code { size * 0.92 } else { size });
        if run.code {
            s = s
                .color(fade(palette.accent_bright))
                .background(Background::Color(fade(tint(palette.raised, 1.0))))
                .padding([1.0, 4.0]);
        } else if run.link.is_some() {
            s = s.color(fade(palette.accent_bright)).underline(true);
        } else {
            s = s.color(fade(base));
        }
        if run.strike {
            s = s.strikethrough(true);
        }
        if let Some(url) = &run.link {
            s = s.link(url.clone());
        }
        spans.push(s);
        // A link whose label is not its address shows where it goes.
        if let Some(url) = &run.link {
            let host = host_of(url);
            if !host.is_empty() && !run.text.contains(&host) {
                spans.push(
                    span(format!(" ({host})"))
                        .size(size * 0.85)
                        .color(fade(palette.text_dim)),
                );
            }
        }
    }
    spans
}

fn rich<'a>(
    runs: Vec<Run>,
    size: f32,
    palette: &'static Palette,
    fade: impl Fn(Color) -> Color + Copy + 'a,
) -> Element<'a, Msg> {
    let spans = spans_for(&runs, size, palette, palette.text, fade);
    Element::<String>::from(
        rich_text(spans)
            .width(Length::Fill)
            .wrapping(text::Wrapping::WordOrGlyph),
    )
    .map(Msg::OpenLink)
}

fn rich_dim<'a>(
    runs: Vec<Run>,
    size: f32,
    palette: &'static Palette,
    fade: impl Fn(Color) -> Color + Copy + 'a,
) -> Element<'a, Msg> {
    let spans = spans_for(&runs, size, palette, palette.text_dim, fade);
    Element::<String>::from(
        rich_text(spans)
            .width(Length::Fill)
            .wrapping(text::Wrapping::WordOrGlyph),
    )
    .map(Msg::OpenLink)
}

fn code_block<'a>(
    lang: String,
    body: String,
    palette: &'static Palette,
    size: f32,
    fade: impl Fn(Color) -> Color + Copy + 'a,
) -> Element<'a, Msg> {
    let label = if lang.is_empty() {
        "code".to_string()
    } else {
        lang
    };
    let header = row![
        text(label)
            .size(TEXT_CAPTION)
            .color(fade(palette.text_dim))
            .width(Length::Fill),
        button(text("Copy").size(TEXT_CAPTION))
            .padding([1.0, 6.0])
            .style(crate::agent_panel::link_button_style)
            .on_press(Msg::CopyAnswer(body.clone())),
    ]
    .align_y(iced::Alignment::Center);
    container(
        column![
            header,
            text(body)
                .font(Font::MONOSPACE)
                .size((size - 1.0).max(TEXT_SMALL - 1.0))
                .color(fade(palette.text))
                .wrapping(text::Wrapping::WordOrGlyph),
        ]
        .spacing(SP_XS)
        .width(Length::Fill),
    )
    .padding([SP_SM, SP_MD])
    .width(Length::Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(fade(palette.surface))),
        border: Border {
            radius: RADIUS_MD.into(),
            width: 1.0,
            color: fade(palette.divider),
        },
        ..container::Style::default()
    })
    .into()
}

fn table<'a>(
    header: Vec<Vec<Run>>,
    rows: Vec<Vec<Vec<Run>>>,
    palette: &'static Palette,
    size: f32,
    fade: impl Fn(Color) -> Color + Copy + 'a,
) -> Element<'a, Msg> {
    let cell_size = (size - 1.0).max(TEXT_SMALL);
    let make_row = move |cells: Vec<Vec<Run>>, head: bool| -> Element<'a, Msg> {
        let mut r = row![].spacing(SP_SM);
        for cell in cells {
            let cell: Vec<Run> = cell
                .into_iter()
                .map(|c| Run {
                    bold: c.bold || head,
                    ..c
                })
                .collect();
            r = r.push(
                container(rich(cell, cell_size, palette, fade)).width(Length::FillPortion(1)),
            );
        }
        container(r)
            .padding([SP_XS + 1.0, SP_SM])
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: head.then(|| Background::Color(fade(palette.raised))),
                border: Border {
                    width: 0.0,
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
    };
    let mut col = column![make_row(header, true)].width(Length::Fill);
    for r in rows {
        col = col.push(
            container(iced::widget::Space::new(Length::Fill, 1.0)).style(move |_| {
                container::Style {
                    background: Some(Background::Color(fade(palette.divider))),
                    ..container::Style::default()
                }
            }),
        );
        col = col.push(make_row(r, false));
    }
    container(col)
        .width(Length::Fill)
        .style(move |theme: &iced::Theme| container::Style {
            border: Border {
                radius: RADIUS_SM.into(),
                width: 1.0,
                color: fade(palette.divider),
            },
            ..card_style(theme)
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(s: &str) -> Vec<Run> {
        inline(s)
    }

    #[test]
    fn bold_italic_code_and_strike_become_styled_runs() {
        let r = runs("a **b** *c* `d` ~~e~~ f");
        let styled: Vec<_> = r
            .iter()
            .map(|r| (r.text.as_str(), r.bold, r.italic, r.code, r.strike))
            .collect();
        assert_eq!(
            styled,
            vec![
                ("a ", false, false, false, false),
                ("b", true, false, false, false),
                (" ", false, false, false, false),
                ("c", false, true, false, false),
                (" ", false, false, false, false),
                ("d", false, false, true, false),
                (" ", false, false, false, false),
                ("e", false, false, false, true),
                (" f", false, false, false, false),
            ]
        );
    }

    #[test]
    fn unbalanced_markers_stay_literal() {
        assert_eq!(
            plain(&runs("2 * 3 = 6, snake_case_name, **open")),
            "2 * 3 = 6, snake_case_name, **open"
        );
        assert!(runs("snake_case_name").iter().all(|r| !r.italic));
    }

    #[test]
    fn bold_can_hold_italic_and_code_is_not_re_parsed() {
        let r = runs("**a *b* c** `**x**`");
        assert!(r.iter().any(|r| r.bold && r.italic && r.text == "b"));
        assert!(r.iter().any(|r| r.code && r.text == "**x**"));
    }

    #[test]
    fn escapes_print_the_character() {
        assert_eq!(
            plain(&runs(r"\*not bold\* and \# hash")),
            "*not bold* and # hash"
        );
    }

    #[test]
    fn only_http_links_are_links() {
        let r =
            runs("[ok](https://a.example/x) [bad](javascript:alert(1)) [file](file:///etc/passwd)");
        assert_eq!(r[0].link.as_deref(), Some("https://a.example/x"));
        assert!(r.iter().skip(1).all(|r| r.link.is_none()));
        // The label of a refused link is still shown.
        assert!(plain(&r).contains("bad") && plain(&r).contains("file"));
    }

    #[test]
    fn bare_addresses_link_without_their_trailing_punctuation() {
        let r = runs("see https://example.com/a?b=1, and (https://x.example).");
        let links: Vec<_> = r.iter().filter_map(|r| r.link.as_deref()).collect();
        assert_eq!(
            links,
            vec!["https://example.com/a?b=1", "https://x.example"]
        );
    }

    #[test]
    fn the_host_shown_beside_a_link_is_the_real_one() {
        assert_eq!(host_of("https://www.news.example/a/b?c#d"), "news.example");
        assert_eq!(host_of("http://localhost:8080/x"), "localhost:8080");
    }

    #[test]
    fn headings_lists_and_paragraphs() {
        let blocks = parse(
            "# Title\n\nSome *text*\nsecond line\n\n- one\n- two\n  - nested\n1. first\n2. second",
        );
        assert!(matches!(&blocks[0], Block::Heading { level: 1, .. }));
        assert!(matches!(&blocks[1], Block::Paragraph(r) if plain(r) == "Some text\nsecond line"));
        let items: Vec<_> = blocks[2..]
            .iter()
            .map(|b| match b {
                Block::Item {
                    depth,
                    marker,
                    runs,
                } => (*depth, *marker, plain(runs)),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            items,
            vec![
                (0, Marker::Bullet, "one".to_string()),
                (0, Marker::Bullet, "two".to_string()),
                (1, Marker::Bullet, "nested".to_string()),
                (0, Marker::Number(1), "first".to_string()),
                (0, Marker::Number(2), "second".to_string()),
            ]
        );
    }

    #[test]
    fn the_screenshot_answer_parses_into_bold_headings_and_nested_bullets() {
        let blocks = parse(
            "The top stories include:\n\n- **AI & Machine Learning**:\n    - **Clef**: An open model.\n- **Tools**:\n    - **Pi 1.0**.",
        );
        assert!(matches!(&blocks[0], Block::Paragraph(_)));
        match &blocks[1] {
            Block::Item { depth: 0, runs, .. } => assert!(runs[0].bold),
            other => panic!("{other:?}"),
        }
        assert!(matches!(&blocks[2], Block::Item { depth: 2, .. }));
        // No `**` survives into any text.
        for b in &blocks {
            if let Block::Item { runs, .. } = b {
                assert!(!plain(runs).contains("**"), "{runs:?}");
            }
        }
    }

    #[test]
    fn task_lists() {
        let blocks = parse("- [x] done\n- [ ] todo");
        assert!(matches!(
            &blocks[0],
            Block::Item {
                marker: Marker::Task(true),
                ..
            }
        ));
        assert!(matches!(
            &blocks[1],
            Block::Item {
                marker: Marker::Task(false),
                ..
            }
        ));
    }

    #[test]
    fn fenced_code_keeps_its_text_and_language() {
        let blocks = parse("before\n```rust\nfn main() {\n    **not bold**\n}\n```\nafter");
        match &blocks[1] {
            Block::Code { lang, body } => {
                assert_eq!(lang, "rust");
                assert_eq!(body, "fn main() {\n    **not bold**\n}");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&blocks[2], Block::Paragraph(_)));
    }

    #[test]
    fn an_unclosed_fence_runs_to_the_end() {
        let blocks = parse("```\nline 1\nline 2");
        assert_eq!(
            blocks,
            vec![Block::Code {
                lang: String::new(),
                body: "line 1\nline 2".into()
            }]
        );
    }

    #[test]
    fn tables_with_alignment_rows_and_short_rows() {
        let blocks = parse("| Name | Pts |\n|:--|--:|\n| **A** | 1 |\n| B |");
        match &blocks[0] {
            Block::Table { header, rows } => {
                assert_eq!(plain(&header[0]), "Name");
                assert_eq!(rows.len(), 2);
                assert!(rows[0][0][0].bold);
                // A short row is padded to the header's width.
                assert_eq!(rows[1].len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn quotes_rules_and_a_pipe_in_prose() {
        let blocks = parse("> quoted *text*\n> more\n\n---\n\na | b is not a table");
        assert!(matches!(&blocks[0], Block::Quote(r) if plain(r) == "quoted text\nmore"));
        assert_eq!(blocks[1], Block::Rule);
        assert!(matches!(&blocks[2], Block::Paragraph(_)));
    }

    #[test]
    fn a_json_answer_is_shown_formatted() {
        let blocks = parse(r#"{"a":1,"b":[true,null]}"#);
        match &blocks[0] {
            Block::Code { lang, body } => {
                assert_eq!(lang, "json");
                assert!(body.contains("\n  \"a\": 1"), "{body}");
            }
            other => panic!("{other:?}"),
        }
        // Not JSON: ordinary text.
        assert!(matches!(&parse("[1] see the note")[0], Block::Paragraph(_)));
        assert!(matches!(&parse("{not json}")[0], Block::Paragraph(_)));
    }

    #[test]
    fn plain_prose_html_and_empty_input_never_fail() {
        assert!(parse("").is_empty());
        assert!(parse("   \n\n  ").is_empty());
        let blocks = parse("<script>alert(1)</script> hello");
        // Shown as text, never interpreted.
        assert!(matches!(&blocks[0], Block::Paragraph(r) if plain(r).contains("<script>")));
        assert!(matches!(&parse("just words")[0], Block::Paragraph(_)));
    }

    #[test]
    fn a_very_long_unbroken_answer_is_still_one_pass() {
        let long = "word ".repeat(20_000);
        assert_eq!(parse(&long).len(), 1);
        let nested = "*".repeat(5_000);
        let _ = parse(&nested);
    }
}
