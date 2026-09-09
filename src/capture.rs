/// Use one explicitly selected stream, so motion cannot be counted twice.
/// Winit/X11 also labels absolute XInput valuators as MouseMotion: receiving
/// that event alone does not establish that a device supplies relative input.
/// Absolute remote-desktop input is differenced against its previous position,
/// never a warped window center (which is not the remote mouse's origin).
#[derive(Default)]
pub struct CaptureMotion {
    enabled: bool,
    use_raw: bool,
    position: Option<(f64, f64)>,
}

impl CaptureMotion {
    pub fn new(use_raw: bool) -> Self {
        Self {
            use_raw,
            ..Self::default()
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.position = None;
            crate::input_log::write(format_args!(
                "capture enabled={enabled} source={}",
                if self.use_raw {
                    "raw"
                } else {
                    "cursor-position"
                }
            ));
        }
    }

    pub fn uses_raw(&self) -> bool {
        self.use_raw
    }

    pub fn raw(&mut self, dx: f64, dy: f64) -> Option<(f64, f64)> {
        (self.enabled && self.use_raw).then_some((dx, dy))
    }

    pub fn position(&mut self, x: f64, y: f64) -> Option<(f64, f64)> {
        let previous = self.position.replace((x, y));
        if !self.enabled || self.use_raw {
            return None;
        }
        previous
            .map(|(px, py)| (x - px, y - py))
            .filter(|delta| *delta != (0.0, 0.0))
    }

    pub fn reset_position(&mut self) {
        self.position = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_capture_has_no_initial_jump_and_keeps_sending() {
        let mut motion = CaptureMotion::default();
        motion.position(10.0, 20.0);
        motion.set_enabled(true);
        assert_eq!(motion.position(900.0, 600.0), None);
        assert_eq!(motion.position(910.0, 590.0), Some((10.0, -10.0)));
        assert_eq!(motion.position(915.0, 580.0), Some((5.0, -10.0)));
        motion.reset_position();
        assert_eq!(motion.position(100.0, 100.0), None);
    }

    #[test]
    fn physical_motion_is_not_counted_twice() {
        let mut motion = CaptureMotion::new(true);
        motion.raw(1.0, 1.0);
        motion.set_enabled(true);
        assert_eq!(motion.raw(2.0, -3.0), Some((2.0, -3.0)));
        assert_eq!(motion.position(100.0, 100.0), None);
        assert_eq!(motion.position(102.0, 97.0), None);
        motion.set_enabled(false);
        assert_eq!(motion.raw(2.0, 3.0), None);
    }
}
