use crate::{OffscreenError, ValidatedFrame, frame::map_reservation_failure};

/// Maximum pixels rendered by one CPU reference invocation.
pub const MAX_ORACLE_PIXELS: usize = 16_777_216;

/// An owned compact linear-premultiplied BGRA8 reference image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bgra8Image {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

impl Bgra8Image {
    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) fn from_compact_parts(width: u32, height: u32, bytes: Vec<u8>) -> Self {
        debug_assert_eq!(
            bytes.len(),
            usize::try_from(width)
                .ok()
                .and_then(|width| usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height)))
                .and_then(|pixels| pixels.checked_mul(4))
                .unwrap_or(usize::MAX)
        );
        Self {
            width,
            height,
            bytes,
        }
    }

    /// Returns the image width in physical pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the image height in physical pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns compact top-to-bottom BGRA8 bytes with no row padding.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns one BGRA8 pixel or `None` when the coordinate is outside the image.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let pixel_index = usize::try_from(y)
            .ok()?
            .checked_mul(usize::try_from(self.width).ok()?)?
            .checked_add(usize::try_from(x).ok()?)?;
        let byte_index = pixel_index.checked_mul(4)?;
        Some([
            *self.bytes.get(byte_index)?,
            *self.bytes.get(byte_index + 1)?,
            *self.bytes.get(byte_index + 2)?,
            *self.bytes.get(byte_index + 3)?,
        ])
    }
}

impl ValidatedFrame {
    /// Renders the independent deterministic CPU reference for this frame.
    ///
    /// Coverage samples physical pixel centers. Scene colors are linear and
    /// unpremultiplied; the composited target stores premultiplied linear BGRA.
    /// This implementation is a correctness oracle, not a production fallback.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the configured oracle pixel limit is
    /// exceeded or image allocation cannot be reserved.
    pub fn reference_image(&self) -> Result<Bgra8Image, OffscreenError> {
        reference_image_with_order(self, false)
    }
}

fn reference_image_with_order(
    frame: &ValidatedFrame,
    reverse_painter_order: bool,
) -> Result<Bgra8Image, OffscreenError> {
    reference_image_with_reservation(frame, reverse_painter_order, |bytes, byte_len| {
        map_reservation_failure(
            bytes.try_reserve_exact(byte_len),
            OffscreenError::OracleAllocationFailed { bytes: byte_len },
        )
    })
}

fn reference_image_with_reservation<F>(
    frame: &ValidatedFrame,
    reverse_painter_order: bool,
    reserve: F,
) -> Result<Bgra8Image, OffscreenError>
where
    F: FnOnce(&mut Vec<u8>, usize) -> Result<(), OffscreenError>,
{
    let descriptor = frame.descriptor();
    let width = usize::try_from(descriptor.pixel_width())
        .map_err(|_| OffscreenError::CompactImageSizeOverflow)?;
    let height = usize::try_from(descriptor.pixel_height())
        .map_err(|_| OffscreenError::CompactImageSizeOverflow)?;
    let pixels = width
        .checked_mul(height)
        .ok_or(OffscreenError::CompactImageSizeOverflow)?;
    validate_oracle_pixel_count(pixels)?;

    let byte_len = frame.readback_layout().compact_len();
    let mut bytes = Vec::new();
    reserve(&mut bytes, byte_len)?;
    bytes.resize(byte_len, 0);

    let clear = descriptor.clear();
    let clear_alpha = clear.alpha();
    let initial = [
        clear.red() * clear_alpha,
        clear.green() * clear_alpha,
        clear.blue() * clear_alpha,
        clear_alpha,
    ];

    for physical_y in 0..descriptor.pixel_height() {
        for physical_x in 0..descriptor.pixel_width() {
            let center_x = f64::from(physical_x) + 0.5;
            let center_y = f64::from(physical_y) + 0.5;
            let mut destination = initial;
            if reverse_painter_order {
                for quad in frame.paints().iter().rev() {
                    composite_instance(frame, *quad, center_x, center_y, &mut destination);
                }
            } else {
                for quad in frame.paints() {
                    composite_instance(frame, *quad, center_x, center_y, &mut destination);
                }
            }
            let y = usize::try_from(physical_y)
                .map_err(|_| OffscreenError::CompactImageSizeOverflow)?;
            let x = usize::try_from(physical_x)
                .map_err(|_| OffscreenError::CompactImageSizeOverflow)?;
            let pixel_offset = (y * width + x) * 4;
            bytes[pixel_offset] = quantize(destination[2]);
            bytes[pixel_offset + 1] = quantize(destination[1]);
            bytes[pixel_offset + 2] = quantize(destination[0]);
            bytes[pixel_offset + 3] = quantize(destination[3]);
        }
    }

    Ok(Bgra8Image {
        width: descriptor.pixel_width(),
        height: descriptor.pixel_height(),
        bytes,
    })
}

fn composite_instance(
    frame: &ValidatedFrame,
    instance: crate::LoweredPaint,
    center_x: f64,
    center_y: f64,
    destination: &mut [f32; 4],
) {
    let bounds = instance.bounds();
    let atlas_uv = instance.atlas_uv();
    let coverage = if atlas_uv[0] < 0.0 {
        1.0
    } else {
        sample_atlas(frame, bounds, atlas_uv, center_x, center_y)
    };
    let mut source = instance.color();
    source[3] *= coverage;
    composite_if_covered(bounds, source, center_x, center_y, destination);
}

fn sample_atlas(
    frame: &ValidatedFrame,
    bounds: [f32; 4],
    atlas_uv: [f32; 4],
    center_x: f64,
    center_y: f64,
) -> f32 {
    let Some(atlas) = frame.glyph_atlas() else {
        return 0.0;
    };
    let horizontal = (center_x - f64::from(bounds[0])) / f64::from(bounds[2] - bounds[0]);
    let vertical = (center_y - f64::from(bounds[1])) / f64::from(bounds[3] - bounds[1]);
    let u = f64::from(atlas_uv[0]) + horizontal * f64::from(atlas_uv[2] - atlas_uv[0]);
    let v = f64::from(atlas_uv[1]) + vertical * f64::from(atlas_uv[3] - atlas_uv[1]);
    let x = (u * f64::from(atlas.width().get())).floor();
    let y = (v * f64::from(atlas.height().get())).floor();
    if x < 0.0 || y < 0.0 {
        return 0.0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (x, y) = (x as usize, y as usize);
    let width = usize::try_from(atlas.width().get()).unwrap_or_default();
    let height = usize::try_from(atlas.height().get()).unwrap_or_default();
    if x >= width || y >= height {
        return 0.0;
    }
    atlas
        .pixel(
            u32::try_from(x).unwrap_or(u32::MAX),
            u32::try_from(y).unwrap_or(u32::MAX),
        )
        .map_or(0.0, |coverage| f32::from(coverage) / 255.0)
}

fn validate_oracle_pixel_count(pixels: usize) -> Result<(), OffscreenError> {
    if pixels > MAX_ORACLE_PIXELS {
        return Err(OffscreenError::OraclePixelLimitExceeded {
            pixels,
            limit: MAX_ORACLE_PIXELS,
        });
    }
    Ok(())
}

fn composite_if_covered(
    bounds: [f32; 4],
    source: [f32; 4],
    center_x: f64,
    center_y: f64,
    destination: &mut [f32; 4],
) {
    if center_x < f64::from(bounds[0])
        || center_y < f64::from(bounds[1])
        || center_x >= f64::from(bounds[2])
        || center_y >= f64::from(bounds[3])
    {
        return;
    }
    let source_alpha = source[3];
    let inverse_source_alpha = 1.0 - source_alpha;
    destination[0] = source[0] * source_alpha + destination[0] * inverse_source_alpha;
    destination[1] = source[1] * source_alpha + destination[1] * inverse_source_alpha;
    destination[2] = source[2] * source_alpha + destination[2] * inverse_source_alpha;
    destination[3] = source_alpha + destination[3] * inverse_source_alpha;
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn quantize(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
#[path = "oracle_coverage_tests.rs"]
mod oracle_coverage_tests;

#[cfg(test)]
mod tests {
    use std::error::Error;

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_scene::{Primitive, SceneBuilder, SceneRevision};

    use crate::{OffscreenDescriptor, OffscreenError, ValidatedFrame};

    use super::{
        Bgra8Image, MAX_ORACLE_PIXELS, composite_if_covered, reference_image_with_order,
        reference_image_with_reservation, validate_oracle_pixel_count,
    };

    fn color(red: f32, green: f32, blue: f32, alpha: f32) -> Result<LinearRgba, &'static str> {
        LinearRgba::new(red, green, blue, alpha).ok_or("valid test color")
    }

    fn size(width: f32, height: f32) -> Result<Size, &'static str> {
        Size::new(width, height).ok_or("valid test size")
    }

    fn point(x: f32, y: f32) -> Result<Point, &'static str> {
        Point::new(x, y).ok_or("valid test point")
    }

    fn overlapping_frame() -> Result<ValidatedFrame, Box<dyn Error>> {
        let mut builder = SceneBuilder::new(SceneRevision::new(3), size(1.0, 1.0)?);
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(0.0, 0.0)?, size(1.0, 1.0)?),
            color: color(1.0, 0.0, 0.0, 0.5)?,
        });
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(0.0, 0.0)?, size(1.0, 1.0)?),
            color: color(0.0, 0.0, 1.0, 0.5)?,
        });
        let descriptor = OffscreenDescriptor::new(1, 1, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?;
        Ok(ValidatedFrame::new(&builder.finish(), descriptor)?)
    }

    #[test]
    fn oracle_uses_pixel_centers_and_linear_source_over() -> Result<(), Box<dyn Error>> {
        let frame = overlapping_frame()?;
        let image = frame.reference_image()?;
        assert_eq!(image.width(), 1);
        assert_eq!(image.height(), 1);
        assert_eq!(image.bytes(), &[128, 0, 64, 191]);
        assert_eq!(image.pixel(0, 0), Some([128, 0, 64, 191]));
        assert_eq!(image.pixel(1, 0), None);
        assert_eq!(image.pixel(0, 1), None);
        Ok(())
    }

    #[test]
    fn pixel_bounds_do_not_alias_adjacent_rows() {
        let image = Bgra8Image {
            width: 2,
            height: 2,
            bytes: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        };
        assert_eq!(image.pixel(1, 1), Some([13, 14, 15, 16]));
        assert_eq!(image.pixel(2, 0), None);
        assert_eq!(image.pixel(0, 2), None);
    }

    #[test]
    fn deliberately_faulty_order_is_detected() -> Result<(), Box<dyn Error>> {
        let frame = overlapping_frame()?;
        let correct = frame.reference_image()?;
        let faulty = reference_image_with_order(&frame, true)?;
        assert_eq!(faulty.bytes(), &[64, 0, 128, 191]);
        assert_ne!(correct, faulty);
        Ok(())
    }

    #[test]
    fn clear_and_coverage_edges_are_deterministic() -> Result<(), Box<dyn Error>> {
        let mut builder = SceneBuilder::new(SceneRevision::new(4), size(2.0, 1.0)?);
        builder.push(Primitive::Quad {
            bounds: Rect::new(point(0.5, 0.0)?, size(0.5, 1.0)?),
            color: color(1.0, 1.0, 1.0, 1.0)?,
        });
        let descriptor = OffscreenDescriptor::new(2, 1, 1.0, color(1.0, 0.0, 0.0, 0.5)?)?;
        let image = ValidatedFrame::new(&builder.finish(), descriptor)?.reference_image()?;
        assert_eq!(image.pixel(0, 0), Some([255, 255, 255, 255]));
        assert_eq!(image.pixel(1, 0), Some([0, 0, 128, 128]));

        let mut vertical = SceneBuilder::new(SceneRevision::new(6), size(1.0, 2.0)?);
        vertical.push(Primitive::Quad {
            bounds: Rect::new(point(0.0, 1.25)?, size(1.0, 0.5)?),
            color: color(1.0, 1.0, 1.0, 1.0)?,
        });
        let descriptor = OffscreenDescriptor::new(1, 2, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?;
        let image = ValidatedFrame::new(&vertical.finish(), descriptor)?.reference_image()?;
        assert_eq!(image.pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(image.pixel(0, 1), Some([255, 255, 255, 255]));
        Ok(())
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn coverage_and_green_blending_have_independent_controls() {
        let mut outside = [0.1, 0.2, 0.3, 0.4];
        composite_if_covered(
            [0.25, 0.25, 0.75, 0.75],
            [0.8, 0.6, 0.4, 0.5],
            0.0,
            0.5,
            &mut outside,
        );
        assert_eq!(outside, [0.1, 0.2, 0.3, 0.4]);

        let mut edge = [0.1, 0.2, 0.3, 0.4];
        composite_if_covered(
            [0.25, 0.25, 0.75, 0.75],
            [0.8, 0.6, 0.4, 0.5],
            0.25,
            0.5,
            &mut edge,
        );
        assert!((edge[1] - 0.4).abs() < f32::EPSILON);

        let mut top_edge = [0.1, 0.2, 0.3, 0.4];
        composite_if_covered(
            [0.25, 0.25, 0.75, 0.75],
            [0.8, 0.6, 0.4, 0.5],
            0.5,
            0.25,
            &mut top_edge,
        );
        assert!((top_edge[1] - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn oracle_rejects_excessive_pixel_work_before_allocation() -> Result<(), Box<dyn Error>> {
        let width = 4_097_u32;
        let height = 4_096_u32;
        let pixels = usize::try_from(width)? * usize::try_from(height)?;
        assert!(pixels > MAX_ORACLE_PIXELS);
        assert_eq!(validate_oracle_pixel_count(MAX_ORACLE_PIXELS), Ok(()));
        assert_eq!(
            validate_oracle_pixel_count(MAX_ORACLE_PIXELS + 1),
            Err(OffscreenError::OraclePixelLimitExceeded {
                pixels: MAX_ORACLE_PIXELS + 1,
                limit: MAX_ORACLE_PIXELS,
            })
        );
        let scene = SceneBuilder::new(SceneRevision::new(5), size(4_097.0, 4_096.0)?).finish();
        let descriptor = OffscreenDescriptor::new(width, height, 1.0, color(0.0, 0.0, 0.0, 0.0)?)?;
        let error = ValidatedFrame::new(&scene, descriptor)?.reference_image();
        assert_eq!(
            error,
            Err(OffscreenError::OraclePixelLimitExceeded {
                pixels,
                limit: MAX_ORACLE_PIXELS,
            })
        );
        Ok(())
    }

    #[test]
    fn oracle_allocation_failure_is_classified() -> Result<(), Box<dyn Error>> {
        let frame = overlapping_frame()?;
        assert_eq!(
            reference_image_with_reservation(&frame, false, |_, bytes| {
                Err(OffscreenError::OracleAllocationFailed { bytes })
            }),
            Err(OffscreenError::OracleAllocationFailed { bytes: 4 })
        );
        Ok(())
    }
}
