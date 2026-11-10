//! Android's `NinePatch` for the drawable preview: a `.9.png`'s 1px border
//! marks, in opaque black, the columns (top edge) and rows (left edge)
//! that stretch; the rest keeps its size. The PNG is decoded here (8-bit,
//! not interlaced, as nine-patches are made) to read the border.

use std::io::Read as _;
use std::ops::Range;

/// The stretchable columns and rows, in pixels of the image inside the
/// border.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Regions {
    pub columns: Vec<Range<u32>>,
    pub rows: Vec<Range<u32>>,
}

/// One piece of the image placed in the bounds: from `source` (pixels of
/// the image inside the border) to `target` (dp from the bounds' start).
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub source: Range<f32>,
    pub target: Range<f32>,
}

/// The stretchable regions of a `.9.png`, if it decodes and marks any.
pub fn regions(png: &[u8]) -> Option<Regions> {
    let (w, h, rgba) = decode(png)?;
    if w < 3 || h < 3 {
        return None;
    }
    let black = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        rgba.get(i..i + 4) == Some(&[0, 0, 0, 255][..])
    };
    let runs = |len: u32, marked: &dyn Fn(u32) -> bool| {
        let mut out: Vec<Range<u32>> = Vec::new();
        for i in 1..len - 1 {
            if marked(i) {
                match out.last_mut() {
                    Some(r) if r.end == i - 1 => r.end = i,
                    _ => out.push(i - 1..i),
                }
            }
        }
        out
    };
    let columns = runs(w, &|x| black(x, 0));
    let rows = runs(h, &|y| black(0, y));
    Some(Regions { columns, rows })
}

/// How one axis of `size` pixels (inside the border) fills `target` dp:
/// fixed parts at `density` pixels per dp, the stretchable ones sharing
/// what's left by their sizes; with too little room the fixed parts
/// shrink alike and the stretchable ones get none, as `NinePatch` draws.
pub fn pieces(stretch: &[Range<u32>], size: u32, target: f32, density: f32) -> Vec<Piece> {
    if stretch.is_empty() {
        return vec![Piece { source: 0.0..size as f32, target: 0.0..target }];
    }
    // The axis cut into fixed and stretchable runs.
    let mut runs: Vec<(Range<u32>, bool)> = Vec::new();
    let mut at = 0;
    for r in stretch {
        if r.start > at {
            runs.push((at..r.start, false));
        }
        runs.push((r.clone(), true));
        at = r.end;
    }
    if at < size {
        runs.push((at..size, false));
    }
    let fixed: f32 = runs.iter().filter(|(_, s)| !s).map(|(r, _)| r.len() as f32 / density).sum();
    let stretchy: u32 = runs.iter().filter(|(_, s)| *s).map(|(r, _)| r.len() as u32).sum();
    let room = target - fixed;
    let shrink = if room < 0. && fixed > 0. { target / fixed } else { 1. };
    let mut pos = 0.;
    let mut out = Vec::new();
    for (r, stretches) in runs {
        let len = if stretches {
            if room > 0. && stretchy > 0 { room * r.len() as f32 / stretchy as f32 } else { 0. }
        } else {
            r.len() as f32 / density * shrink
        };
        if len > 0. {
            out.push(Piece { source: r.start as f32..r.end as f32, target: pos..pos + len });
        }
        pos += len;
    }
    out
}

/// A PNG as RGBA: 8-bit samples, not interlaced.
pub fn decode(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut at = 8;
    let (mut w, mut h, mut color) = (0u32, 0u32, 0u8);
    let mut data = Vec::new();
    let mut palette: &[u8] = &[];
    let mut alpha: &[u8] = &[];
    while at + 8 <= png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().ok()?) as usize;
        let kind = &png[at + 4..at + 8];
        let body = png.get(at + 8..at + 8 + len)?;
        match kind {
            b"IHDR" => {
                w = u32::from_be_bytes(body.get(0..4)?.try_into().ok()?);
                h = u32::from_be_bytes(body.get(4..8)?.try_into().ok()?);
                let (depth, interlace) = (*body.get(8)?, *body.get(12)?);
                color = *body.get(9)?;
                if depth != 8 || interlace != 0 {
                    return None;
                }
            }
            b"PLTE" => palette = body,
            b"tRNS" => alpha = body,
            b"IDAT" => data.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let channels = match color {
        0 | 3 => 1,
        4 => 2,
        2 => 3,
        6 => 4,
        _ => return None,
    };
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&data[..]).read_to_end(&mut raw).ok()?;
    let stride = w as usize * channels;
    if raw.len() < (stride + 1) * h as usize {
        return None;
    }
    // Undo each row's filter.
    let mut pixels = vec![0u8; stride * h as usize];
    for y in 0..h as usize {
        let filter = raw[y * (stride + 1)];
        let line = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= channels { pixels[y * stride + x - channels] } else { 0 };
            let b = if y > 0 { pixels[(y - 1) * stride + x] } else { 0 };
            let c = if x >= channels && y > 0 { pixels[(y - 1) * stride + x - channels] } else { 0 };
            let predicted = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                4 => {
                    let p = a as i16 + b as i16 - c as i16;
                    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return None,
            };
            pixels[y * stride + x] = line[x].wrapping_add(predicted);
        }
    }
    let rgba = pixels
        .chunks(channels)
        .flat_map(|p| match color {
            0 => [p[0], p[0], p[0], 255],
            4 => [p[0], p[0], p[0], p[1]],
            2 => [p[0], p[1], p[2], 255],
            6 => [p[0], p[1], p[2], p[3]],
            _ => {
                let i = p[0] as usize;
                let rgb = palette.get(i * 3..i * 3 + 3).unwrap_or(&[0, 0, 0]);
                [rgb[0], rgb[1], rgb[2], alpha.get(i).copied().unwrap_or(255)]
            }
        })
        .collect();
    Some((w, h, rgba))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write as _;

    /// An RGBA PNG of `w` × `h` from a pixel function, rows filtered by
    /// turns (none, sub, up, average, Paeth) to cover every filter.
    pub(crate) fn png(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let rows: Vec<Vec<u8>> = (0..h).map(|y| (0..w).flat_map(|x| px(x, y)).collect()).collect();
        let mut raw = Vec::new();
        for (y, row) in rows.iter().enumerate() {
            let filter = (y % 5) as u8;
            raw.push(filter);
            for x in 0..row.len() {
                let a = if x >= 4 { row[x - 4] } else { 0 };
                let b = if y > 0 { rows[y - 1][x] } else { 0 };
                let c = if x >= 4 && y > 0 { rows[y - 1][x - 4] } else { 0 };
                let predicted = match filter {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((a as u16 + b as u16) / 2) as u8,
                    _ => {
                        let p = a as i16 + b as i16 - c as i16;
                        let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
                        if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
                    }
                };
                raw.push(row[x].wrapping_sub(predicted));
            }
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&raw).unwrap();
        let chunk = |kind: &[u8], body: &[u8]| {
            let mut c = (body.len() as u32).to_be_bytes().to_vec();
            c.extend_from_slice(kind);
            c.extend_from_slice(body);
            c.extend_from_slice(&[0, 0, 0, 0]);
            c
        };
        let mut ihdr = w.to_be_bytes().to_vec();
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend(chunk(b"IHDR", &ihdr));
        out.extend(chunk(b"IDAT", &z.finish().unwrap()));
        out.extend(chunk(b"IEND", &[]));
        out
    }

    /// A 12 × 8 nine-patch: columns 3–6 and rows 2–3 (inside the border) stretch.
    pub(crate) fn nine() -> Vec<u8> {
        png(12, 8, |x, y| match (x, y) {
            (4..=7, 0) | (0, 3..=4) => [0, 0, 0, 255],
            (0, _) | (11, _) | (_, 0) | (_, 7) => [0, 0, 0, 0],
            _ => [x as u8 * 20, y as u8 * 30, 100, 255],
        })
    }

    #[test]
    fn decodes_and_finds_regions() {
        let (w, h, rgba) = decode(&nine()).unwrap();
        assert_eq!((w, h), (12, 8));
        assert_eq!(&rgba[(2 * 12 + 5) * 4..(2 * 12 + 5) * 4 + 4], &[100, 60, 100, 255]);
        assert_eq!(regions(&nine()), Some(Regions { columns: vec![3..7], rows: vec![2..4] }));
    }

    #[test]
    fn stretches_only_the_marked_parts() {
        // 10 px: 3 fixed, 4 stretch, 3 fixed; at 1 px/dp into 30 dp.
        let p = pieces(&[3..7], 10, 30., 1.);
        let t: Vec<_> = p.iter().map(|p| (p.source.clone(), p.target.clone())).collect();
        assert_eq!(t, [(0.0..3.0, 0.0..3.0), (3.0..7.0, 3.0..27.0), (7.0..10.0, 27.0..30.0)]);
        // Two stretchable runs share by size; density 2.
        let p = pieces(&[0..2, 6..10], 10, 20., 2.);
        assert_eq!(p.iter().map(|p| p.target.clone()).collect::<Vec<_>>(), [0.0..6.0, 6.0..8.0, 8.0..20.0]);
        // Too small: the fixed parts shrink, the stretchable ones vanish.
        let p = pieces(&[3..7], 10, 3., 1.);
        assert_eq!(p.iter().map(|p| p.target.clone()).collect::<Vec<_>>(), [0.0..1.5, 1.5..3.0]);
    }
}
