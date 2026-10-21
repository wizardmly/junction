//! Layout Inspector's snapshot of the screen: each window's view tree, read
//! from `dumpsys activity top` (Views with their ids, bounds and flags, as
//! Android Studio's legacy inspector got them) or from `uiautomator dump`
//! (also Compose and text, through accessibility).

/// A view's place on the screen in device pixels: left, top, right, bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewNode {
    /// `android.widget.TextView`.
    pub class: String,
    /// `app:id/title`, `android:id/content`.
    pub id: Option<String>,
    pub text: Option<String>,
    pub bounds: Rect,
    pub visible: bool,
    pub depth: usize,
    /// What the attributes panel lists, in order.
    pub attributes: Vec<(String, String)>,
}

impl ViewNode {
    /// `TextView`.
    pub fn short_class(&self) -> &str {
        self.class.rsplit(['.', '$']).next().unwrap_or(&self.class)
    }

    /// `title` of `app:id/title`.
    pub fn id_name(&self) -> Option<&str> {
        self.id.as_deref().map(|id| id.rsplit('/').next().unwrap_or(id))
    }
}

/// A window's views in tree order (each node before its children).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutWindow {
    pub name: String,
    pub nodes: Vec<ViewNode>,
}

impl LayoutWindow {
    /// The node's children's range: the nodes after it that are deeper.
    pub fn subtree(&self, ix: usize) -> std::ops::Range<usize> {
        let depth = self.nodes[ix].depth;
        let end = self.nodes[ix + 1..].iter().position(|n| n.depth <= depth).map_or(self.nodes.len(), |p| ix + 1 + p);
        ix + 1..end
    }

    /// The parent of a node.
    pub fn parent(&self, ix: usize) -> Option<usize> {
        let depth = self.nodes[ix].depth;
        self.nodes[..ix].iter().rposition(|n| n.depth < depth)
    }

    /// Whether the node and every view above it are visible.
    pub fn shown(&self, ix: usize) -> bool {
        let mut at = Some(ix);
        while let Some(i) = at {
            if !self.nodes[i].visible {
                return false;
            }
            at = self.parent(i);
        }
        true
    }

    /// The innermost shown view under the point, as a click in the
    /// inspector's image picks it.
    pub fn view_at(&self, x: i32, y: i32) -> Option<usize> {
        (0..self.nodes.len()).rev().filter(|&i| self.nodes[i].bounds.contains(x, y) && self.shown(i)).max_by_key(|&i| self.nodes[i].depth)
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn yes(flag: bool) -> String {
    flag.to_string()
}

/// One line of a `View.toString()` dump:
/// `android.widget.TextView{5a1c3f V.ED..... ......ID 42,0-300,60 #7f0800a1 app:id/title}`.
fn parse_view(line: &str) -> Option<(String, Rect, bool, Option<String>, Vec<(String, String)>)> {
    let open = line.find('{')?;
    let class = line[..open].trim().to_owned();
    let inner = line[open + 1..].trim_end().strip_suffix('}')?;
    let mut parts = inner.split_whitespace();
    let hash = parts.next()?;
    let flags = parts.next()?;
    let private = parts.next()?;
    let (pos, size) = parts.next()?.split_once('-')?;
    let (l, t) = pos.split_once(',')?;
    let (r, b) = size.split_once(',')?;
    let rect = Rect { left: l.parse().ok()?, top: t.parse().ok()?, right: r.parse().ok()?, bottom: b.parse().ok()? };
    let rest: Vec<&str> = parts.collect();
    let id = rest.iter().find(|p| p.contains(":id/")).map(|s| s.to_string());
    let numeric = rest.iter().find(|p| p.starts_with('#')).map(|s| s.to_string());
    let f = |s: &str, i: usize, c: char| s.chars().nth(i) == Some(c);
    let visibility = match flags.chars().next() {
        Some('V') => "visible",
        Some('I') => "invisible",
        _ => "gone",
    };
    let mut attributes = vec![("visibility".to_owned(), visibility.to_owned())];
    attributes.extend([
        ("focusable", f(flags, 1, 'F')),
        ("enabled", f(flags, 2, 'E')),
        ("willNotDraw", f(flags, 3, 'D')),
        ("clickable", f(flags, 6, 'C')),
        ("longClickable", f(flags, 7, 'L')),
        ("focused", f(private, 1, 'F')),
        ("selected", f(private, 2, 'S')),
        ("pressed", f(private, 3, 'P')),
        ("activated", f(private, 5, 'A')),
    ].map(|(k, v)| (k.to_owned(), yes(v))));
    attributes.push(("hashCode".to_owned(), hash.to_owned()));
    if let Some(numeric) = numeric {
        attributes.push(("idValue".to_owned(), numeric.trim_start_matches('#').to_owned()));
    }
    Some((class, rect, visibility == "visible", id, attributes))
}

/// The windows of `dumpsys activity top`: each activity's View Hierarchy,
/// named by its component; the resumed one first.
pub fn parse_dumpsys(text: &str) -> Vec<LayoutWindow> {
    let lines: Vec<&str> = text.lines().collect();
    let mut windows: Vec<(bool, LayoutWindow)> = Vec::new();
    let (mut activity, mut resumed) = (String::new(), false);
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("ACTIVITY ") {
            activity = rest.split_whitespace().next().unwrap_or_default().to_owned();
            resumed = false;
        } else if trimmed.contains("mResumed=true") {
            resumed = true;
        } else if trimmed == "View Hierarchy:" {
            let base = indent(line);
            let mut window = LayoutWindow { name: activity.clone(), nodes: Vec::new() };
            // Each view's offset on the screen, by depth.
            let mut origins: Vec<(i32, i32)> = Vec::new();
            i += 1;
            while i < lines.len() && indent(lines[i]) > base && !lines[i].trim().is_empty() {
                let depth = (indent(lines[i]) - base) / 2 - 1;
                let body = lines[i].trim();
                i += 1;
                origins.truncate(depth);
                let (ox, oy) = origins.last().copied().unwrap_or((0, 0));
                if let Some((class, rect, visible, id, attributes)) = parse_view(body) {
                    let bounds = Rect { left: ox + rect.left, top: oy + rect.top, right: ox + rect.right, bottom: oy + rect.bottom };
                    origins.push((bounds.left, bounds.top));
                    let mut all = vec![("layout".to_owned(), format!("{}, {}, {}, {}", rect.left, rect.top, rect.right, rect.bottom)), ("size".to_owned(), format!("{} × {}", rect.width(), rect.height()))];
                    all.extend(attributes);
                    window.nodes.push(ViewNode { class, id, text: None, bounds, visible, depth, attributes: all });
                } else if body.contains('@') && !body.contains('{') {
                    // `DecorView@5c3b2a[MainActivity]`: the window's root,
                    // its size that of its first child.
                    let class = body.split('@').next().unwrap_or(body).to_owned();
                    origins.push((0, 0));
                    window.nodes.push(ViewNode { class, id: None, text: None, bounds: Rect::default(), visible: true, depth, attributes: Vec::new() });
                } else {
                    origins.push((ox, oy));
                }
            }
            // A root without bounds covers its children.
            for ix in 0..window.nodes.len() {
                if window.nodes[ix].bounds == Rect::default() {
                    let range = window.subtree(ix);
                    if let Some(first) = window.nodes.get(range.start).filter(|_| !range.is_empty()) {
                        let b = first.bounds;
                        window.nodes[ix].bounds = b;
                        window.nodes[ix].attributes = vec![("size".to_owned(), format!("{} × {}", b.width(), b.height()))];
                    }
                }
            }
            if !window.nodes.is_empty() {
                windows.push((resumed, window));
            }
            continue;
        }
        i += 1;
    }
    windows.sort_by_key(|(resumed, _)| !resumed);
    windows.into_iter().map(|(_, w)| w).collect()
}

/// `[0,0][1080,2400]`.
fn parse_bounds(text: &str) -> Option<Rect> {
    let numbers: Vec<i32> = text.split(|c: char| !(c.is_ascii_digit() || c == '-')).filter(|s| !s.is_empty()).map(|s| s.parse().ok()).collect::<Option<_>>()?;
    match numbers[..] {
        [left, top, right, bottom] => Some(Rect { left, top, right, bottom }),
        _ => None,
    }
}

/// The window of a `uiautomator dump`.
pub fn parse_uiautomator(xml: &str) -> Option<LayoutWindow> {
    let start = xml.find("<?xml").or_else(|| xml.find("<hierarchy"))?;
    let end = xml.rfind("</hierarchy>").map_or(xml.len(), |e| e + "</hierarchy>".len());
    let doc = roxmltree::Document::parse(&xml[start..end]).ok()?;
    let root = doc.root_element();
    let mut window = LayoutWindow::default();
    fn walk(node: roxmltree::Node, depth: usize, window: &mut LayoutWindow) {
        for child in node.children().filter(|n| n.has_tag_name("node")) {
            let attr = |name: &str| child.attribute(name).filter(|v| !v.is_empty()).map(str::to_owned);
            if window.name.is_empty() {
                window.name = attr("package").unwrap_or_default();
            }
            let bounds = attr("bounds").and_then(|b| parse_bounds(&b)).unwrap_or_default();
            let mut attributes = vec![("bounds".to_owned(), format!("{}, {}, {}, {}", bounds.left, bounds.top, bounds.right, bounds.bottom)), ("size".to_owned(), format!("{} × {}", bounds.width(), bounds.height()))];
            for name in ["text", "content-desc", "package", "checkable", "checked", "clickable", "enabled", "focusable", "focused", "scrollable", "long-clickable", "password", "selected"] {
                if let Some(value) = attr(name) {
                    attributes.push((name.to_owned(), value));
                }
            }
            window.nodes.push(ViewNode {
                class: attr("class").unwrap_or_else(|| "View".to_owned()),
                id: attr("resource-id"),
                text: attr("text").or_else(|| attr("content-desc")),
                bounds,
                visible: true,
                depth,
                attributes,
            });
            walk(child, depth + 1, window);
        }
    }
    walk(root, 0, &mut window);
    (!window.nodes.is_empty()).then_some(window)
}

/// The screen's size from a PNG's header.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || !png.starts_with(b"\x89PNG") {
        return None;
    }
    let be = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
    Some((be(16), be(20)))
}

/// The 3D view's camera: each view a layer `depth × spacing` device
/// pixels in front of the screen, turned `yaw` radians around the vertical
/// axis and `pitch` around the horizontal one, seen without perspective.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rotation {
    pub yaw: f32,
    pub pitch: f32,
    pub spacing: f32,
}

impl Rotation {
    /// How Android Studio turns the layers when 3D mode opens.
    pub const START: Rotation = Rotation { yaw: -0.6, pitch: 0.25, spacing: 60. };

    /// A device point on layer `depth` of a `width × height` screen, as
    /// seen: centered on the screen's middle.
    pub fn project(&self, x: f32, y: f32, depth: usize, width: f32, height: f32) -> (f32, f32) {
        let (x, y, z) = (x - width / 2., y - height / 2., depth as f32 * self.spacing);
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let x1 = x * cy + z * sy;
        let z1 = -x * sy + z * cy;
        (x1, y * cp - z1 * sp)
    }

    /// A view's four corners on its layer, clockwise from top left.
    pub fn corners(&self, r: &Rect, depth: usize, width: f32, height: f32) -> [(f32, f32); 4] {
        let (l, t, rt, b) = (r.left as f32, r.top as f32, r.right as f32, r.bottom as f32);
        [self.project(l, t, depth, width, height), self.project(rt, t, depth, width, height), self.project(rt, b, depth, width, height), self.project(l, b, depth, width, height)]
    }
}

/// Whether `p` is inside the convex quadrilateral `q`.
pub fn quad_contains(q: &[(f32, f32); 4], p: (f32, f32)) -> bool {
    let mut sign = 0.;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        if cross != 0. {
            if sign != 0. && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    true
}

/// Where the snapshot comes from.
pub trait LayoutSource {
    /// `dumpsys activity top`'s text, or the `uiautomator dump` XML.
    fn hierarchy(&self, serial: &str, uiautomator: bool) -> Result<String, String>;
    fn screenshot(&self, serial: &str) -> Result<Vec<u8>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_layers() {
        let flat = Rotation { yaw: 0., pitch: 0., spacing: 50. };
        assert_eq!(flat.project(100., 50., 3, 200., 100.), (0., 0.));
        let r = Rect { left: 0, top: 0, right: 200, bottom: 100 };
        assert_eq!(flat.corners(&r, 0, 200., 100.), [(-100., -50.), (100., -50.), (100., 50.), (-100., 50.)]);
        // Turned a quarter: a layer shows edge on, deeper layers further right.
        let side = Rotation { yaw: std::f32::consts::FRAC_PI_2, pitch: 0., spacing: 50. };
        let (x0, _) = side.project(0., 0., 0, 200., 100.);
        let (x1, _) = side.project(0., 0., 1, 200., 100.);
        assert!(x0.abs() < 1e-3 && (x1 - 50.).abs() < 1e-3);
        let q = Rotation::START.corners(&r, 2, 200., 100.);
        let middle = Rotation::START.project(100., 50., 2, 200., 100.);
        assert!(quad_contains(&q, middle));
        assert!(!quad_contains(&q, (middle.0 + 500., middle.1)));
    }

    const DUMPSYS: &str = "TASK 1:com.android.launcher3 id=2 userId=0
  ACTIVITY com.android.launcher3/.Launcher 1a2b pid=900
    Local Activity 1 State:
      mResumed=false mStopped=true mFinished=false
    View Hierarchy:
      DecorView@aa1[Launcher]
        android.widget.FrameLayout{1 V.E...... ........ 0,0-1080,2400}
    Looper (main, tid 2) {1}
TASK 10:com.example id=45 userId=0
  ACTIVITY com.example/.MainActivity 3c4d pid=12345
    Local Activity 9f State:
      mResumed=true mStopped=false mFinished=false
    View Hierarchy:
      DecorView@5c3b2a[MainActivity]
        android.widget.LinearLayout{8d2e1f V.E...... ........ 0,0-1080,2400}
          android.view.ViewStub{c1a5b0 G.E...... ......I. 0,0-0,0 #10201b0 android:id/action_mode_bar_stub}
          android.widget.FrameLayout{a4c6d29 V.E...... ........ 0,100-1080,2337 #1020002 android:id/content}
            androidx.appcompat.widget.AppCompatTextView{5678 VFED..CL. .F...... 42,10-300,60 #7f0800a1 app:id/title}
    Looper (main, tid 2) {1}
";

    #[test]
    fn dumpsys_views_with_screen_bounds() {
        let windows = parse_dumpsys(DUMPSYS);
        assert_eq!(windows.len(), 2);
        let w = &windows[0];
        assert_eq!(w.name, "com.example/.MainActivity");
        let classes: Vec<(&str, usize)> = w.nodes.iter().map(|n| (n.short_class(), n.depth)).collect();
        assert_eq!(classes, [("DecorView", 0), ("LinearLayout", 1), ("ViewStub", 2), ("FrameLayout", 2), ("AppCompatTextView", 3)]);
        let text = &w.nodes[4];
        assert_eq!(text.bounds, Rect { left: 42, top: 110, right: 300, bottom: 160 });
        assert_eq!(text.id_name(), Some("title"));
        assert!(text.attributes.contains(&("clickable".into(), "true".into())));
        assert!(text.attributes.contains(&("focused".into(), "true".into())));
        assert!(!w.nodes[2].visible);
        assert_eq!(w.nodes[0].bounds, Rect { left: 0, top: 0, right: 1080, bottom: 2400 });
        assert_eq!(w.subtree(1), 2..5);
        assert_eq!(w.parent(4), Some(3));
        assert_eq!(w.view_at(50, 120), Some(4));
        assert_eq!(w.view_at(500, 50), Some(1));
    }

    #[test]
    fn uiautomator_nodes() {
        let xml = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?><hierarchy rotation="0"><node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.example" content-desc="" clickable="false" enabled="true" bounds="[0,0][1080,2400]"><node index="0" text="Hello" resource-id="com.example:id/title" class="android.widget.TextView" package="com.example" content-desc="" clickable="true" enabled="true" bounds="[42,110][300,160]" /></node></hierarchy>UI hierchary dumped to: /dev/tty"#;
        let w = parse_uiautomator(xml).unwrap();
        assert_eq!(w.name, "com.example");
        assert_eq!(w.nodes.len(), 2);
        assert_eq!(w.nodes[1].text.as_deref(), Some("Hello"));
        assert_eq!(w.nodes[1].bounds, Rect { left: 42, top: 110, right: 300, bottom: 160 });
        assert_eq!(w.nodes[1].depth, 1);
        assert_eq!(w.view_at(100, 120), Some(1));
    }

    #[test]
    fn png_header_size() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(1080u32.to_be_bytes());
        png.extend(2400u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((1080, 2400)));
    }
}
