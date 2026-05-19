use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::LazyLock;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_utils_cache::BlockingLruCache;
use codex_utils_cache::sha1_digest;
use image::ColorType;
use image::DynamicImage;
use image::GenericImageView;
use image::ImageEncoder;
use image::ImageFormat;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
/// Maximum width or height used when resizing images before uploading.
pub const MAX_DIMENSION: u32 = 2048;

pub mod error;

pub use crate::error::ImageProcessingError;

#[derive(Debug, Clone)]
pub struct EncodedImage {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub width: u32,
    pub height: u32,
}

impl EncodedImage {
    pub fn into_data_url(self) -> String {
        let encoded = BASE64_STANDARD.encode(&self.bytes);
        format!("data:{};base64,{encoded}", self.mime)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptImageMode {
    ResizeToFit,
    Original,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PromptImageUrlError {
    #[error("image URL must not be empty")]
    EmptyUrl,
    #[error("image data URL must include metadata and a payload")]
    MissingDataUrlSeparator,
    #[error("image data URL must use an image MIME type")]
    NonImageDataUrl,
    #[error("image data URL must use base64 encoding")]
    MissingBase64Marker,
    #[error("image data URL must include base64 payload bytes")]
    EmptyBase64Payload,
    #[error("image data URL payload must be valid base64")]
    InvalidBase64Payload,
    #[error("image data URL payload must decode to a valid image")]
    InvalidImageBytes,
}

pub fn validate_prompt_image_url(image_url: &str) -> Result<(), PromptImageUrlError> {
    if image_url.trim().is_empty() {
        return Err(PromptImageUrlError::EmptyUrl);
    }

    if !image_url
        .get(.."data:".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
    {
        return Ok(());
    }

    let (metadata, payload) = image_url
        .split_once(',')
        .ok_or(PromptImageUrlError::MissingDataUrlSeparator)?;
    let metadata_without_scheme = &metadata["data:".len()..];
    let mut metadata_parts = metadata_without_scheme.split(';');
    let mime_type = metadata_parts.next().unwrap_or_default();
    if !mime_type
        .get(.."image/".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
    {
        return Err(PromptImageUrlError::NonImageDataUrl);
    }
    if !metadata_parts.any(|part| part.eq_ignore_ascii_case("base64")) {
        return Err(PromptImageUrlError::MissingBase64Marker);
    }
    if payload.trim().is_empty() {
        return Err(PromptImageUrlError::EmptyBase64Payload);
    }

    let bytes = BASE64_STANDARD
        .decode(payload)
        .map_err(|_| PromptImageUrlError::InvalidBase64Payload)?;
    if bytes.is_empty() {
        return Err(PromptImageUrlError::EmptyBase64Payload);
    }
    image::load_from_memory(&bytes).map_err(|_| PromptImageUrlError::InvalidImageBytes)?;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ImageCacheKey {
    digest: [u8; 20],
    mode: PromptImageMode,
}

static IMAGE_CACHE: LazyLock<BlockingLruCache<ImageCacheKey, EncodedImage>> =
    LazyLock::new(|| BlockingLruCache::new(NonZeroUsize::new(32).unwrap_or(NonZeroUsize::MIN)));

pub fn load_for_prompt_bytes(
    path: &Path,
    file_bytes: Vec<u8>,
    mode: PromptImageMode,
) -> Result<EncodedImage, ImageProcessingError> {
    let path_buf = path.to_path_buf();

    let key = ImageCacheKey {
        digest: sha1_digest(&file_bytes),
        mode,
    };

    IMAGE_CACHE.get_or_try_insert_with(key, move || {
        let format = match image::guess_format(&file_bytes) {
            Ok(ImageFormat::Png) => Some(ImageFormat::Png),
            Ok(ImageFormat::Jpeg) => Some(ImageFormat::Jpeg),
            Ok(ImageFormat::Gif) => Some(ImageFormat::Gif),
            Ok(ImageFormat::WebP) => Some(ImageFormat::WebP),
            _ => None,
        };

        let dynamic = image::load_from_memory(&file_bytes)
            .map_err(|source| ImageProcessingError::decode_error(&path_buf, source))?;

        let (width, height) = dynamic.dimensions();

        let encoded = if mode == PromptImageMode::Original
            || (width <= MAX_DIMENSION && height <= MAX_DIMENSION)
        {
            if let Some(format) = format.filter(|format| can_preserve_source_bytes(*format)) {
                let mime = format_to_mime(format);
                EncodedImage {
                    bytes: file_bytes,
                    mime,
                    width,
                    height,
                }
            } else {
                let (bytes, output_format) = encode_image(&dynamic, ImageFormat::Png)?;
                let mime = format_to_mime(output_format);
                EncodedImage {
                    bytes,
                    mime,
                    width,
                    height,
                }
            }
        } else {
            let resized = dynamic.resize(MAX_DIMENSION, MAX_DIMENSION, FilterType::Triangle);
            let target_format = format
                .filter(|format| can_preserve_source_bytes(*format))
                .unwrap_or(ImageFormat::Png);
            let (bytes, output_format) = encode_image(&resized, target_format)?;
            let mime = format_to_mime(output_format);
            EncodedImage {
                bytes,
                mime,
                width: resized.width(),
                height: resized.height(),
            }
        };

        Ok(encoded)
    })
}

fn can_preserve_source_bytes(format: ImageFormat) -> bool {
    // Public API docs explicitly call out non-animated GIF support only.
    // Preserve byte-for-byte only for formats we can safely pass through.
    matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    )
}

fn encode_image(
    image: &DynamicImage,
    preferred_format: ImageFormat,
) -> Result<(Vec<u8>, ImageFormat), ImageProcessingError> {
    let target_format = match preferred_format {
        ImageFormat::Jpeg => ImageFormat::Jpeg,
        ImageFormat::WebP => ImageFormat::WebP,
        _ => ImageFormat::Png,
    };

    let mut buffer = Vec::new();

    match target_format {
        ImageFormat::Png => {
            let rgba = image.to_rgba8();
            let encoder = PngEncoder::new(&mut buffer);
            encoder
                .write_image(
                    rgba.as_raw(),
                    image.width(),
                    image.height(),
                    ColorType::Rgba8.into(),
                )
                .map_err(|source| ImageProcessingError::Encode {
                    format: target_format,
                    source,
                })?;
        }
        ImageFormat::Jpeg => {
            let mut encoder = JpegEncoder::new_with_quality(&mut buffer, 85);
            encoder
                .encode_image(image)
                .map_err(|source| ImageProcessingError::Encode {
                    format: target_format,
                    source,
                })?;
        }
        ImageFormat::WebP => {
            let rgba = image.to_rgba8();
            let encoder = WebPEncoder::new_lossless(&mut buffer);
            encoder
                .write_image(
                    rgba.as_raw(),
                    image.width(),
                    image.height(),
                    ColorType::Rgba8.into(),
                )
                .map_err(|source| ImageProcessingError::Encode {
                    format: target_format,
                    source,
                })?;
        }
        _ => unreachable!("unsupported target_format should have been handled earlier"),
    }

    Ok((buffer, target_format))
}

fn format_to_mime(format: ImageFormat) -> String {
    match format {
        ImageFormat::Jpeg => "image/jpeg".to_string(),
        ImageFormat::Gif => "image/gif".to_string(),
        ImageFormat::WebP => "image/webp".to_string(),
        _ => "image/png".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use image::GenericImageView;
    use image::ImageBuffer;
    use image::Rgba;

    fn image_bytes(image: &ImageBuffer<Rgba<u8>, Vec<u8>>, format: ImageFormat) -> Vec<u8> {
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut encoded, format)
            .expect("encode image to bytes");
        encoded.into_inner()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn returns_original_image_when_within_bounds() {
        for (format, mime) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let image = ImageBuffer::from_pixel(64, 32, Rgba([10u8, 20, 30, 255]));
            let original_bytes = image_bytes(&image, format);

            let encoded = load_for_prompt_bytes(
                Path::new("in-memory-image"),
                original_bytes.clone(),
                PromptImageMode::ResizeToFit,
            )
            .expect("process image");

            assert_eq!(encoded.width, 64);
            assert_eq!(encoded.height, 32);
            assert_eq!(encoded.mime, mime);
            assert_eq!(encoded.bytes, original_bytes);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downscales_large_image() {
        for (format, mime) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let image = ImageBuffer::from_pixel(4096, 2048, Rgba([200u8, 10, 10, 255]));
            let original_bytes = image_bytes(&image, format);

            let processed = load_for_prompt_bytes(
                Path::new("in-memory-image"),
                original_bytes,
                PromptImageMode::ResizeToFit,
            )
            .expect("process image");

            assert!(processed.width <= MAX_DIMENSION);
            assert!(processed.height <= MAX_DIMENSION);
            assert_eq!(processed.mime, mime);

            let detected_format =
                image::guess_format(&processed.bytes).expect("detect resized output format");
            assert_eq!(detected_format, format);

            let loaded = image::load_from_memory(&processed.bytes)
                .expect("read resized bytes back into image");
            assert_eq!(loaded.dimensions(), (processed.width, processed.height));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downscales_tall_image_to_fit_square_bounds() {
        let image = ImageBuffer::from_pixel(1024, 4096, Rgba([200u8, 10, 10, 255]));
        let original_bytes = image_bytes(&image, ImageFormat::Png);

        let processed = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            original_bytes,
            PromptImageMode::ResizeToFit,
        )
        .expect("process image");

        assert_eq!(processed.width, 512);
        assert_eq!(processed.height, MAX_DIMENSION);
        assert_eq!(processed.mime, "image/png");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn preserves_large_image_in_original_mode() {
        let image = ImageBuffer::from_pixel(4096, 2048, Rgba([180u8, 30, 30, 255]));
        let original_bytes = image_bytes(&image, ImageFormat::Png);

        let processed = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            original_bytes.clone(),
            PromptImageMode::Original,
        )
        .expect("process image");

        assert_eq!(processed.width, 4096);
        assert_eq!(processed.height, 2048);
        assert_eq!(processed.mime, "image/png");
        assert_eq!(processed.bytes, original_bytes);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_cleanly_for_invalid_images() {
        let err = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            b"not an image".to_vec(),
            PromptImageMode::ResizeToFit,
        )
        .expect_err("invalid image should fail");
        assert!(matches!(
            err,
            ImageProcessingError::Decode { .. }
                | ImageProcessingError::UnsupportedImageFormat { .. }
        ));
    }

    #[test]
    fn validate_prompt_image_url_accepts_remote_and_inline_images() {
        let image = ImageBuffer::from_pixel(1, 1, Rgba([10u8, 20, 30, 255]));
        let payload = BASE64_STANDARD.encode(image_bytes(&image, ImageFormat::Png));
        let data_url = format!("data:image/png;base64,{payload}");

        assert_eq!(
            validate_prompt_image_url("https://example.com/image.png"),
            Ok(())
        );
        assert_eq!(validate_prompt_image_url(&data_url), Ok(()));
    }

    #[test]
    fn validate_prompt_image_url_rejects_malformed_inline_images() {
        assert_eq!(
            validate_prompt_image_url("   "),
            Err(PromptImageUrlError::EmptyUrl)
        );
        assert_eq!(
            validate_prompt_image_url("data:image/png;base64"),
            Err(PromptImageUrlError::MissingDataUrlSeparator)
        );
        assert_eq!(
            validate_prompt_image_url("data:text/plain;base64,aGVsbG8="),
            Err(PromptImageUrlError::NonImageDataUrl)
        );
        assert_eq!(
            validate_prompt_image_url("data:image/png,aGVsbG8="),
            Err(PromptImageUrlError::MissingBase64Marker)
        );
        assert_eq!(
            validate_prompt_image_url("data:image/png;base64,"),
            Err(PromptImageUrlError::EmptyBase64Payload)
        );
        assert_eq!(
            validate_prompt_image_url("data:image/png;base64,%%%"),
            Err(PromptImageUrlError::InvalidBase64Payload)
        );
        assert_eq!(
            validate_prompt_image_url("data:image/png;base64,aGVsbG8="),
            Err(PromptImageUrlError::InvalidImageBytes)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reprocesses_updated_file_contents() {
        {
            IMAGE_CACHE.clear();
        }

        let first_image = ImageBuffer::from_pixel(32, 16, Rgba([20u8, 120, 220, 255]));
        let first_bytes = image_bytes(&first_image, ImageFormat::Png);

        let first = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            first_bytes,
            PromptImageMode::ResizeToFit,
        )
        .expect("process first image");

        let second_image = ImageBuffer::from_pixel(96, 48, Rgba([50u8, 60, 70, 255]));
        let second_bytes = image_bytes(&second_image, ImageFormat::Png);

        let second = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            second_bytes,
            PromptImageMode::ResizeToFit,
        )
        .expect("process updated image");

        assert_eq!(first.width, 32);
        assert_eq!(first.height, 16);
        assert_eq!(second.width, 96);
        assert_eq!(second.height, 48);
        assert_ne!(second.bytes, first.bytes);
    }
}
