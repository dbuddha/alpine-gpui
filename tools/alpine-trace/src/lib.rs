//! Typed, fail-closed decoding for `alpine-scene-trace/v1` workloads.
//!
//! Serialization stays in the non-shipping assurance boundary. This crate owns
//! the dependency-free semantic conversion shared by Alpine and its isolated
//! comparison lab.

use std::{error::Error, fmt};

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_metal::{OffscreenDescriptor, OffscreenError, ValidatedFrame};
use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};

#[cfg(kani)]
mod proofs;

/// Maximum operations accepted by one decoded workload.
pub const MAX_TRACE_OPERATIONS: usize = 65_536;

/// Raw logical and physical target identity from a scene trace.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceViewport {
    /// Logical width in pixels.
    pub logical_width: f32,
    /// Logical height in pixels.
    pub logical_height: f32,
    /// Logical-to-physical scale.
    pub scale_factor: f32,
    /// Explicit physical width in pixels.
    pub pixel_width: u32,
    /// Explicit physical height in pixels.
    pub pixel_height: u32,
    /// Linear unpremultiplied clear color.
    pub clear_color: [f32; 4],
}

/// One named clip resolved by the serialization boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceClip {
    /// Clip origin and extent as `[x, y, width, height]`.
    pub bounds: [f32; 4],
}

/// One solid quad operation in painter order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceQuad {
    /// Zero-based operation sequence.
    pub sequence: u64,
    /// Quad origin and extent as `[x, y, width, height]`.
    pub bounds: [f32; 4],
    /// Linear unpremultiplied `[red, green, blue, alpha]`.
    pub color: [f32; 4],
    /// Resolved operation clip.
    pub clip: TraceClip,
}

/// A serialization-neutral `alpine-scene-trace/v1` workload.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceInput {
    /// Persisted scene revision.
    pub revision: u64,
    /// Exact logical and physical target identity.
    pub viewport: TraceViewport,
    /// Operations in declared painter order.
    pub quads: Vec<TraceQuad>,
}

/// A trace decoded into Alpine-owned renderer inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedTrace {
    scene: Scene,
    descriptor: OffscreenDescriptor,
}

impl DecodedTrace {
    /// Returns the immutable scene.
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Returns the exact offscreen target.
    #[must_use]
    pub const fn descriptor(&self) -> OffscreenDescriptor {
        self.descriptor
    }

    /// Revalidates and lowers the decoded scene for renderer consumption.
    ///
    /// # Errors
    ///
    /// Returns the underlying frame validation error when the decoded scene
    /// cannot be represented by the Direct Metal contract.
    pub fn validated_frame(&self) -> Result<ValidatedFrame, OffscreenError> {
        ValidatedFrame::new(&self.scene, self.descriptor)
    }
}

/// Fail-closed semantic decoding errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceDecodeError {
    /// Scene revision zero is reserved and cannot identify a workload.
    ZeroRevision,
    /// The logical viewport is empty or non-finite.
    InvalidLogicalViewport,
    /// The clear color is non-finite or outside the normalized range.
    InvalidClearColor,
    /// The physical target is empty or has an invalid scale.
    InvalidPhysicalTarget,
    /// Explicit physical dimensions disagree with the rounding contract.
    PhysicalViewportMismatch,
    /// The trace exceeds the decoder's explicit operation bound.
    TooManyOperations,
    /// An operation sequence is not contiguous from zero.
    NoncontiguousSequence {
        /// Required zero-based sequence.
        expected: usize,
        /// Sequence encoded by the trace.
        actual: u64,
    },
    /// A quad contains invalid geometry.
    InvalidQuadBounds {
        /// Sequence of the rejected operation.
        sequence: u64,
    },
    /// A quad contains an invalid color.
    InvalidQuadColor {
        /// Sequence of the rejected operation.
        sequence: u64,
    },
    /// The current protocol slice supports only the full viewport clip.
    UnsupportedClip {
        /// Sequence of the rejected operation.
        sequence: u64,
    },
}

impl fmt::Display for TraceDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroRevision => formatter.write_str("trace revision must be positive"),
            Self::InvalidLogicalViewport => {
                formatter.write_str("trace logical viewport must be finite and positive")
            }
            Self::InvalidClearColor => {
                formatter.write_str("trace clear color must contain normalized finite channels")
            }
            Self::InvalidPhysicalTarget => {
                formatter.write_str("trace physical target and scale must be valid")
            }
            Self::PhysicalViewportMismatch => formatter.write_str(
                "trace physical target must equal rounded logical size multiplied by scale",
            ),
            Self::TooManyOperations => formatter.write_str("trace operation limit exceeded"),
            Self::NoncontiguousSequence { expected, actual } => write!(
                formatter,
                "trace operation sequence must be contiguous: expected {expected}, got {actual}"
            ),
            Self::InvalidQuadBounds { sequence } => {
                write!(formatter, "trace quad {sequence} has invalid bounds")
            }
            Self::InvalidQuadColor { sequence } => {
                write!(formatter, "trace quad {sequence} has invalid color")
            }
            Self::UnsupportedClip { sequence } => write!(
                formatter,
                "trace quad {sequence} uses a clip unsupported by this protocol slice"
            ),
        }
    }
}

impl Error for TraceDecodeError {}

impl TraceInput {
    /// Decodes the workload into an immutable Alpine scene and exact target.
    ///
    /// Only the full viewport clip is supported by the current solid-quad scene
    /// contract. A narrower or translated clip fails instead of silently
    /// changing pixels.
    ///
    /// # Errors
    ///
    /// Returns a stage-specific error for invalid identity, target geometry,
    /// operation ordering, bounds, colors, clips, or capacity.
    pub fn decode(self) -> Result<DecodedTrace, TraceDecodeError> {
        if self.revision == 0 {
            return Err(TraceDecodeError::ZeroRevision);
        }
        let logical_size = Size::new(self.viewport.logical_width, self.viewport.logical_height)
            .filter(|size| !size.is_empty())
            .ok_or(TraceDecodeError::InvalidLogicalViewport)?;
        let clear =
            decode_color(self.viewport.clear_color).ok_or(TraceDecodeError::InvalidClearColor)?;
        let descriptor = OffscreenDescriptor::new(
            self.viewport.pixel_width,
            self.viewport.pixel_height,
            self.viewport.scale_factor,
            clear,
        )
        .map_err(|_| TraceDecodeError::InvalidPhysicalTarget)?;
        if !physical_matches(
            self.viewport.logical_width,
            self.viewport.scale_factor,
            self.viewport.pixel_width,
        ) || !physical_matches(
            self.viewport.logical_height,
            self.viewport.scale_factor,
            self.viewport.pixel_height,
        ) {
            return Err(TraceDecodeError::PhysicalViewportMismatch);
        }
        if self.quads.len() > MAX_TRACE_OPERATIONS {
            return Err(TraceDecodeError::TooManyOperations);
        }

        let viewport_bounds = [
            0.0,
            0.0,
            self.viewport.logical_width,
            self.viewport.logical_height,
        ];
        let mut builder = SceneBuilder::new(SceneRevision::new(self.revision), logical_size);
        for (expected, quad) in self.quads.into_iter().enumerate() {
            if quad.sequence != expected as u64 {
                return Err(TraceDecodeError::NoncontiguousSequence {
                    expected,
                    actual: quad.sequence,
                });
            }
            if !float_arrays_match(quad.clip.bounds, viewport_bounds) {
                return Err(TraceDecodeError::UnsupportedClip {
                    sequence: quad.sequence,
                });
            }
            let bounds = decode_rect(quad.bounds).ok_or(TraceDecodeError::InvalidQuadBounds {
                sequence: quad.sequence,
            })?;
            let color = decode_color(quad.color).ok_or(TraceDecodeError::InvalidQuadColor {
                sequence: quad.sequence,
            })?;
            builder.push(Primitive::Quad { bounds, color });
        }

        Ok(DecodedTrace {
            scene: builder.finish(),
            descriptor,
        })
    }
}

fn decode_rect(values: [f32; 4]) -> Option<Rect> {
    let origin = Point::new(values[0], values[1])?;
    let size = Size::new(values[2], values[3])?;
    (!size.is_empty()).then_some(Rect::new(origin, size))
}

fn decode_color(values: [f32; 4]) -> Option<LinearRgba> {
    LinearRgba::new(values[0], values[1], values[2], values[3])
}

fn physical_matches(logical: f32, scale: f32, pixel: u32) -> bool {
    let physical = f64::from(logical) * f64::from(scale);
    let expected = f64::from(pixel);
    logical > 0.0
        && scale > 0.0
        && physical.is_finite()
        && physical >= expected - 0.5
        && physical < expected + 0.5
}

fn float_arrays_match(left: [f32; 4], right: [f32; 4]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| left.to_bits() == right.to_bits())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_TRACE_OPERATIONS, TraceClip, TraceDecodeError, TraceInput, TraceQuad, TraceViewport,
        physical_matches,
    };

    fn viewport() -> TraceViewport {
        TraceViewport {
            logical_width: 4.0,
            logical_height: 2.0,
            scale_factor: 2.0,
            pixel_width: 8,
            pixel_height: 4,
            clear_color: [0.0, 0.0, 0.0, 0.0],
        }
    }

    fn quad(sequence: u64) -> TraceQuad {
        TraceQuad {
            sequence,
            bounds: [1.0, 0.0, 2.0, 2.0],
            color: [1.0, 0.0, 0.0, 0.5],
            clip: TraceClip {
                bounds: [0.0, 0.0, 4.0, 2.0],
            },
        }
    }

    #[test]
    fn decodes_exact_target_and_preserves_painter_order() {
        let decoded = TraceInput {
            revision: 9,
            viewport: viewport(),
            quads: vec![quad(0), quad(1)],
        }
        .decode();
        assert_eq!(
            decoded.as_ref().map(|decoded| (
                decoded.scene().revision().get(),
                decoded.scene().operation_count(),
                decoded.descriptor().pixel_width(),
                decoded.descriptor().pixel_height(),
                decoded.descriptor().scale().to_bits(),
            )),
            Ok((9, 2, 8, 4, 2.0_f32.to_bits()))
        );
        assert_eq!(
            decoded.as_ref().map(|decoded| {
                decoded
                    .validated_frame()
                    .map(|frame| (frame.consumed_primitives(), frame.omitted_primitives()))
            }),
            Ok(Ok((2, 0)))
        );
    }

    #[test]
    fn rejects_every_identity_and_target_mismatch() {
        let mut input = TraceInput {
            revision: 0,
            viewport: viewport(),
            quads: vec![quad(0)],
        };
        assert_eq!(input.clone().decode(), Err(TraceDecodeError::ZeroRevision));
        input.revision = 1;
        input.viewport.logical_width = 0.0;
        assert_eq!(
            input.clone().decode(),
            Err(TraceDecodeError::InvalidLogicalViewport)
        );
        input.viewport.logical_width = 4.0;
        input.viewport.clear_color[3] = 1.5;
        assert_eq!(
            input.clone().decode(),
            Err(TraceDecodeError::InvalidClearColor)
        );
        input.viewport.clear_color[3] = 0.0;
        input.viewport.pixel_width = 0;
        assert_eq!(
            input.clone().decode(),
            Err(TraceDecodeError::InvalidPhysicalTarget)
        );
        input.viewport.pixel_width = 7;
        assert_eq!(
            input.decode(),
            Err(TraceDecodeError::PhysicalViewportMismatch)
        );
    }

    #[test]
    fn rejects_sequence_geometry_color_and_clip_breaks() {
        let base = TraceInput {
            revision: 1,
            viewport: viewport(),
            quads: vec![quad(1)],
        };
        assert_eq!(
            base.clone().decode(),
            Err(TraceDecodeError::NoncontiguousSequence {
                expected: 0,
                actual: 1,
            })
        );

        let mut invalid = quad(0);
        invalid.bounds[2] = -1.0;
        assert_eq!(
            TraceInput {
                quads: vec![invalid],
                ..base.clone()
            }
            .decode(),
            Err(TraceDecodeError::InvalidQuadBounds { sequence: 0 })
        );
        invalid = quad(0);
        invalid.color[0] = f32::NAN;
        assert_eq!(
            TraceInput {
                quads: vec![invalid],
                ..base.clone()
            }
            .decode(),
            Err(TraceDecodeError::InvalidQuadColor { sequence: 0 })
        );
        invalid = quad(0);
        invalid.clip.bounds[2] = 3.0;
        assert_eq!(
            TraceInput {
                quads: vec![invalid],
                ..base
            }
            .decode(),
            Err(TraceDecodeError::UnsupportedClip { sequence: 0 })
        );
    }

    #[test]
    fn operation_limit_is_explicit() {
        let quads = (0..MAX_TRACE_OPERATIONS)
            .map(|sequence| quad(sequence as u64))
            .collect::<Vec<_>>();
        assert!(
            TraceInput {
                revision: 1,
                viewport: viewport(),
                quads: quads.clone(),
            }
            .decode()
            .is_ok()
        );
        let mut too_many = quads;
        too_many.push(quad(MAX_TRACE_OPERATIONS as u64));
        assert_eq!(
            TraceInput {
                revision: 1,
                viewport: viewport(),
                quads: too_many,
            }
            .decode(),
            Err(TraceDecodeError::TooManyOperations)
        );
    }

    #[test]
    fn physical_rounding_checks_every_precondition_and_half_boundary() {
        assert!(!physical_matches(f32::NAN, 1.0, 1));
        assert!(!physical_matches(1.0, f32::INFINITY, 1));
        assert!(!physical_matches(0.0, 1.0, 1));
        assert!(!physical_matches(1.0, 0.0, 1));
        assert!(!physical_matches(0.0, 1.0, 0));
        assert!(!physical_matches(1.0, 0.0, 0));
        assert!(physical_matches(1.5, 1.0, 2));
        assert!(!physical_matches(2.5, 1.0, 2));
    }

    #[test]
    fn errors_expose_stable_stage_specific_messages() {
        let cases = [
            (
                TraceDecodeError::ZeroRevision,
                "trace revision must be positive",
            ),
            (
                TraceDecodeError::InvalidLogicalViewport,
                "trace logical viewport must be finite and positive",
            ),
            (
                TraceDecodeError::InvalidClearColor,
                "trace clear color must contain normalized finite channels",
            ),
            (
                TraceDecodeError::InvalidPhysicalTarget,
                "trace physical target and scale must be valid",
            ),
            (
                TraceDecodeError::PhysicalViewportMismatch,
                "trace physical target must equal rounded logical size multiplied by scale",
            ),
            (
                TraceDecodeError::TooManyOperations,
                "trace operation limit exceeded",
            ),
            (
                TraceDecodeError::NoncontiguousSequence {
                    expected: 2,
                    actual: 4,
                },
                "trace operation sequence must be contiguous: expected 2, got 4",
            ),
            (
                TraceDecodeError::InvalidQuadBounds { sequence: 3 },
                "trace quad 3 has invalid bounds",
            ),
            (
                TraceDecodeError::InvalidQuadColor { sequence: 5 },
                "trace quad 5 has invalid color",
            ),
            (
                TraceDecodeError::UnsupportedClip { sequence: 7 },
                "trace quad 7 uses a clip unsupported by this protocol slice",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message);
        }
    }
}
