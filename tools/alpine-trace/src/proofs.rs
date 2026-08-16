use super::{TraceClip, TraceInput, TraceQuad, TraceViewport};

#[kani::proof]
fn bounded_trace_preserves_operation_order_and_values() {
    let width = 2_u16;
    let height = 2_u16;
    let red = if kani::any::<bool>() { 1.0 } else { 0.0 };
    let alpha = if kani::any::<bool>() { 1.0 } else { 0.0 };
    let viewport = TraceViewport {
        logical_width: f32::from(width),
        logical_height: f32::from(height),
        scale_factor: 1.0,
        pixel_width: u32::from(width),
        pixel_height: u32::from(height),
        clear_color: [0.0, 0.0, 0.0, 0.0],
    };
    let first = TraceQuad {
        sequence: 0,
        bounds: [0.0, 0.0, f32::from(width), f32::from(height)],
        color: [red, 0.0, 0.0, alpha],
        clip: TraceClip {
            bounds: [0.0, 0.0, f32::from(width), f32::from(height)],
        },
    };
    let second = TraceQuad {
        sequence: 1,
        color: [0.0, red, 0.0, alpha],
        ..first
    };
    let decoded = TraceInput {
        revision: 1,
        viewport,
        quads: vec![first, second],
    }
    .decode();

    assert!(decoded.is_ok());
    if let Ok(decoded) = decoded {
        assert_eq!(decoded.scene().operation_count(), 2);
        assert_eq!(decoded.descriptor().pixel_width(), u32::from(width));
        assert_eq!(decoded.descriptor().pixel_height(), u32::from(height));
        let quad = decoded.scene().quads()[0];
        let bounds = quad.bounds();
        let color = quad.color();
        assert_eq!(bounds.origin().x().to_bits(), 0.0_f32.to_bits());
        assert_eq!(bounds.origin().y().to_bits(), 0.0_f32.to_bits());
        assert_eq!(bounds.size().width().to_bits(), f32::from(width).to_bits());
        assert_eq!(
            bounds.size().height().to_bits(),
            f32::from(height).to_bits()
        );
        assert_eq!(color.red().to_bits(), red.to_bits());
        assert_eq!(color.alpha().to_bits(), alpha.to_bits());
        kani::cover!(red == 0.0 && alpha == 0.0, "transparent boundary");
        kani::cover!(red == 1.0 && alpha == 1.0, "opaque boundary");
    }
}

#[kani::proof]
fn bounded_trace_rejects_noncontiguous_index() {
    let sequence = u64::from(kani::any::<u8>()) + 1;
    let input = TraceInput {
        revision: 1,
        viewport: TraceViewport {
            logical_width: 1.0,
            logical_height: 1.0,
            scale_factor: 1.0,
            pixel_width: 1,
            pixel_height: 1,
            clear_color: [0.0, 0.0, 0.0, 0.0],
        },
        quads: vec![TraceQuad {
            sequence,
            bounds: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 1.0, 1.0, 1.0],
            clip: TraceClip {
                bounds: [0.0, 0.0, 1.0, 1.0],
            },
        }],
    };

    assert!(input.decode().is_err());
    kani::cover!(sequence == 1, "nearest invalid sequence");
    kani::cover!(sequence == 256, "bounded maximum invalid sequence");
}
