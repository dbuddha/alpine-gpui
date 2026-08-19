//! Fixed-hardware compositor and SDR transfer qualification driver.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    validation::run()
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
mod validation {
    use std::{
        env,
        error::Error,
        fmt::Write as _,
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::Duration,
    };

    use alpine_core::{LinearRgba, Point, Rect, Size};
    use alpine_platform_macos::{
        NativeSurface, SurfaceDescriptor, SurfaceSnapshot, native_validation,
    };
    use alpine_scene::{Primitive, Scene, SceneBuilder, SceneRevision};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    const TITLE: &str = "Alpine onscreen SDR qualification";
    const INITIAL_WIDTH: f32 = 500.0;
    const INITIAL_HEIGHT: f32 = 300.0;
    const RESIZED_WIDTH: f32 = 640.0;
    const RESIZED_HEIGHT: f32 = 360.0;
    const PATCH_LEVELS: [f32; 5] = [0.0, 0.18, 0.5, 0.75, 1.0];

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TransferControl {
        Accepted,
        WrongDoubleEncoded,
    }

    impl TransferControl {
        const fn name(self) -> &'static str {
            match self {
                Self::Accepted => "accepted",
                Self::WrongDoubleEncoded => "wrong-transfer",
            }
        }

        fn value(self, linear: f32) -> f32 {
            match self {
                Self::Accepted => linear,
                Self::WrongDoubleEncoded => linear_to_srgb(linear),
            }
        }
    }

    struct CaptureConfig {
        helper: PathBuf,
        output: PathBuf,
        revision: String,
    }

    impl CaptureConfig {
        fn from_environment() -> TestResult<Option<Self>> {
            let Some(helper) = env::var_os("ALPINE_ONSCREEN_SDR_HELPER") else {
                return Ok(None);
            };
            let output = env::var_os("ALPINE_ONSCREEN_SDR_OUTPUT")
                .ok_or("ALPINE_ONSCREEN_SDR_OUTPUT is required with the capture helper")?;
            let revision = env::var("ALPINE_ONSCREEN_SDR_REVISION")
                .map_err(|_| "ALPINE_ONSCREEN_SDR_REVISION is required")?;
            if revision.len() != 40
                || !revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err("onscreen SDR revision must be 40 lowercase hexadecimal bytes".into());
            }
            Ok(Some(Self {
                helper: PathBuf::from(helper),
                output: PathBuf::from(output),
                revision,
            }))
        }
    }

    pub(super) fn run() -> TestResult {
        let descriptor = SurfaceDescriptor::new(
            TITLE,
            f64::from(INITIAL_WIDTH),
            f64::from(INITIAL_HEIGHT),
            1.0,
        )?;
        let surface = native_validation::new_surface(&descriptor)?;
        let screens = native_validation::screen_configurations(&surface);
        assert!(!screens.is_empty());
        assert!(native_validation::move_window_to_screen(&surface, usize::MAX).is_err());

        let Some(config) = CaptureConfig::from_environment()? else {
            let first = native_validation::move_window_to_screen(&surface, 0)?;
            assert_eq!(first.index(), 0);
            assert_ne!(first.identity(), 0);
            assert!(first.backing_scale().is_finite());
            assert!(first.backing_scale() > 0.0);
            let (_, _, width, height) = first.visible_frame();
            assert!(width > 0.0 && height > 0.0);
            surface.close();
            return Ok(());
        };

        fs::create_dir_all(&config.output)?;
        let (first, second) = distinct_scale_pair(&screens)
            .ok_or("full qualification requires two real displays with different backing scales")?;
        let first = native_validation::move_window_to_screen(&surface, first.index())?;
        surface.show()?;

        let accepted_scene = config.output.join("accepted.scene");
        let wrong_scene = config.output.join("wrong-transfer.scene");
        fs::write(&accepted_scene, canonical_scene(TransferControl::Accepted))?;
        fs::write(
            &wrong_scene,
            canonical_scene(TransferControl::WrongDoubleEncoded),
        )?;

        let mut revision = 1_u64;
        let launch = present(
            &surface,
            revision,
            INITIAL_WIDTH,
            INITIAL_HEIGHT,
            TransferControl::Accepted,
        )?;
        capture(
            &config,
            "launch",
            TransferControl::Accepted,
            revision,
            INITIAL_WIDTH,
            INITIAL_HEIGHT,
            first.backing_scale(),
            launch,
            &accepted_scene,
        )?;

        native_validation::resize_content(
            &surface,
            f64::from(RESIZED_WIDTH),
            f64::from(RESIZED_HEIGHT),
        );
        if !native_validation::inject_configuration_callback(&surface) {
            return Err("AppKit did not publish the resized native configuration".into());
        }
        revision += 1;
        let resized = present(
            &surface,
            revision,
            RESIZED_WIDTH,
            RESIZED_HEIGHT,
            TransferControl::Accepted,
        )?;
        capture(
            &config,
            "resize",
            TransferControl::Accepted,
            revision,
            RESIZED_WIDTH,
            RESIZED_HEIGHT,
            first.backing_scale(),
            resized,
            &accepted_scene,
        )?;

        let moved = native_validation::move_window_to_screen(&surface, second.index())?;
        if moved.identity() == first.identity()
            || moved.backing_scale().to_bits() == first.backing_scale().to_bits()
        {
            return Err(
                "real display move did not change display identity and backing scale".into(),
            );
        }
        revision += 1;
        let display_move = present(
            &surface,
            revision,
            RESIZED_WIDTH,
            RESIZED_HEIGHT,
            TransferControl::Accepted,
        )?;
        capture(
            &config,
            "display-move",
            TransferControl::Accepted,
            revision,
            RESIZED_WIDTH,
            RESIZED_HEIGHT,
            moved.backing_scale(),
            display_move,
            &accepted_scene,
        )?;

        revision += 1;
        let wrong = present(
            &surface,
            revision,
            RESIZED_WIDTH,
            RESIZED_HEIGHT,
            TransferControl::WrongDoubleEncoded,
        )?;
        capture(
            &config,
            "wrong-transfer",
            TransferControl::WrongDoubleEncoded,
            revision,
            RESIZED_WIDTH,
            RESIZED_HEIGHT,
            moved.backing_scale(),
            wrong,
            &wrong_scene,
        )?;

        surface.close();
        Ok(())
    }

    fn distinct_scale_pair(
        screens: &[native_validation::ValidationScreenConfiguration],
    ) -> Option<(
        native_validation::ValidationScreenConfiguration,
        native_validation::ValidationScreenConfiguration,
    )> {
        screens.iter().copied().find_map(|first| {
            screens
                .iter()
                .copied()
                .find(|second| {
                    second.identity() != first.identity()
                        && second.backing_scale().to_bits() != first.backing_scale().to_bits()
                })
                .map(|second| (first, second))
        })
    }

    fn present(
        surface: &NativeSurface,
        revision: u64,
        width: f32,
        height: f32,
        control: TransferControl,
    ) -> TestResult<SurfaceSnapshot> {
        let before = surface.snapshot();
        let scene = chart_scene(revision, width, height, control)?;
        let clear = LinearRgba::new(0.0, 0.0, 0.0, 1.0).ok_or("valid clear")?;
        assert_eq!(surface.request_frame(scene, clear)?.get(), revision);
        native_validation::run_until_frame_terminal(surface, Duration::from_secs(8));
        if let Some(error) = surface.take_error()? {
            return Err(error.into());
        }
        let snapshot = surface.snapshot();
        assert!(snapshot.submission_count() > before.submission_count());
        assert!(snapshot.presented_count() > before.presented_count());
        assert_ne!(snapshot.last_presented_time_bits(), 0);
        assert_eq!(snapshot.occupied_frame_slots(), 0);
        assert_eq!(snapshot.submitted_frame_slots(), 0);
        assert!(snapshot.display_link_paused());
        Ok(snapshot)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the external capture process receives every independent evidence identity"
    )]
    fn capture(
        config: &CaptureConfig,
        stage: &str,
        control: TransferControl,
        scene_revision: u64,
        logical_width: f32,
        logical_height: f32,
        backing_scale: f64,
        snapshot: SurfaceSnapshot,
        scene_path: &Path,
    ) -> TestResult {
        let status = Command::new(&config.helper)
            .args([
                "--pid",
                &std::process::id().to_string(),
                "--title",
                TITLE,
                "--stage",
                stage,
                "--control",
                control.name(),
                "--revision",
                &config.revision,
                "--scene-revision",
                &scene_revision.to_string(),
                "--scene",
                &scene_path.to_string_lossy(),
                "--output",
                &config.output.to_string_lossy(),
                "--logical-width",
                &logical_width.to_string(),
                "--logical-height",
                &logical_height.to_string(),
                "--backing-scale",
                &backing_scale.to_string(),
                "--presented-time-bits",
                &snapshot.last_presented_time_bits().to_string(),
            ])
            .status()?;
        if !status.success() {
            return Err(format!("onscreen capture stage {stage:?} failed with {status}").into());
        }
        Ok(())
    }

    fn chart_scene(
        revision: u64,
        width: f32,
        height: f32,
        control: TransferControl,
    ) -> TestResult<Scene> {
        let viewport = Size::new(width, height).ok_or("valid chart viewport")?;
        let patch_width = width / 5.0;
        let mut builder = SceneBuilder::new(SceneRevision::new(revision), viewport);
        for (index, level) in PATCH_LEVELS.into_iter().enumerate() {
            let x = f32::from(u16::try_from(index)?) * patch_width;
            let bounds = Rect::new(
                Point::new(x, 0.0).ok_or("valid patch origin")?,
                Size::new(patch_width, height).ok_or("valid patch size")?,
            );
            let value = control.value(level);
            let color = LinearRgba::new(value, value, value, 1.0).ok_or("valid patch color")?;
            builder.push(Primitive::Quad { bounds, color });
        }
        Ok(builder.finish())
    }

    fn canonical_scene(control: TransferControl) -> String {
        let mut output = String::from("schema=alpine-onscreen-sdr-scene/v1\n");
        let _ = writeln!(output, "control={}", control.name());
        for (index, level) in PATCH_LEVELS.into_iter().enumerate() {
            let _ = writeln!(
                output,
                "patch={index},linear={level:.6},submitted={:.6}",
                control.value(level)
            );
        }
        output
    }

    fn linear_to_srgb(value: f32) -> f32 {
        if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        }
    }
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
