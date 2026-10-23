//! The length of SVG path data (VectorDrawable's `pathData`), for
//! `trimPathStart` / `trimPathEnd` / `trimPathOffset`: the renderer has no
//! `pathLength`, so trimmed strokes are drawn as dashes of real lengths.
//! Curves and arcs are measured by flattening them.

/// Segments per curve or arc when flattening.
const STEPS: usize = 32;

type Point = (f32, f32);

fn dist(a: Point, b: Point) -> f32 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// The path data's numbers and commands, in order.
#[derive(Debug, PartialEq)]
enum Token {
    Command(char),
    Number(f32),
}

fn tokens(data: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let bytes = data.as_bytes();
    let mut i = 0;
    // An arc's flags are single digits, may be written together (`01`).
    let mut arc = false;
    let mut count = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_alphabetic() && !matches!(c, 'e' | 'E') {
            out.push(Token::Command(c));
            arc = matches!(c, 'a' | 'A');
            count = 0;
            i += 1;
        } else if arc && matches!(count % 7, 3 | 4) && matches!(c, '0' | '1') {
            out.push(Token::Number(if c == '1' { 1. } else { 0. }));
            count += 1;
            i += 1;
        } else if c.is_ascii_digit() || matches!(c, '-' | '+' | '.') {
            // A number: sign, digits, one dot, exponent.
            let start = i;
            i += 1;
            let mut dot = c == '.';
            while i < bytes.len() {
                let d = bytes[i] as char;
                if d.is_ascii_digit() {
                    i += 1;
                } else if d == '.' && !dot {
                    dot = true;
                    i += 1;
                } else if matches!(d, 'e' | 'E') {
                    i += 1;
                    if i < bytes.len() && matches!(bytes[i], b'-' | b'+') {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            if let Ok(n) = data[start..i].parse() {
                out.push(Token::Number(n));
                count += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

fn cubic(p0: Point, p1: Point, p2: Point, p3: Point) -> f32 {
    let at = |t: f32| {
        let u = 1. - t;
        (
            u * u * u * p0.0 + 3. * u * u * t * p1.0 + 3. * u * t * t * p2.0 + t * t * t * p3.0,
            u * u * u * p0.1 + 3. * u * u * t * p1.1 + 3. * u * t * t * p2.1 + t * t * t * p3.1,
        )
    };
    (1..=STEPS).map(|i| dist(at((i - 1) as f32 / STEPS as f32), at(i as f32 / STEPS as f32))).sum()
}

fn quad(p0: Point, p1: Point, p2: Point) -> f32 {
    let at = |t: f32| {
        let u = 1. - t;
        (u * u * p0.0 + 2. * u * t * p1.0 + t * t * p2.0, u * u * p0.1 + 2. * u * t * p1.1 + t * t * p2.1)
    };
    (1..=STEPS).map(|i| dist(at((i - 1) as f32 / STEPS as f32), at(i as f32 / STEPS as f32))).sum()
}

/// An elliptical arc's length (SVG's endpoint form, converted to centre
/// form as the SVG spec's appendix does).
fn arc(p0: Point, rx: f32, ry: f32, angle: f32, large: bool, sweep: bool, p1: Point) -> f32 {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0. || ry == 0. {
        return dist(p0, p1);
    }
    let phi = angle.to_radians();
    let (cos, sin) = (phi.cos(), phi.sin());
    let (dx, dy) = ((p0.0 - p1.0) / 2., (p0.1 - p1.1) / 2.);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1. {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = if den == 0. { 0. } else { (num / den).sqrt() };
    if large == sweep {
        k = -k;
    }
    let (cx1, cy1) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let angle_of = |ux: f32, uy: f32| uy.atan2(ux);
    let theta = angle_of((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut delta = angle_of((-x1 - cx1) / rx, (-y1 - cy1) / ry) - theta;
    let tau = std::f32::consts::TAU;
    if sweep && delta < 0. {
        delta += tau;
    } else if !sweep && delta > 0. {
        delta -= tau;
    }
    let (cx, cy) = (cos * cx1 - sin * cy1 + (p0.0 + p1.0) / 2., sin * cx1 + cos * cy1 + (p0.1 + p1.1) / 2.);
    let at = |t: f32| {
        let a = theta + delta * t;
        let (ex, ey) = (rx * a.cos(), ry * a.sin());
        (cos * ex - sin * ey + cx, sin * ex + cos * ey + cy)
    };
    (1..=STEPS).map(|i| dist(at((i - 1) as f32 / STEPS as f32), at(i as f32 / STEPS as f32))).sum()
}

/// The total length of the path data, as Android's PathMeasure adds up
/// its contours (a close counts its line back to the start).
pub fn length(data: &str) -> f32 {
    let tokens = tokens(data);
    let mut total = 0.;
    let (mut cur, mut start): (Point, Point) = ((0., 0.), (0., 0.));
    // The last control point, for S and T.
    let mut last_cubic: Option<Point> = None;
    let mut last_quad: Option<Point> = None;
    let mut i = 0;
    let mut command = 'M';
    while i < tokens.len() {
        if let Token::Command(c) = tokens[i] {
            command = c;
            i += 1;
            if matches!(c, 'z' | 'Z') {
                total += dist(cur, start);
                cur = start;
                last_cubic = None;
                last_quad = None;
                continue;
            }
        }
        let arity = match command.to_ascii_uppercase() {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            _ => {
                i += 1;
                continue;
            }
        };
        let numbers: Vec<f32> = tokens[i..].iter().take(arity).map_while(|t| if let Token::Number(n) = t { Some(*n) } else { None }).collect();
        if numbers.len() < arity {
            // Skip to the next command.
            i += numbers.len().max(1);
            continue;
        }
        i += arity;
        let rel = command.is_ascii_lowercase();
        let p = |x: f32, y: f32| if rel { (cur.0 + x, cur.1 + y) } else { (x, y) };
        let (mut next_cubic, mut next_quad) = (None, None);
        match command.to_ascii_uppercase() {
            'M' => {
                cur = p(numbers[0], numbers[1]);
                start = cur;
                // Further pairs are lines.
                command = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let to = p(numbers[0], numbers[1]);
                total += dist(cur, to);
                cur = to;
            }
            'H' => {
                let to = (if rel { cur.0 + numbers[0] } else { numbers[0] }, cur.1);
                total += dist(cur, to);
                cur = to;
            }
            'V' => {
                let to = (cur.0, if rel { cur.1 + numbers[0] } else { numbers[0] });
                total += dist(cur, to);
                cur = to;
            }
            'C' => {
                let (c1, c2, to) = (p(numbers[0], numbers[1]), p(numbers[2], numbers[3]), p(numbers[4], numbers[5]));
                total += cubic(cur, c1, c2, to);
                next_cubic = Some(c2);
                cur = to;
            }
            'S' => {
                let c1 = last_cubic.map_or(cur, |c| (2. * cur.0 - c.0, 2. * cur.1 - c.1));
                let (c2, to) = (p(numbers[0], numbers[1]), p(numbers[2], numbers[3]));
                total += cubic(cur, c1, c2, to);
                next_cubic = Some(c2);
                cur = to;
            }
            'Q' => {
                let (c, to) = (p(numbers[0], numbers[1]), p(numbers[2], numbers[3]));
                total += quad(cur, c, to);
                next_quad = Some(c);
                cur = to;
            }
            'T' => {
                let c = last_quad.map_or(cur, |q| (2. * cur.0 - q.0, 2. * cur.1 - q.1));
                let to = p(numbers[0], numbers[1]);
                total += quad(cur, c, to);
                next_quad = Some(c);
                cur = to;
            }
            'A' => {
                let n = &numbers;
                let to = p(n[5], n[6]);
                total += arc(cur, n[0], n[1], n[2], n[3] != 0., n[4] != 0., to);
                cur = to;
            }
            _ => {}
        }
        last_cubic = next_cubic;
        last_quad = next_quad;
    }
    total
}

/// The stroke dashes that draw `[start, end]` (fractions, after the
/// offset, wrapping past 1) of a path `length` long: the SVG
/// `stroke-dasharray` and `stroke-dashoffset`; none when nothing is
/// trimmed, an empty array when nothing is left.
pub fn trim_dashes(length: f32, start: f32, end: f32, offset: f32) -> Option<(String, f32)> {
    if (start <= 0. && end >= 1.) || length <= 0. {
        return None;
    }
    let (s, e) = (start.min(end), start.max(end));
    if e - s <= 0. {
        return Some((String::new(), 0.));
    }
    let s = (s + offset).rem_euclid(1.);
    let e = (e + offset).rem_euclid(1.);
    let e = if e == 0. && s > 0. { 1. } else { e };
    // Rounded: the SVG stays short and stable.
    let l = |f: f32| (f * length * 1000.).round() / 1000.;
    if s <= e {
        // One visible piece: a dash from s to e.
        Some((format!("{} {}", l(e - s), length * 2.), -l(s)))
    } else {
        // It wraps: [0, e] and [s, 1].
        Some((format!("{} {} {} {}", l(e), l(s - e), l(1. - s), length), 0.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.05
    }

    #[test]
    fn measures_paths() {
        assert!(close(length("M0,0 L10,0 l0,10"), 20.));
        assert!(close(length("M0 0h10v10z"), 20. + 200f32.sqrt()));
        // Implicit lines after a move.
        assert!(close(length("M0,0 10,0 10,10"), 20.));
        // A half circle of radius 10.
        assert!(close(length("M0,10 A10,10 0 0,1 20,10"), std::f32::consts::PI * 10.));
        // A cubic that is a straight line.
        assert!(close(length("M0,0 C3,0 6,0 9,0"), 9.));
        assert!(close(length("M0,0 Q5,0 10,0 T20,0"), 20.));
        assert!(close(length("M2.5e1,0 L0,0"), 25.));
        // Flags written together.
        assert!(close(length("M0,10 a10 10 0 0120 0"), std::f32::consts::PI * 10.));
    }

    #[test]
    fn trims_into_dashes() {
        assert_eq!(trim_dashes(100., 0., 1., 0.), None);
        assert_eq!(trim_dashes(100., 0.25, 0.75, 0.), Some(("50 200".into(), -25.)));
        assert_eq!(trim_dashes(100., 0.5, 0.5, 0.), Some((String::new(), 0.)));
        // Offset past the end wraps: [0.1, 0.2] and [0.9, 1].
        assert_eq!(trim_dashes(100., 0., 0.3, 0.9), Some(("20 70 10 100".into(), 0.)));
    }
}
