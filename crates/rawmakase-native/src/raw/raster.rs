//! Display-referred photos enter the existing renderer as linear sRGB, not CFA data.
use super::{Raw, Source, ffi};
use crate::camera_data::{CameraImage, Decode, Metadata};
use anyhow::{Result, ensure};
use image::ImageDecoder;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn open(path: &Path) -> Result<Raw> {
    let mut decoder = image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()?;
    let (width, height) = decoder.dimensions();
    ensure!(
        u64::from(width) * u64::from(height) <= 150_000_000,
        "图像超过 1.5 亿像素"
    );
    let orientation = decoder.orientation()?;
    let profile = decoder.icc_profile()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let mut rgba = image.to_rgba32f();
    let mut pixels: Vec<[f32; 3]> = rgba.pixels().map(|p| [p[0], p[1], p[2]]).collect();
    if let Some(profile) = profile {
        ffi::input_transform(&profile, &mut pixels)?;
    }
    // The photo renderer is RGB. Transparency is composited on white for editing/export;
    // the original PNG and its alpha remain untouched on disk.
    for (pixel, alpha) in pixels.iter().zip(rgba.pixels_mut()) {
        for c in 0..3 {
            alpha[c] = pixel[c] * alpha[3] + 1. - alpha[3];
        }
        alpha[3] = 1.;
    }
    let (width, height) = rgba.dimensions();
    let metadata = Metadata {
        display_referred: true,
        width,
        height,
        raw_width: width,
        raw_height: height,
        crop_width: width,
        crop_height: height,
        wb: [1.; 3],
        daylight_wb: [1.; 3],
        matrix: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        baseline_exposure: Some(0.),
        ..Default::default()
    };
    Ok(Raw {
        handle: Source::Raster(image::DynamicImage::ImageRgba32F(rgba)),
        metadata,
    })
}

pub(super) fn develop(raw: Raw, decode: Decode, cancel: &AtomicBool) -> Result<CameraImage> {
    ensure!(!cancel.load(Ordering::Relaxed), "已取消");
    let Source::Raster(image) = raw.handle else {
        unreachable!()
    };
    let fast = decode == Decode::Half;
    let rgb = if fast {
        image
            .resize(
                (image.width() / 2).max(1),
                (image.height() / 2).max(1),
                image::imageops::FilterType::Triangle,
            )
            .to_rgb32f()
    } else {
        image.to_rgb32f()
    };
    let pixels = rgb
        .pixels()
        .map(|p| p.0.map(rawmakase_model::color::srgb_decode))
        .collect();
    ensure!(!cancel.load(Ordering::Relaxed), "已取消");
    Ok(CameraImage {
        width: rgb.width(),
        height: rgb.height(),
        pixels,
        metadata: raw.metadata,
        recovered: Default::default(),
        fast,
        scale_factor: 1.,
        scale_clipped: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_images_edit_without_changing_the_source() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for extension in ["jpg", "png", "tiff"] {
            let path = dir.path().join(format!("图像.{extension}"));
            let mut fixture = image::RgbImage::from_pixel(64, 48, image::Rgb([128, 96, 64]));
            for (x, _, pixel) in fixture.enumerate_pixels_mut() {
                if x < 16 {
                    *pixel = image::Rgb([255, 64, 96]);
                } else if x >= 32 {
                    *pixel = image::Rgb([255, 255, 255]);
                }
            }
            fixture.save(&path)?;
            let original = std::fs::read(&path)?;
            let expected = image::open(&path)?.to_rgb8();
            let mut raw = crate::photo::open(&path)?;
            assert_eq!(crate::raw::thumbnail(&mut raw)?.dimensions(), (64, 48));
            let (profiles, _) = crate::camera_profiles::installed(&raw.metadata);
            let recipe =
                rawmakase_model::model::recipe::Recipe::with_profiles(&raw.metadata, &profiles);
            let decoded = raw.develop(Decode::full(Default::default()), &AtomicBool::new(false))?;
            assert_eq!((decoded.width, decoded.height), (64, 48));
            let normal = crate::develop::render(&decoded, &recipe.checked()?, 64)?;
            let changed = rawmakase_model::model::recipe::Recipe {
                exposure: 1.,
                ..recipe
            };
            let edited = crate::develop::render(&decoded, &changed.checked()?, 64)?;
            assert_ne!(normal.rgb8(), edited.rgb8());
            for (index, (actual, expected)) in
                normal.rgb8().iter().zip(expected.as_raw()).enumerate()
            {
                // The renderer's existing Lanczos resampling rings at the sharp
                // white boundary. Check neutral colors away from that edge.
                let x = (index / 3) % 64;
                if (12..20).contains(&x) || (28..36).contains(&x) {
                    continue;
                }
                assert!(
                    (i32::from(*actual) - i32::from(*expected)).abs() <= 2,
                    "neutral {extension} image changed at {index}: {actual} != {expected}"
                );
            }
            assert_eq!(std::fs::read(&path)?, original);
        }
        Ok(())
    }
    #[test]
    fn alpha_is_composited_and_cancellation_is_respected() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("alpha.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 0])).save(&path)?;
        assert!(
            crate::photo::open(&path)?
                .develop(Decode::Half, &AtomicBool::new(true))
                .is_err()
        );
        let decoded = crate::photo::open(&path)?
            .develop(Decode::full(Default::default()), &AtomicBool::new(false))?;
        assert_eq!(decoded.pixels[0], [1.; 3]);
        Ok(())
    }
}
