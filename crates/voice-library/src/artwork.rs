//! Bounded preset artwork import. This runs on the library/control worker, never in audio callbacks.
use super::{Library, new_id, validate_id};
use anyhow::{Context, Result, ensure};
use image::{ImageFormat, ImageReader, Limits, imageops::FilterType};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

pub const PRESET_IMAGE_SIDE: u32 = 256;
pub const MAX_PRESET_IMAGE_BYTES: u64 = 300 * 1024;
const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SOURCE_SIDE: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

/// A local managed thumbnail. Images remain separate from the versioned DSP preset JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PresetImage {
    pub preset_id: String,
    pub path: PathBuf,
}

impl Library {
    fn require_image_preset(&self, id: &str, editable: bool) -> Result<()> {
        validate_id(id)?;
        ensure!(
            !editable || !id.starts_with("builtin."),
            "only custom presets can have a custom image"
        );
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM presets WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?;
        ensure!(exists, "preset not found: {id}");
        Ok(())
    }

    fn image_file_name(&self, id: &str) -> Result<Option<String>> {
        self.db
            .query_row(
                "SELECT file_name FROM preset_images WHERE preset_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .context("read preset image metadata")
    }

    fn managed_image_path(&self, file_name: &str) -> Result<PathBuf> {
        let stem = file_name
            .strip_suffix(".png")
            .context("invalid managed preset image filename")?;
        validate_id(stem)?;
        ensure!(
            stem.starts_with("image-"),
            "invalid managed preset image filename"
        );
        Ok(self.root.join("artwork").join(file_name))
    }

    pub fn preset_image(&self, id: &str) -> Result<Option<PresetImage>> {
        self.require_image_preset(id, false)?;
        self.image_file_name(id)?
            .map(|name| {
                Ok(PresetImage {
                    preset_id: id.into(),
                    path: self.managed_image_path(&name)?,
                })
            })
            .transpose()
    }

    /// Import a local PNG/JPEG/WebP once, center-crop and persist a small managed PNG.
    /// Any decode/limit/database error keeps the previous image intact.
    pub fn set_preset_image(&self, id: &str, path: &Path) -> Result<PresetImage> {
        self.require_image_preset(id, true)?;
        let previous = self.preset_image(id)?;
        ensure!(
            fs::metadata(path)?.is_file(),
            "image source must be a regular file"
        );
        let source = File::open(path).context("open preset image")?;
        let metadata = source.metadata()?;
        ensure!(metadata.is_file(), "image source must be a regular file");
        ensure!(
            metadata.len() <= MAX_SOURCE_BYTES,
            "preset image exceeds 8 MiB"
        );
        let mut bytes = Vec::new();
        source.take(MAX_SOURCE_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_SOURCE_BYTES,
            "preset image exceeds 8 MiB"
        );
        let mut reader = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .context("identify preset image format")?;
        ensure!(
            matches!(
                reader.format(),
                Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
            ),
            "preset images must be PNG, JPEG or WebP"
        );
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_SOURCE_SIDE);
        limits.max_image_height = Some(MAX_SOURCE_SIDE);
        limits.max_alloc = Some(MAX_DECODE_BYTES);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .context("decode preset image (maximum 4096×4096, 64 MiB decoder budget)")?;
        let (width, height) = (decoded.width(), decoded.height());
        ensure!(width > 0 && height > 0, "preset image is empty");
        let side = width.min(height);
        // Crop before resize: a very narrow source must not expand into a huge intermediate image.
        let square = decoded.crop_imm((width - side) / 2, (height - side) / 2, side, side);
        drop(decoded);
        let thumbnail = square
            .resize_exact(PRESET_IMAGE_SIDE, PRESET_IMAGE_SIDE, FilterType::Lanczos3)
            .into_rgba8();
        drop(square);
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(thumbnail).write_to(&mut encoded, ImageFormat::Png)?;
        let encoded = encoded.into_inner();
        ensure!(
            encoded.len() as u64 <= MAX_PRESET_IMAGE_BYTES,
            "managed preset image exceeds 300 KiB"
        );

        fs::create_dir_all(self.root.join("artwork"))?;
        let file_name = format!("{}.png", new_id("image"));
        let destination = self.managed_image_path(&file_name)?;
        let mut created = false;
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)?;
            created = true;
            file.write_all(&encoded)
                .context("write managed preset image")?;
            file.sync_all()?;
            self.db.execute(
                "INSERT INTO preset_images(preset_id,file_name) VALUES(?1,?2)
                 ON CONFLICT(preset_id) DO UPDATE SET file_name=excluded.file_name",
                params![id, file_name],
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            if created {
                let _ = fs::remove_file(&destination);
            }
            return Err(error);
        }
        if let Some(previous) = previous {
            let _ = fs::remove_file(previous.path);
        }
        Ok(PresetImage {
            preset_id: id.into(),
            path: destination,
        })
    }

    /// Removing an absent image is idempotent; built-in and unknown presets remain protected.
    pub fn clear_preset_image(&self, id: &str) -> Result<()> {
        self.require_image_preset(id, true)?;
        let previous = self.preset_image(id)?;
        self.db
            .execute("DELETE FROM preset_images WHERE preset_id=?1", [id])?;
        if let Some(previous) = previous {
            let _ = fs::remove_file(previous.path);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Preset;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};
    use rusqlite::Connection;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(new_id("voice-image-test"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn save_custom(library: &Library) -> Result<()> {
        library.save_preset(&Preset {
            original: false,
            id: "custom.marine".into(),
            name: "Marine".into(),
            settings: Default::default(),
        })
    }

    #[test]
    fn artwork_is_managed_center_cropped_persistent_and_removed_with_preset() -> Result<()> {
        let temp = Temp::new();
        let root = temp.0.join("library");
        let source = temp.0.join("wide.png");
        let input = RgbaImage::from_fn(80, 40, |x, _| {
            if x < 20 {
                Rgba([255, 0, 0, 255])
            } else if x < 60 {
                Rgba([0, 255, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        input.save(&source)?;
        let managed;
        {
            let library = Library::open(&root)?;
            save_custom(&library)?;
            assert!(library.preset_image("custom.marine")?.is_none());
            let initial = library.set_preset_image("custom.marine", &source)?;
            library.set_preset_original("custom.marine", true)?;
            // Curated originals retain custom ownership and editable artwork.
            managed = library.set_preset_image("custom.marine", &source)?;
            assert!(!initial.path.exists());
            assert_ne!(managed.path, source);
            assert!(managed.path.starts_with(root.join("artwork")));
            assert!(fs::metadata(&managed.path)?.len() <= MAX_PRESET_IMAGE_BYTES);
            let thumbnail = image::open(&managed.path)?.into_rgba8();
            assert_eq!(
                thumbnail.dimensions(),
                (PRESET_IMAGE_SIDE, PRESET_IMAGE_SIDE)
            );
            assert!(
                thumbnail
                    .pixels()
                    .all(|pixel| *pixel == Rgba([0, 255, 0, 255]))
            );
            fs::remove_file(&source)?;
            library.export_preset("custom.marine", &temp.0.join("preset.json"))?;
        }
        let library = Library::open(&root)?;
        assert_eq!(
            library.preset_image("custom.marine")?,
            Some(managed.clone())
        );
        assert!(managed.path.is_file());
        // JSON interchange remains a DSP preset; reimport must retain local artwork.
        library.import_preset(&temp.0.join("preset.json"))?;
        assert_eq!(
            library.preset_image("custom.marine")?,
            Some(managed.clone())
        );
        library.delete_preset("custom.marine")?;
        assert!(!managed.path.exists());
        assert!(library.preset_image("custom.marine").is_err());
        Ok(())
    }

    #[test]
    fn jpeg_webp_replacement_clear_and_legacy_database_are_supported() -> Result<()> {
        let temp = Temp::new();
        let root = temp.0.join("library");
        {
            let library = Library::open(&root)?;
            save_custom(&library)?;
        }
        // A database made by an earlier build has no artwork table.
        Connection::open(root.join("library.sqlite3"))?
            .execute_batch("DROP TABLE preset_images;")?;
        let library = Library::open(&root)?;
        assert_eq!(library.presets()?.len(), 13);
        let jpg = temp.0.join("portrait.jpg");
        RgbImage::from_pixel(24, 48, Rgb([130, 80, 20])).save(&jpg)?;
        let first = library.set_preset_image("custom.marine", &jpg)?;
        let webp = temp.0.join("portrait.webp");
        RgbaImage::from_pixel(32, 16, Rgba([10, 40, 120, 255])).save(&webp)?;
        let second = library.set_preset_image("custom.marine", &webp)?;
        assert_ne!(first.path, second.path);
        assert!(!first.path.exists());
        assert!(second.path.is_file());
        library.clear_preset_image("custom.marine")?;
        library.clear_preset_image("custom.marine")?;
        assert!(!second.path.exists());
        assert!(library.preset_image("custom.marine")?.is_none());
        Ok(())
    }

    #[test]
    fn invalid_limited_and_database_failed_imports_preserve_the_previous_image() -> Result<()> {
        let temp = Temp::new();
        let library = Library::open(&temp.0.join("library"))?;
        save_custom(&library)?;
        let source = temp.0.join("valid.png");
        RgbaImage::from_pixel(16, 16, Rgba([50, 60, 70, 255])).save(&source)?;
        let original = library.set_preset_image("custom.marine", &source)?;
        let original_bytes = fs::read(&original.path)?;

        let invalid = temp.0.join("invalid.png");
        fs::write(&invalid, b"not a PNG")?;
        let truncated = temp.0.join("truncated.png");
        let valid_bytes = fs::read(&source)?;
        fs::write(&truncated, &valid_bytes[..valid_bytes.len() / 2])?;
        let too_wide = temp.0.join("too-wide.png");
        RgbaImage::from_pixel(MAX_SOURCE_SIDE + 1, 1, Rgba([0, 0, 0, 255])).save(&too_wide)?;
        let too_large = temp.0.join("too-large.png");
        File::create(&too_large)?.set_len(MAX_SOURCE_BYTES + 1)?;
        for path in [&invalid, &truncated, &too_wide, &too_large, &temp.0] {
            assert!(library.set_preset_image("custom.marine", path).is_err());
            assert_eq!(
                library.preset_image("custom.marine")?,
                Some(original.clone())
            );
            assert_eq!(fs::read(&original.path)?, original_bytes);
        }
        for id in ["builtin.clean", "custom.missing", "../outside"] {
            assert!(library.set_preset_image(id, &source).is_err());
            assert!(library.clear_preset_image(id).is_err());
        }
        // Force a commit failure after PNG creation. No unreferenced image should remain.
        library.db.execute_batch(
            "CREATE TRIGGER reject_artwork_update BEFORE UPDATE ON preset_images
             BEGIN SELECT RAISE(FAIL, 'test write failure'); END;",
        )?;
        assert!(library.set_preset_image("custom.marine", &source).is_err());
        assert_eq!(
            library.preset_image("custom.marine")?,
            Some(original.clone())
        );
        assert_eq!(fs::read(&original.path)?, original_bytes);
        assert_eq!(fs::read_dir(library.root.join("artwork"))?.count(), 1);
        Ok(())
    }
}
