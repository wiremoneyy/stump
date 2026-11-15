use std::fs;

use image::{imageops, DynamicImage, EncodableLayout, GenericImageView};
use models::shared::image_processor_options::{
	Dimension, ImageProcessorOptions, ImageResizeMethod, ScaledDimensionResize,
};
use webp::Encoder;

use crate::image::{
	error::ImageProcessorError,
	process::{resized_dimensions, ImageProcessor},
	scale_height_dimension, scale_width_dimension,
};

pub struct WebpProcessor;

impl ImageProcessor for WebpProcessor {
	fn generate(
		buffer: &[u8],
		options: ImageProcessorOptions,
	) -> Result<Vec<u8>, ImageProcessorError> {
		let mut image = image::load_from_memory(buffer)?;

		if let Some(method) = options.resize_method {
			let resized_image = WebpProcessor::resize_image(image, method);
			image = resized_image;
		}

		let mut result = WebpProcessor::encode(image)?;
		result.shrink_to_fit(); // free excess capacity

		Ok(result)
	}

	fn generate_from_path(
		path: &str,
		options: ImageProcessorOptions,
	) -> Result<Vec<u8>, ImageProcessorError> {
		let bytes = fs::read(path)?;
		Self::generate(&bytes, options)
	}

	fn resize_scaled(
		buf: &[u8],
		config: ScaledDimensionResize,
	) -> Result<Vec<u8>, ImageProcessorError> {
		let image = image::load_from_memory(buf)?;

		let (current_width, current_height) = image.dimensions();

		let (width, height) = match config.dimension {
			Dimension::Width => scale_height_dimension(
				current_width as f32,
				current_height as f32,
				config.size as f32,
			),
			Dimension::Height => scale_width_dimension(
				current_width as f32,
				current_height as f32,
				config.size as f32,
			),
		};

		let resized_image =
			image.resize(width, height, image::imageops::FilterType::Lanczos3);

		WebpProcessor::encode(resized_image)
	}
}

/// The largest width or height the WebP format can store
const WEBP_MAX_DIMENSION: u32 = 16383;

impl WebpProcessor {
	fn encode(mut image: DynamicImage) -> Result<Vec<u8>, ImageProcessorError> {
		// Note: Long-strip pages (e.g. webtoons) can be taller than WebP allows, so they are
		// scaled down to fit, keeping the aspect ratio
		let (width, height) = image.dimensions();
		if width > WEBP_MAX_DIMENSION || height > WEBP_MAX_DIMENSION {
			image = image.resize(
				WEBP_MAX_DIMENSION,
				WEBP_MAX_DIMENSION,
				imageops::FilterType::Triangle,
			);
		}

		let encoder = Encoder::from_image(&image)
			.map_err(|err| ImageProcessorError::WebpEncode(err.to_string()))?;
		// Note: `Encoder::encode` panics if the encoder rejects the image
		let encoded_webp = encoder
			.encode_simple(false, 100f32)
			.map_err(|err| ImageProcessorError::WebpEncode(format!("{err:?}")))?;

		Ok(encoded_webp.as_bytes().to_vec())
	}

	fn resize_image(
		image: DynamicImage,
		resize_method: ImageResizeMethod,
	) -> DynamicImage {
		let (current_width, current_height) = image.dimensions();
		let (height, width) =
			resized_dimensions(current_height, current_width, resize_method);
		DynamicImage::ImageRgba8(imageops::resize(
			&image,
			width,
			height,
			// TODO: determine best filter or allow user to specify
			imageops::FilterType::Triangle,
		))
	}
}

#[cfg(test)]
mod tests {
	use models::shared::image_processor_options::{
		ExactDimensionResize, ScaleEvenlyByFactor, SupportedImageFormat,
	};
	use rust_decimal::Decimal;

	use super::*;
	use crate::image::tests::{get_test_jpg_path, get_test_png_path, get_test_webp_path};
	use std::fs;

	#[test]
	fn test_generate_webp_from_webp_data() {
		let bytes = get_test_webp_data();
		let options = ImageProcessorOptions {
			resize_method: None,
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let result = WebpProcessor::generate(&bytes, options);
		assert!(result.is_ok());

		let webp_bytes = result.unwrap();
		// should *still* be a valid webp image
		assert!(image::load_from_memory_with_format(
			&webp_bytes,
			image::ImageFormat::WebP
		)
		.is_ok());
	}

	#[test]
	fn test_generate_webp_from_jpg() {
		let jpg_path = get_test_jpg_path();
		let options = ImageProcessorOptions {
			resize_method: None,
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let result = WebpProcessor::generate_from_path(&jpg_path, options);
		assert!(result.is_ok());

		let webp_bytes = result.unwrap();
		// should be a valid webp image
		assert!(image::load_from_memory_with_format(
			&webp_bytes,
			image::ImageFormat::WebP
		)
		.is_ok());
	}

	#[test]
	fn test_generate_webp_from_jpg_with_even_rescale() {
		let jpg_path = get_test_jpg_path();
		let options = ImageProcessorOptions {
			resize_method: Some(ImageResizeMethod::ScaleEvenlyByFactor(
				ScaleEvenlyByFactor {
					factor: Decimal::new(5, 1),
				},
			)),
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let current_dimensions =
			image::image_dimensions(&jpg_path).expect("Failed to get dimensions");

		let buffer = WebpProcessor::generate_from_path(&jpg_path, options)
			.expect("Failed to generate image buffer");

		let dimensions = image::load_from_memory(&buffer)
			.expect("Failed to load image from buffer")
			.dimensions();

		assert_eq!(dimensions.0, (current_dimensions.0 as f32 * 0.5) as u32);
		assert_eq!(dimensions.1, (current_dimensions.1 as f32 * 0.5) as u32);
	}

	#[test]
	fn test_generate_webp_from_jpg_with_exact_resize() {
		let jpg_path = get_test_jpg_path();
		let options = ImageProcessorOptions {
			resize_method: Some(ImageResizeMethod::Exact(ExactDimensionResize {
				width: 100,
				height: 100,
			})),
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let buffer = WebpProcessor::generate_from_path(&jpg_path, options)
			.expect("Failed to generate image buffer");

		let dimensions = image::load_from_memory(&buffer)
			.expect("Failed to load image from buffer")
			.dimensions();

		assert_eq!(dimensions.0, 100);
		assert_eq!(dimensions.1, 100);
	}

	#[test]
	fn test_generate_webp_from_png() {
		let png_path = get_test_png_path();
		let options = ImageProcessorOptions {
			resize_method: None,
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let result = WebpProcessor::generate_from_path(&png_path, options);
		assert!(result.is_ok());

		let webp_bytes = result.unwrap();
		// should be a valid webp image
		assert!(image::load_from_memory_with_format(
			&webp_bytes,
			image::ImageFormat::WebP
		)
		.is_ok());
	}

	#[test]
	fn test_generate_webp_from_png_with_rescale() {
		let png_path = get_test_png_path();
		let options = ImageProcessorOptions {
			resize_method: Some(ImageResizeMethod::ScaleEvenlyByFactor(
				ScaleEvenlyByFactor {
					factor: Decimal::new(5, 1),
				},
			)),
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let current_dimensions =
			image::image_dimensions(&png_path).expect("Failed to get dimensions");

		let buffer = WebpProcessor::generate_from_path(&png_path, options)
			.expect("Failed to generate image buffer");

		let dimensions = image::load_from_memory(&buffer)
			.expect("Failed to load image from buffer")
			.dimensions();

		assert_eq!(dimensions.0, (current_dimensions.0 as f32 * 0.5) as u32);
		assert_eq!(dimensions.1, (current_dimensions.1 as f32 * 0.5) as u32);
	}

	#[test]
	fn test_generate_webp_from_png_with_resize() {
		let png_path = get_test_png_path();
		let options = ImageProcessorOptions {
			resize_method: Some(ImageResizeMethod::Exact(ExactDimensionResize {
				width: 100,
				height: 100,
			})),
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let buffer = WebpProcessor::generate_from_path(&png_path, options)
			.expect("Failed to generate image buffer");

		let dimensions = image::load_from_memory(&buffer)
			.expect("Failed to load image from buffer")
			.dimensions();

		assert_eq!(dimensions.0, 100);
		assert_eq!(dimensions.1, 100);
	}

	#[test]
	fn test_generate_webp_from_webp() {
		let webp_path = get_test_webp_path();
		let options = ImageProcessorOptions {
			resize_method: None,
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		WebpProcessor::generate_from_path(&webp_path, options).unwrap();
	}

	#[test]
	fn test_generate_webp_from_webp_with_rescale() {
		let webp_path = get_test_webp_path();
		let options = ImageProcessorOptions {
			resize_method: Some(ImageResizeMethod::ScaleEvenlyByFactor(
				ScaleEvenlyByFactor {
					factor: Decimal::new(5, 1),
				},
			)),
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let current_dimensions =
			image::image_dimensions(&webp_path).expect("Failed to get dimensions");

		let buffer = WebpProcessor::generate_from_path(&webp_path, options)
			.expect("Failed to generate image buffer");

		let dimensions = image::load_from_memory(&buffer)
			.expect("Failed to load image from buffer")
			.dimensions();

		assert_eq!(dimensions.0, (current_dimensions.0 as f32 * 0.5) as u32);
		assert_eq!(dimensions.1, (current_dimensions.1 as f32 * 0.5) as u32);
	}

	#[test]
	fn test_generate_webp_from_webp_with_resize() {
		let webp_path = get_test_webp_path();
		let options = ImageProcessorOptions {
			resize_method: Some(ImageResizeMethod::Exact(ExactDimensionResize {
				width: 100,
				height: 100,
			})),
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let buffer = WebpProcessor::generate_from_path(&webp_path, options)
			.expect("Failed to generate image buffer");

		let dimensions = image::load_from_memory(&buffer)
			.expect("Failed to load image from buffer")
			.dimensions();

		assert_eq!(dimensions.0, 100);
		assert_eq!(dimensions.1, 100);
	}

	#[test]
	fn test_generate_webp_from_image_taller_than_webp_allows() {
		let tall = DynamicImage::new_rgb8(8, WEBP_MAX_DIMENSION + 1000);
		let mut png = std::io::Cursor::new(Vec::new());
		tall.write_to(&mut png, image::ImageFormat::Png).unwrap();
		let options = ImageProcessorOptions {
			resize_method: None,
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		let buffer = WebpProcessor::generate(png.get_ref(), options)
			.expect("Failed to generate image buffer");

		let dimensions = image::load_from_memory(&buffer)
			.expect("Failed to load image from buffer")
			.dimensions();
		assert_eq!(dimensions.1, WEBP_MAX_DIMENSION);
	}

	#[test]
	fn test_generate_webp_with_empty_dimension_is_an_error() {
		let options = ImageProcessorOptions {
			resize_method: Some(ImageResizeMethod::Exact(ExactDimensionResize {
				width: 0,
				height: 100,
			})),
			format: SupportedImageFormat::Webp,
			quality: None,
			page: None,
		};

		assert!(WebpProcessor::generate(&get_test_webp_data(), options).is_err());
	}

	fn get_test_webp_data() -> Vec<u8> {
		let path = get_test_webp_path();
		fs::read(path).expect("Failed to fetch example webp image")
	}
}
