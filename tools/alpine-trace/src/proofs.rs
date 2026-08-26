use super::{
    PreparedTraceInput, PreparedTraceOperation, PreparedTraceQuad, TraceClip, TraceInput,
    TraceQuad, TraceSequenceAtlas, TraceSequenceInput, TraceSequenceStep, TraceSequenceTransition,
    TraceViewport,
};

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
    if let Ok(decoded) = &decoded {
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
    // This proof owns successful protocol admission and value preservation,
    // not destruction of its dynamically sized scene fixture. Kani otherwise
    // spends an unbounded proof budget unwinding the Arc-backed atlas patches;
    // normal tests and Miri retain teardown coverage.
    std::mem::forget(decoded);
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

    let decoded = input.decode();
    assert!(decoded.is_err());
    kani::cover!(sequence == 1, "nearest invalid sequence");
    kani::cover!(sequence == 256, "bounded maximum invalid sequence");
    // This proof owns sequence admission, not destruction of the unreachable
    // success payload. Forgetting the fixture prevents Kani from unwinding
    // dynamically sized scene resources that no concrete error path owns;
    // normal tests and Miri retain teardown coverage.
    std::mem::forget(decoded);
}

#[kani::proof]
fn prepared_trace_rejects_every_bounded_invalid_clip_reference() {
    let invalid_clip = if kani::any::<bool>() { 1 } else { usize::MAX };
    let input = PreparedTraceInput {
        revision: 1,
        viewport: TraceViewport {
            logical_width: 1.0,
            logical_height: 1.0,
            scale_factor: 1.0,
            pixel_width: 1,
            pixel_height: 1,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        clips: vec![TraceClip {
            bounds: [0.0, 0.0, 1.0, 1.0],
        }],
        atlas: None,
        operations: vec![PreparedTraceOperation::Quad(PreparedTraceQuad {
            sequence: 0,
            bounds: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 1.0, 1.0, 1.0],
            clip: Some(invalid_clip),
        })],
    };
    let decoded = input.decode();
    assert!(decoded.is_err());
    kani::cover!(invalid_clip == 1, "nearest invalid clip index");
    kani::cover!(invalid_clip == usize::MAX, "maximum invalid clip index");
    std::mem::forget(decoded);
}

#[kani::proof]
fn lifecycle_sequence_rejects_symbolic_incompatible_reuse() {
    let faulty_reuse = kani::any::<bool>();
    let initial = TraceSequenceAtlas {
        identity: 1,
        revision: 1,
        width: 2,
        height: 2,
        content_hash: [1; 32],
    };
    let reused = TraceSequenceAtlas {
        content_hash: if faulty_reuse { [9; 32] } else { [1; 32] },
        ..initial
    };
    let content = TraceSequenceAtlas {
        revision: 2,
        content_hash: [2; 32],
        ..initial
    };
    let capacity = TraceSequenceAtlas {
        revision: 3,
        width: 4,
        content_hash: [3; 32],
        ..content
    };
    let input = TraceSequenceInput {
        steps: vec![
            TraceSequenceStep {
                sequence: 0,
                transition: TraceSequenceTransition::FullAdmission,
                workload_hash: Some([1; 32]),
                renderer_generation: 1,
                atlas: Some(initial),
                expected_atlas_upload_bytes: 4,
                expected_terminal_retained_bytes: 0,
            },
            TraceSequenceStep {
                sequence: 1,
                transition: TraceSequenceTransition::CompatibleReuse,
                workload_hash: Some([1; 32]),
                renderer_generation: 1,
                atlas: Some(reused),
                expected_atlas_upload_bytes: 0,
                expected_terminal_retained_bytes: 0,
            },
            TraceSequenceStep {
                sequence: 2,
                transition: TraceSequenceTransition::ContentReplacement,
                workload_hash: Some([2; 32]),
                renderer_generation: 1,
                atlas: Some(content),
                expected_atlas_upload_bytes: 4,
                expected_terminal_retained_bytes: 0,
            },
            TraceSequenceStep {
                sequence: 3,
                transition: TraceSequenceTransition::CapacityReplacement,
                workload_hash: Some([3; 32]),
                renderer_generation: 1,
                atlas: Some(capacity),
                expected_atlas_upload_bytes: 8,
                expected_terminal_retained_bytes: 0,
            },
            TraceSequenceStep {
                sequence: 4,
                transition: TraceSequenceTransition::Teardown,
                workload_hash: None,
                renderer_generation: 1,
                atlas: None,
                expected_atlas_upload_bytes: 0,
                expected_terminal_retained_bytes: 0,
            },
            TraceSequenceStep {
                sequence: 5,
                transition: TraceSequenceTransition::FullResynchronization,
                workload_hash: Some([3; 32]),
                renderer_generation: 2,
                atlas: Some(capacity),
                expected_atlas_upload_bytes: 8,
                expected_terminal_retained_bytes: 0,
            },
        ],
    };
    assert_eq!(input.validate().is_err(), faulty_reuse);
    kani::cover!(faulty_reuse, "incompatible reuse is rejected");
    kani::cover!(!faulty_reuse, "exact compatible reuse is accepted");
    std::mem::forget(input);
}
