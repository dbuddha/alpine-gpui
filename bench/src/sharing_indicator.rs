//! Evidence for the capture-owned title-bar indicator of the pinned fixture.

use crate::helpers::WindowInfo;
const BASE: &str = "NSLocalWindowSharingWindow";
const KVO: &str = "NSKVONotifying_NSLocalWindowSharingWindow";
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect([f64; 4]);
impl Rect {
    fn window(w: &WindowInfo) -> Self {
        Self([w.x, w.y, w.width, w.height])
    }
    fn parse(text: &str) -> Result<Self> {
        let mut values = text.split(',');
        let mut result = [0.0; 4];
        for value in &mut result {
            *value = values
                .next()
                .ok_or("short rectangle")?
                .parse::<f64>()
                .map_err(|_| "invalid rectangle")?;
        }
        let [x, y, w, h] = result;
        if values.next().is_some()
            || !result.iter().all(|v| v.is_finite())
            || x.abs() > 1_000_000.0
            || y.abs() > 1_000_000.0
            || !(0.0..=65_536.0).contains(&w)
            || !(0.0..=65_536.0).contains(&h)
            || w == 0.0
            || h == 0.0
        {
            return Err("unbounded rectangle".into());
        }
        Ok(Self(result))
    }
    fn contains(self, other: Self) -> bool {
        let [left, top, width, height] = self.0;
        let [x, y, inner_width, inner_height] = other.0;
        x >= left && y >= top && x + inner_width <= left + width && y + inner_height <= top + height
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Proof {
    child: u32,
    frame: Rect,
    content: Rect,
    indicator: Rect,
}
impl Proof {
    pub(crate) fn read(line: &str, seq: u8, main: &WindowInfo, child: &WindowInfo) -> Result<Self> {
        if line.len() > 512 {
            return Err("oversize inspection reply".into());
        }
        let parts: Vec<_> = line.split('\t').collect();
        if parts.len() != 12
            || parts[0] != "scene-sharing"
            || parts[1] != seq.to_string()
            || parts[2] != main.pid.to_string()
            || parts[3] != main.id.to_string()
            || parts[4] != child.id.to_string()
            || parts[10] != BASE
            || !matches!((parts[5], parts[11]), (BASE, "exact") | (KVO, "kvo-direct"))
            || parts[6] != main.id.to_string()
        {
            return Err("unresolved or mismatched indicator identity".into());
        }
        let frame = Rect::parse(parts[7])?;
        let content = Rect::parse(parts[8])?;
        let indicator = Rect::parse(parts[9])?;
        let [x, y, w, h] = frame.0;
        let [_, content_y, cw, ch] = content.0;
        let [_, indicator_y, _, ih] = indicator.0;
        let sample = Rect([x + (w - 500.0) / 2.0, y + (h - 40.0) / 2.0, 500.0, 40.0]);
        if frame != Rect::window(main)
            || indicator != Rect::window(child)
            || [cw.to_bits(), ch.to_bits()] != [500.0f64.to_bits(), 300.0f64.to_bits()]
            || !frame.contains(content)
            || !frame.contains(indicator)
            || !content.contains(sample)
            || indicator_y + ih > content_y
        {
            return Err("indicator geometry does not prove title-bar containment".into());
        }
        Ok(Self {
            child: child.id,
            frame,
            content,
            indicator,
        })
    }
}
