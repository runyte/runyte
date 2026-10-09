// SPDX-License-Identifier: MPL-2.0
//! A bounded document-space cache around the visible viewport, without changing layout.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub origin: [f64; 2],
}
impl Raster {
    pub fn new(width: u32, height: u32, scale: f64, scroll: [f64; 2], adjacent: bool) -> Self {
        let dimensions = |fraction: f64| {
            let x = (f64::from(width) * 0.25 * fraction) as u32;
            let y = (f64::from(height) * fraction) as u32;
            (width + 2 * x, height + 2 * y, x, y)
        };
        let (mut lo, mut hi) = (0.0, if adjacent { 1.0 } else { 0.0 });
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            let (w, h, _, _) = dimensions(mid);
            if w <= 8192 && h <= 8192 && u64::from(w) * u64::from(h) <= 8_000_000 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (width, height, x, y) = dimensions(lo);
        Self {
            width,
            height,
            origin: [
                (scroll[0] - f64::from(x) / scale).max(0.0),
                (scroll[1] - f64::from(y) / scale).max(0.0),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Raster;
    #[test]
    fn adjacent_content_fits_budget_without_changing_viewport_resolution() {
        for (w, h) in [(1080, 800), (2048, 1900), (4096, 900), (900, 4096)] {
            let r = Raster::new(w, h, 1.5, [1000.0, 2000.0], true);
            assert!(r.width >= w && r.height > h);
            assert!(r.width <= 8192 && r.height <= 8192);
            assert!(u64::from(r.width) * u64::from(r.height) <= 8_000_000);
            assert!(r.origin[0] < 1000.0 && r.origin[1] < 2000.0);
            assert!(r.origin[1] + f64::from(r.height) / 1.5 > 2000.0 + f64::from(h) / 1.5);
        }
        let top = Raster::new(1080, 800, 1.0, [0.0, 0.0], true);
        assert_eq!(top.origin, [0.0; 2]);
        assert!(top.height >= 2398);
    }
    #[test]
    fn viewport_anchored_paint_gets_an_exact_raster() {
        let r = Raster::new(1080, 800, 1.0, [20.0, 100.25], false);
        assert_eq!((r.width, r.height, r.origin), (1080, 800, [20.0, 100.25]));
    }
}
