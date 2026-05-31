use adapterlibgfx::api::{Adapter, AdapterConfig};
use adapterlibgfx::vertex::Rgba8;
use adapterlibgfx::window::{FrameProducer, WgpuWindowApp};
use std::time::Instant;

struct Triangle {
    started_at: Instant,
}

impl Default for Triangle {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_rgb(out: &mut Vec<u8>, x: f32, y: f32, color: Rgba8) {
    push_f32(out, x);
    push_f32(out, y);
    out.extend_from_slice(&[color.r, color.g, color.b, color.a]);
}

fn rotate_clockwise(x: f32, y: f32, radians: f32) -> (f32, f32) {
    let (sin, cos) = radians.sin_cos();
    ((x * cos) + (y * sin), (-x * sin) + (y * cos))
}

impl FrameProducer for Triangle {
    fn build_frame(&mut self, adapter: &mut Adapter) {
        let _ = adapter.begin_frame(0x20242A);
        let _ = adapter.set_blend_raw(1, 0x0302, 0x0303);

        let ticks = self.started_at.elapsed().as_millis() / 25;
        let radians = (ticks as f32).to_radians();
        let points = [
            (-0.70, -0.65, Rgba8::new(0xFF, 0x66, 0x33, 0xFF)),
            (0.70, -0.65, Rgba8::new(0x22, 0xCC, 0x88, 0xFF)),
            (0.00, 0.70, Rgba8::new(0x66, 0x99, 0xFF, 0xFF)),
        ];
        let mut vtx = Vec::new();
        for (x, y, color) in points {
            let (x, y) = rotate_clockwise(x, y, radians);
            push_rgb(&mut vtx, x, y, color);
        }

        let _ = adapter.draw_rgb_triangles_no_present(&vtx);
        let _ = adapter.end_frame();
    }
}

fn main() -> Result<(), winit::error::EventLoopError> {
    WgpuWindowApp::new(
        "adapterlibgfx triangle",
        AdapterConfig {
            width: 900,
            height: 560,
        },
        Triangle::default(),
    )
    .run()
}
