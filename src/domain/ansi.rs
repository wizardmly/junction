//! ANSI escape codes in console output, as IntelliJ's `AnsiEscapeDecoder`
//! reads them: SGR (`ESC [ … m`) colors (the 16 named ones, 256-color
//! and 24-bit), bold, dim, italic, underline and inverse, kept from one
//! line to the next; OSC 8 hyperlinks become links (`decode_links`);
//! every other control sequence (cursor moves, erasing, OSC titles) is
//! dropped. A carriage return starts the line
//! over, as a progress bar redraws itself.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnsiColor {
    /// 0–7 the normal colors, 8–15 the bright ones.
    Named(u8),
    /// 16–255 of the 256-color palette.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnsiStyle {
    pub fg: Option<AnsiColor>,
    pub bg: Option<AnsiColor>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl AnsiStyle {
    pub fn is_plain(&self) -> bool {
        *self == AnsiStyle::default()
    }

    /// Applies an SGR sequence's parameters.
    fn apply(&mut self, params: &[u16]) {
        if params.is_empty() {
            *self = AnsiStyle::default();
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let p = params[i];
            match p {
                0 => *self = AnsiStyle::default(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                7 => self.inverse = true,
                22 => (self.bold, self.dim) = (false, false),
                23 => self.italic = false,
                24 => self.underline = false,
                27 => self.inverse = false,
                30..=37 => self.fg = Some(AnsiColor::Named((p - 30) as u8)),
                39 => self.fg = None,
                40..=47 => self.bg = Some(AnsiColor::Named((p - 40) as u8)),
                49 => self.bg = None,
                90..=97 => self.fg = Some(AnsiColor::Named((p - 90 + 8) as u8)),
                100..=107 => self.bg = Some(AnsiColor::Named((p - 100 + 8) as u8)),
                38 | 48 => {
                    let color = match params.get(i + 1) {
                        Some(5) => {
                            let n = params.get(i + 2).copied().unwrap_or(0).min(255) as u8;
                            i += 2;
                            Some(if n < 16 { AnsiColor::Named(n) } else { AnsiColor::Indexed(n) })
                        }
                        Some(2) => {
                            let c = |k: usize| params.get(i + k).copied().unwrap_or(0).min(255) as u8;
                            let rgb = AnsiColor::Rgb(c(2), c(3), c(4));
                            i += 4;
                            Some(rgb)
                        }
                        _ => None,
                    };
                    if p == 38 {
                        self.fg = color;
                    } else {
                        self.bg = color;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

/// Whether a line has anything to decode.
pub fn has_escapes(text: &str) -> bool {
    text.contains('\x1b') || text.contains('\r')
}

/// A line's text without its escape codes, and its styled stretches;
/// `state` is the style the line starts with and is left as it ends.
pub fn decode(line: &str, state: &mut AnsiStyle) -> (String, Vec<(Range<usize>, AnsiStyle)>) {
    let (text, spans, _) = decode_links(line, state);
    (text, spans)
}

/// `decode`, with the line's OSC 8 hyperlinks (`ESC ] 8 ; params ; URI ST`
/// … `ESC ] 8 ; ; ST`): the text each covers and its URI. One left open
/// ends with the line.
pub fn decode_links(line: &str, state: &mut AnsiStyle) -> (String, Vec<(Range<usize>, AnsiStyle)>, Vec<(Range<usize>, String)>) {
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    let mut open: Option<(usize, String)> = None;
    let mut text = String::new();
    let mut spans: Vec<(Range<usize>, AnsiStyle)> = Vec::new();
    let mut chars = line.char_indices().peekable();
    let mut run_start = 0;
    let close = |text: &str, spans: &mut Vec<(Range<usize>, AnsiStyle)>, start: usize, style: AnsiStyle| {
        if !style.is_plain() && start < text.len() {
            spans.push((start..text.len(), style));
        }
    };
    while let Some((_, c)) = chars.next() {
        match c {
            '\x1b' => match chars.peek().map(|(_, c)| *c) {
                Some('[') => {
                    chars.next();
                    let mut body = String::new();
                    let mut end = None;
                    for (_, c) in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            end = Some(c);
                            break;
                        }
                        body.push(c);
                    }
                    if end == Some('m') {
                        close(&text, &mut spans, run_start, *state);
                        let params: Vec<u16> = body.split([';', ':']).map(|p| p.parse().unwrap_or(0)).collect();
                        state.apply(if body.is_empty() { &[] } else { &params });
                        run_start = text.len();
                    }
                }
                Some(']') => {
                    // OSC: up to BEL or ESC \.
                    chars.next();
                    let mut body = String::new();
                    while let Some((_, c)) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek().is_some_and(|(_, c)| *c == '\\') {
                            chars.next();
                            break;
                        }
                        body.push(c);
                    }
                    // A hyperlink starts (with a URI) or ends (without).
                    if let Some((_, uri)) = body.strip_prefix("8;").and_then(|rest| rest.split_once(';')) {
                        if let Some((start, uri)) = open.take() {
                            links.push((start..text.len(), uri));
                        }
                        if !uri.is_empty() {
                            open = Some((text.len(), uri.to_owned()));
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\r' => {
                // Unless it ends the line, what follows overwrites it.
                if chars.peek().is_some() {
                    text.clear();
                    spans.clear();
                    links.clear();
                    open = open.map(|(_, uri)| (0, uri));
                    run_start = 0;
                }
            }
            c => text.push(c),
        }
    }
    close(&text, &mut spans, run_start, *state);
    if let Some((start, uri)) = open {
        links.push((start..text.len(), uri));
    }
    links.retain(|(r, _)| !r.is_empty());
    (text, spans, links)
}

/// The 256-color palette's color for 16–255: the 6×6×6 cube, then the
/// gray ramp.
pub fn indexed_rgb(n: u8) -> (u8, u8, u8) {
    if n >= 232 {
        let v = 8 + (n - 232) * 10;
        return (v, v, v);
    }
    let n = n.saturating_sub(16);
    let level = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
    (level(n / 36), level(n / 6 % 6), level(n % 6))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_colors_and_styles() {
        let mut state = AnsiStyle::default();
        let (text, spans) = decode("\x1b[1;31mFAILED\x1b[0m: 2 tests", &mut state);
        assert_eq!(text, "FAILED: 2 tests");
        assert_eq!(spans, vec![(0..6, AnsiStyle { fg: Some(AnsiColor::Named(1)), bold: true, ..Default::default() })]);
        assert!(state.is_plain());
        // 256 colors and 24-bit, bright.
        let (_, spans) = decode("\x1b[38;5;208ma\x1b[48;2;1;2;3mb\x1b[92mc\x1b[39;49md", &mut state);
        assert_eq!(spans[0].1.fg, Some(AnsiColor::Indexed(208)));
        assert_eq!(spans[1].1.bg, Some(AnsiColor::Rgb(1, 2, 3)));
        assert_eq!(spans[2].1.fg, Some(AnsiColor::Named(10)));
        assert_eq!(spans.len(), 3);
    }

    #[test]
    fn keeps_the_style_across_lines() {
        let mut state = AnsiStyle::default();
        let (_, spans) = decode("\x1b[33mwarning: one", &mut state);
        assert_eq!(spans, vec![(0..12, AnsiStyle { fg: Some(AnsiColor::Named(3)), ..Default::default() })]);
        let (text, spans) = decode("two\x1b[m three", &mut state);
        assert_eq!((text.as_str(), spans.len(), spans[0].0.clone()), ("two three", 1, 0..3));
    }

    #[test]
    fn osc8_hyperlinks() {
        let mut state = AnsiStyle::default();
        let line = "see \x1b]8;;https://example.com/a\x1b\\the docs\x1b]8;;\x1b\\ or \x1b]8;id=1;file:///tmp/x.txt\x07x.txt";
        let (text, _, links) = decode_links(line, &mut state);
        assert_eq!(text, "see the docs or x.txt");
        assert_eq!(links, [(4..12, "https://example.com/a".to_owned()), (16..21, "file:///tmp/x.txt".to_owned())]);
    }

    #[test]
    fn drops_other_sequences_and_redraws_on_carriage_return() {
        let mut state = AnsiStyle::default();
        let (text, spans) = decode("\x1b]0;title\x07\x1b[2Kdone\x1b[1A", &mut state);
        assert_eq!((text.as_str(), spans.len()), ("done", 0));
        let (text, _) = decode("10%\r50%\r100%", &mut state);
        assert_eq!(text, "100%");
        let (text, _) = decode("windows line\r", &mut state);
        assert_eq!(text, "windows line");
        assert!(!has_escapes("plain"));
    }

    #[test]
    fn maps_the_256_color_palette() {
        assert_eq!(indexed_rgb(16), (0, 0, 0));
        assert_eq!(indexed_rgb(196), (255, 0, 0));
        assert_eq!(indexed_rgb(232), (8, 8, 8));
    }
}
