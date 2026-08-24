//! Typed, fail-closed decoding for `alpine-scene-trace/v1` workloads.
//!
//! Serialization stays in the non-shipping assurance boundary. This crate owns
//! the dependency-free semantic conversion shared by Alpine and its isolated
//! comparison lab.

use std::{error::Error, fmt, num::NonZeroU32, sync::Arc};

use alpine_core::{LinearRgba, Point, Rect, Size};
use alpine_metal::{OffscreenDescriptor, OffscreenError, ValidatedFrame};
use alpine_scene::{
    AtlasBounds, Clip, Glyph, GlyphAtlasImage, Primitive, Quad, Scene, SceneBuilder, SceneRevision,
};

#[cfg(kani)]
mod proofs;

/// Maximum operations accepted by one decoded workload.
pub const MAX_TRACE_OPERATIONS: usize = 65_536;

/// Maximum clips accepted by one prepared-scene workload.
pub const MAX_TRACE_CLIPS: usize = 4_096;

/// Maximum A8 pixels retained by one prepared-scene workload.
pub const MAX_TRACE_ATLAS_PIXELS: usize = 16_777_216;

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

/// One immutable A8 atlas embedded in a prepared renderer workload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceAtlas {
    /// Positive atlas content revision.
    pub revision: u64,
    /// Atlas width in pixels.
    pub width: u32,
    /// Atlas height in pixels.
    pub height: u32,
    /// Tightly packed top-down A8 pixels.
    pub pixels: Vec<u8>,
}

/// One prepared solid quad with an optional resolved clip index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedTraceQuad {
    /// Zero-based painter-order sequence.
    pub sequence: u64,
    /// Quad origin and extent as `[x, y, width, height]`.
    pub bounds: [f32; 4],
    /// Linear unpremultiplied color.
    pub color: [f32; 4],
    /// Optional index into [`PreparedTraceInput::clips`].
    pub clip: Option<usize>,
}

/// One prepared monochrome glyph with an optional resolved clip index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceGlyph {
    /// Zero-based painter-order sequence.
    pub sequence: u64,
    /// Destination origin and extent as `[x, y, width, height]`.
    pub bounds: [f32; 4],
    /// Integer A8 source origin and extent as `[x, y, width, height]`.
    pub atlas_bounds: [u32; 4],
    /// Linear unpremultiplied color.
    pub color: [f32; 4],
    /// Optional index into [`PreparedTraceInput::clips`].
    pub clip: Option<usize>,
}

/// One painter-ordered operation in a prepared renderer workload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreparedTraceOperation {
    /// Paint one solid quad.
    Quad(PreparedTraceQuad),
    /// Paint one monochrome A8 glyph.
    Glyph(TraceGlyph),
}

impl PreparedTraceOperation {
    const fn sequence(self) -> u64 {
        match self {
            Self::Quad(quad) => quad.sequence,
            Self::Glyph(glyph) => glyph.sequence,
        }
    }
}

/// Serialization-neutral `alpine-scene-trace/v2` prepared-scene semantics.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedTraceInput {
    /// Persisted scene revision.
    pub revision: u64,
    /// Exact logical and physical target identity.
    pub viewport: TraceViewport,
    /// Axis-aligned clips in resolved storage order.
    pub clips: Vec<TraceClip>,
    /// Optional immutable A8 atlas required by glyph operations.
    pub atlas: Option<TraceAtlas>,
    /// Painter-ordered prepared operations.
    pub operations: Vec<PreparedTraceOperation>,
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
    /// The prepared scene exceeds its explicit clip bound.
    TooManyClips,
    /// A prepared clip contains invalid geometry.
    InvalidClipBounds {
        /// Zero-based clip storage index.
        index: usize,
    },
    /// An operation references a clip outside the prepared clip array.
    InvalidClipReference {
        /// Operation sequence.
        sequence: u64,
        /// Rejected clip index.
        index: usize,
    },
    /// The prepared A8 atlas has an invalid revision, extent, length, or size.
    InvalidAtlas,
    /// A glyph operation has no prepared A8 atlas.
    MissingAtlas {
        /// Operation sequence.
        sequence: u64,
    },
    /// A glyph destination contains invalid geometry.
    InvalidGlyphBounds {
        /// Operation sequence.
        sequence: u64,
    },
    /// A glyph source lies outside the prepared A8 atlas.
    InvalidGlyphAtlasBounds {
        /// Operation sequence.
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
            Self::TooManyClips => formatter.write_str("trace clip limit exceeded"),
            Self::InvalidClipBounds { index } => {
                write!(formatter, "trace clip {index} has invalid bounds")
            }
            Self::InvalidClipReference { sequence, index } => write!(
                formatter,
                "trace operation {sequence} references invalid clip index {index}"
            ),
            Self::InvalidAtlas => formatter.write_str("trace A8 atlas is invalid"),
            Self::MissingAtlas { sequence } => {
                write!(formatter, "trace glyph {sequence} requires an A8 atlas")
            }
            Self::InvalidGlyphBounds { sequence } => {
                write!(
                    formatter,
                    "trace glyph {sequence} has invalid destination bounds"
                )
            }
            Self::InvalidGlyphAtlasBounds { sequence } => {
                write!(formatter, "trace glyph {sequence} has invalid atlas bounds")
            }
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
        let (revision, logical_size, descriptor) = decode_viewport(self.revision, &self.viewport)?;
        if self.quads.len() > MAX_TRACE_OPERATIONS {
            return Err(TraceDecodeError::TooManyOperations);
        }

        let viewport_bounds = [
            0.0,
            0.0,
            self.viewport.logical_width,
            self.viewport.logical_height,
        ];
        let mut builder = SceneBuilder::new(revision, logical_size);
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

impl PreparedTraceInput {
    /// Decodes one prepared renderer scene without shaping, rasterization, or native adaptation.
    ///
    /// # Errors
    ///
    /// Returns a stage-specific error for invalid target, clip, operation,
    /// atlas, painter order, geometry, color, or capacity identity.
    pub fn decode(self) -> Result<DecodedTrace, TraceDecodeError> {
        let (revision, logical_size, descriptor) = decode_viewport(self.revision, &self.viewport)?;
        if self.clips.len() > MAX_TRACE_CLIPS {
            return Err(TraceDecodeError::TooManyClips);
        }
        if self.operations.len() > MAX_TRACE_OPERATIONS {
            return Err(TraceDecodeError::TooManyOperations);
        }

        let mut builder = SceneBuilder::new(revision, logical_size);
        let mut clip_ids = Vec::new();
        clip_ids
            .try_reserve_exact(self.clips.len())
            .map_err(|_| TraceDecodeError::TooManyClips)?;
        for (index, clip) in self.clips.into_iter().enumerate() {
            let bounds =
                decode_rect(clip.bounds).ok_or(TraceDecodeError::InvalidClipBounds { index })?;
            clip_ids.push(builder.push_clip(Clip::new(bounds)));
        }

        let atlas_extent = self.atlas.as_ref().map(|atlas| (atlas.width, atlas.height));
        if let Some(atlas) = self.atlas {
            let width = NonZeroU32::new(atlas.width).ok_or(TraceDecodeError::InvalidAtlas)?;
            let height = NonZeroU32::new(atlas.height).ok_or(TraceDecodeError::InvalidAtlas)?;
            let pixels = usize::try_from(atlas.width)
                .ok()
                .and_then(|width| {
                    usize::try_from(atlas.height)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .ok_or(TraceDecodeError::InvalidAtlas)?;
            if atlas.revision == 0
                || pixels > MAX_TRACE_ATLAS_PIXELS
                || atlas.pixels.len() != pixels
            {
                return Err(TraceDecodeError::InvalidAtlas);
            }
            let image =
                GlyphAtlasImage::new(atlas.revision, width, height, Arc::from(atlas.pixels))
                    .map_err(|_| TraceDecodeError::InvalidAtlas)?;
            builder
                .set_glyph_atlas(image)
                .map_err(|_| TraceDecodeError::InvalidAtlas)?;
        }

        for (expected, operation) in self.operations.into_iter().enumerate() {
            let sequence = operation.sequence();
            if sequence != expected as u64 {
                return Err(TraceDecodeError::NoncontiguousSequence {
                    expected,
                    actual: sequence,
                });
            }
            match operation {
                PreparedTraceOperation::Quad(quad) => {
                    let bounds = decode_rect(quad.bounds)
                        .ok_or(TraceDecodeError::InvalidQuadBounds { sequence })?;
                    let color = decode_color(quad.color)
                        .ok_or(TraceDecodeError::InvalidQuadColor { sequence })?;
                    let mut primitive = Quad::new(bounds, color);
                    if let Some(index) = quad.clip {
                        let clip = clip_ids
                            .get(index)
                            .copied()
                            .ok_or(TraceDecodeError::InvalidClipReference { sequence, index })?;
                        primitive = primitive.clipped(clip);
                    }
                    let scene_error = TraceDecodeError::InvalidClipReference {
                        sequence,
                        index: quad.clip.unwrap_or(usize::MAX),
                    };
                    builder.push_quad(primitive).or(Err(scene_error))?;
                }
                PreparedTraceOperation::Glyph(glyph) => {
                    let bounds = decode_rect(glyph.bounds)
                        .ok_or(TraceDecodeError::InvalidGlyphBounds { sequence })?;
                    let color = decode_color(glyph.color)
                        .ok_or(TraceDecodeError::InvalidQuadColor { sequence })?;
                    let (atlas_width, atlas_height) =
                        atlas_extent.ok_or(TraceDecodeError::MissingAtlas { sequence })?;
                    let source = decode_atlas_bounds(glyph.atlas_bounds, atlas_width, atlas_height)
                        .ok_or(TraceDecodeError::InvalidGlyphAtlasBounds { sequence })?;
                    let mut primitive = Glyph::new(bounds, source, color);
                    if let Some(index) = glyph.clip {
                        let clip = clip_ids
                            .get(index)
                            .copied()
                            .ok_or(TraceDecodeError::InvalidClipReference { sequence, index })?;
                        primitive = primitive.clipped(clip);
                    }
                    let scene_error = TraceDecodeError::InvalidGlyphAtlasBounds { sequence };
                    builder.push_glyph(primitive).or(Err(scene_error))?;
                }
            }
        }

        Ok(DecodedTrace {
            scene: builder.finish(),
            descriptor,
        })
    }
}

fn decode_viewport(
    revision: u64,
    viewport: &TraceViewport,
) -> Result<(SceneRevision, Size, OffscreenDescriptor), TraceDecodeError> {
    if revision == 0 {
        return Err(TraceDecodeError::ZeroRevision);
    }
    let logical_size = Size::new(viewport.logical_width, viewport.logical_height)
        .filter(|size| !size.is_empty())
        .ok_or(TraceDecodeError::InvalidLogicalViewport)?;
    let clear = decode_color(viewport.clear_color).ok_or(TraceDecodeError::InvalidClearColor)?;
    let descriptor = OffscreenDescriptor::new(
        viewport.pixel_width,
        viewport.pixel_height,
        viewport.scale_factor,
        clear,
    )
    .map_err(|_| TraceDecodeError::InvalidPhysicalTarget)?;
    if !physical_matches(
        viewport.logical_width,
        viewport.scale_factor,
        viewport.pixel_width,
    ) || !physical_matches(
        viewport.logical_height,
        viewport.scale_factor,
        viewport.pixel_height,
    ) {
        return Err(TraceDecodeError::PhysicalViewportMismatch);
    }
    Ok((SceneRevision::new(revision), logical_size, descriptor))
}

fn decode_rect(values: [f32; 4]) -> Option<Rect> {
    let origin = Point::new(values[0], values[1])?;
    let size = Size::new(values[2], values[3])?;
    (!size.is_empty()).then_some(Rect::new(origin, size))
}

fn decode_color(values: [f32; 4]) -> Option<LinearRgba> {
    LinearRgba::new(values[0], values[1], values[2], values[3])
}

fn decode_atlas_bounds(values: [u32; 4], width: u32, height: u32) -> Option<AtlasBounds> {
    let source_width = NonZeroU32::new(values[2])?;
    let source_height = NonZeroU32::new(values[3])?;
    let end_x = values[0].checked_add(source_width.get())?;
    let end_y = values[1].checked_add(source_height.get())?;
    (end_x <= width && end_y <= height).then_some(AtlasBounds::new(
        values[0],
        values[1],
        source_width,
        source_height,
    ))
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
        MAX_TRACE_ATLAS_PIXELS, MAX_TRACE_CLIPS, MAX_TRACE_OPERATIONS, PreparedTraceInput,
        PreparedTraceOperation, PreparedTraceQuad, TraceAtlas, TraceClip, TraceDecodeError,
        TraceGlyph, TraceInput, TraceQuad, TraceViewport, decode_atlas_bounds, physical_matches,
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
            (TraceDecodeError::TooManyClips, "trace clip limit exceeded"),
            (
                TraceDecodeError::InvalidClipBounds { index: 9 },
                "trace clip 9 has invalid bounds",
            ),
            (
                TraceDecodeError::InvalidClipReference {
                    sequence: 10,
                    index: 11,
                },
                "trace operation 10 references invalid clip index 11",
            ),
            (TraceDecodeError::InvalidAtlas, "trace A8 atlas is invalid"),
            (
                TraceDecodeError::MissingAtlas { sequence: 12 },
                "trace glyph 12 requires an A8 atlas",
            ),
            (
                TraceDecodeError::InvalidGlyphBounds { sequence: 13 },
                "trace glyph 13 has invalid destination bounds",
            ),
            (
                TraceDecodeError::InvalidGlyphAtlasBounds { sequence: 14 },
                "trace glyph 14 has invalid atlas bounds",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message);
        }
    }

    fn prepared_trace() -> PreparedTraceInput {
        PreparedTraceInput {
            revision: 11,
            viewport: TraceViewport {
                clear_color: [0.0, 0.0, 0.0, 1.0],
                ..viewport()
            },
            clips: vec![TraceClip {
                bounds: [0.0, 0.0, 4.0, 2.0],
            }],
            atlas: Some(TraceAtlas {
                revision: 1,
                width: 2,
                height: 2,
                pixels: vec![255, 0, 0, 255],
            }),
            operations: vec![
                PreparedTraceOperation::Quad(PreparedTraceQuad {
                    sequence: 0,
                    bounds: [0.0, 0.0, 4.0, 2.0],
                    color: [0.0, 0.0, 1.0, 1.0],
                    clip: Some(0),
                }),
                PreparedTraceOperation::Glyph(TraceGlyph {
                    sequence: 1,
                    bounds: [1.0, 0.0, 2.0, 2.0],
                    atlas_bounds: [0, 0, 2, 2],
                    color: [1.0, 1.0, 1.0, 1.0],
                    clip: Some(0),
                }),
            ],
        }
    }

    #[test]
    fn prepared_scene_preserves_clips_atlas_glyphs_and_painter_order() {
        let decoded = prepared_trace().decode();
        assert_eq!(
            decoded.as_ref().map(|decoded| (
                decoded.scene().clips().len(),
                decoded.scene().quads().len(),
                decoded.scene().glyphs().len(),
                decoded.scene().operation_count(),
                decoded
                    .scene()
                    .glyph_atlas()
                    .map(alpine_scene::GlyphAtlasImage::pixels),
            )),
            Ok((1, 1, 1, 2, Some(&[255, 0, 0, 255][..])))
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
    fn prepared_scene_enforces_exact_collection_limits() {
        let clip = TraceClip {
            bounds: [0.0, 0.0, 4.0, 2.0],
        };
        let exact_clips = PreparedTraceInput {
            clips: vec![clip; MAX_TRACE_CLIPS],
            atlas: None,
            operations: Vec::new(),
            ..prepared_trace()
        };
        assert!(exact_clips.clone().decode().is_ok());
        let mut too_many_clips = exact_clips;
        too_many_clips.clips.push(clip);
        assert_eq!(too_many_clips.decode(), Err(TraceDecodeError::TooManyClips));

        let operations = (0..MAX_TRACE_OPERATIONS)
            .map(|sequence| {
                PreparedTraceOperation::Quad(PreparedTraceQuad {
                    sequence: sequence as u64,
                    bounds: [0.0, 0.0, 1.0, 1.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                    clip: None,
                })
            })
            .collect::<Vec<_>>();
        let exact_operations = PreparedTraceInput {
            clips: Vec::new(),
            atlas: None,
            operations: operations.clone(),
            ..prepared_trace()
        };
        assert_eq!(
            exact_operations
                .decode()
                .map(|decoded| decoded.scene().operation_count()),
            Ok(MAX_TRACE_OPERATIONS)
        );
        let mut too_many_operations = operations;
        too_many_operations.push(PreparedTraceOperation::Quad(PreparedTraceQuad {
            sequence: MAX_TRACE_OPERATIONS as u64,
            bounds: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 1.0, 1.0, 1.0],
            clip: None,
        }));
        assert_eq!(
            PreparedTraceInput {
                clips: Vec::new(),
                atlas: None,
                operations: too_many_operations,
                ..prepared_trace()
            }
            .decode(),
            Err(TraceDecodeError::TooManyOperations)
        );
    }

    #[test]
    fn prepared_scene_enforces_atlas_limit_and_independent_conditions() {
        let exact_pixels = MAX_TRACE_ATLAS_PIXELS;
        let exact = PreparedTraceInput {
            clips: Vec::new(),
            atlas: Some(TraceAtlas {
                revision: 1,
                width: u32::try_from(exact_pixels).unwrap_or(u32::MAX),
                height: 1,
                pixels: vec![0; exact_pixels],
            }),
            operations: Vec::new(),
            ..prepared_trace()
        };
        assert!(exact.decode().is_ok());

        let over_limit = MAX_TRACE_ATLAS_PIXELS + 1;
        assert_eq!(
            PreparedTraceInput {
                clips: Vec::new(),
                atlas: Some(TraceAtlas {
                    revision: 1,
                    width: u32::try_from(over_limit).unwrap_or(u32::MAX),
                    height: 1,
                    pixels: vec![0; over_limit],
                }),
                operations: Vec::new(),
                ..prepared_trace()
            }
            .decode(),
            Err(TraceDecodeError::InvalidAtlas)
        );

        let mut zero_revision = prepared_trace();
        if let Some(atlas) = &mut zero_revision.atlas {
            atlas.revision = 0;
        }
        assert_eq!(zero_revision.decode(), Err(TraceDecodeError::InvalidAtlas));
    }

    #[test]
    fn prepared_scene_covers_unclipped_and_noncontiguous_operations() {
        let unclipped_quad = PreparedTraceInput {
            clips: Vec::new(),
            atlas: None,
            operations: vec![PreparedTraceOperation::Quad(PreparedTraceQuad {
                sequence: 0,
                bounds: [0.0, 0.0, 1.0, 1.0],
                color: [1.0, 1.0, 1.0, 1.0],
                clip: None,
            })],
            ..prepared_trace()
        };
        assert!(unclipped_quad.decode().is_ok());

        let mut unclipped_glyph = prepared_trace();
        if let PreparedTraceOperation::Glyph(glyph) = &mut unclipped_glyph.operations[1] {
            glyph.clip = None;
        }
        assert!(unclipped_glyph.decode().is_ok());

        let mut noncontiguous = prepared_trace();
        if let PreparedTraceOperation::Glyph(glyph) = &mut noncontiguous.operations[1] {
            glyph.sequence = 3;
        }
        assert_eq!(
            noncontiguous.decode(),
            Err(TraceDecodeError::NoncontiguousSequence {
                expected: 1,
                actual: 3,
            })
        );
    }

    #[test]
    fn prepared_scene_rejects_clip_atlas_and_glyph_contract_breaks() {
        assert!(decode_atlas_bounds([1, 0, 2, 2], 2, 2).is_none());
        assert!(decode_atlas_bounds([0, 1, 2, 2], 2, 2).is_none());

        let mut invalid_clip = prepared_trace();
        if let PreparedTraceOperation::Quad(quad) = &mut invalid_clip.operations[0] {
            quad.clip = Some(1);
        }
        assert_eq!(
            invalid_clip.decode(),
            Err(TraceDecodeError::InvalidClipReference {
                sequence: 0,
                index: 1,
            })
        );

        let mut invalid_atlas = prepared_trace();
        if let Some(atlas) = &mut invalid_atlas.atlas {
            atlas.pixels.pop();
        }
        assert_eq!(invalid_atlas.decode(), Err(TraceDecodeError::InvalidAtlas));

        let mut invalid_source = prepared_trace();
        if let PreparedTraceOperation::Glyph(glyph) = &mut invalid_source.operations[1] {
            glyph.atlas_bounds = [1, 1, 2, 2];
        }
        assert_eq!(
            invalid_source.decode(),
            Err(TraceDecodeError::InvalidGlyphAtlasBounds { sequence: 1 })
        );

        let mut horizontal_overflow = prepared_trace();
        if let PreparedTraceOperation::Glyph(glyph) = &mut horizontal_overflow.operations[1] {
            glyph.atlas_bounds = [1, 0, 2, 2];
        }
        assert_eq!(
            horizontal_overflow.decode(),
            Err(TraceDecodeError::InvalidGlyphAtlasBounds { sequence: 1 })
        );

        let mut vertical_overflow = prepared_trace();
        if let PreparedTraceOperation::Glyph(glyph) = &mut vertical_overflow.operations[1] {
            glyph.atlas_bounds = [0, 1, 2, 2];
        }
        assert_eq!(
            vertical_overflow.decode(),
            Err(TraceDecodeError::InvalidGlyphAtlasBounds { sequence: 1 })
        );
    }
}
