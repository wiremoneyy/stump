use std::{
	path::{Path, PathBuf},
	time::{Duration, SystemTime},
};

use models::{
	entity::{library_config, series},
	shared::{
		enums::LibraryPattern,
		image_processor_options::{
			ExactDimensionResize, ImageProcessorOptions, ImageResizeMethod,
			SupportedImageFormat,
		},
	},
};
use sea_orm::{prelude::*, ActiveValue::Set};
use stump_core::image::{is_generated_thumbnail, series_cover_thumbnail_tag};

use super::common::TestLibrary;

const SERIES: &str = "Absolute Necromancer";

fn write_image(path: &Path, color: [u8; 3]) {
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	image::RgbImage::from_pixel(8, 12, image::Rgb(color))
		.save(path)
		.expect("failed to write image");
}

/// bumps the mtime of a file into the future so it is newer than any thumbnail written during
/// the test, simulating a cover file being replaced
fn touch_future(path: &Path) {
	set_mtime(path, SystemTime::now() + Duration::from_secs(60));
}

fn set_mtime(path: &Path, modified: SystemTime) {
	std::fs::File::options()
		.write(true)
		.open(path)
		.unwrap()
		.set_modified(modified)
		.unwrap();
}

/// the layout from the feature request: a top-level collections library where each series
/// folder has numbered chapter folders, a cover file and a desc.json
async fn collections_library() -> TestLibrary {
	let library = TestLibrary::new().await;
	library
		.update_config(library_config::ActiveModel {
			library_pattern: Set(LibraryPattern::CollectionBased),
			..Default::default()
		})
		.await;
	library.add_epub(&format!("{SERIES}/1"), "Chapter 0001");
	library.add_epub(&format!("{SERIES}/2"), "Chapter 0002");
	std::fs::write(library.path(&format!("{SERIES}/desc.json")), b"{}").unwrap();
	library
}

impl TestLibrary {
	async fn series_named(&self, relative: &str) -> series::Model {
		self.snapshot()
			.await
			.series_at(&self.path_str(relative))
			.cloned()
			.unwrap_or_else(|| panic!("series {relative} should exist"))
	}

	fn thumbnails_dir(&self) -> PathBuf {
		self.state.config.thumbnails_directory()
	}

	async fn set_thumbnail_config(&self, config: Option<ImageProcessorOptions>) {
		self.update_config(library_config::ActiveModel {
			thumbnail_config: Set(config),
			..Default::default()
		})
		.await;
	}
}

fn cover_tag(series: &series::Model) -> Option<String> {
	series
		.thumbnail_path
		.as_deref()
		.and_then(|p| series_cover_thumbnail_tag(Path::new(p)))
		.map(str::to_string)
}

fn small_webp_options() -> ImageProcessorOptions {
	ImageProcessorOptions {
		resize_method: Some(ImageResizeMethod::Exact(ExactDimensionResize {
			width: 4,
			height: 6,
		})),
		format: SupportedImageFormat::Webp,
		quality: None,
		page: None,
	}
}

#[tokio::test]
async fn cover_file_is_used_as_thumbnail_and_never_ingested() {
	let library = collections_library().await;
	let cover = library.path(&format!("{SERIES}/cover.png"));
	write_image(&cover, [255, 0, 0]);
	// lower priority candidates in the root, and a higher priority name nested in a chapter
	// folder which must be ignored
	write_image(&library.path(&format!("{SERIES}/poster.jpg")), [0, 0, 255]);
	write_image(
		&library.path(&format!("{SERIES}/1/cover.webp")),
		[0, 255, 0],
	);

	library.scan().await;

	let snapshot = library.snapshot().await;
	snapshot.assert_unique_media(2);
	assert!(
		snapshot.media.iter().all(|m| m.extension == "epub"),
		"cover images should never be ingested as media\n{snapshot:#?}"
	);

	let series = library.series_named(SERIES).await;
	assert_eq!(cover_tag(&series).as_deref(), Some("cover-png"));
	// thumbnail generation is disabled, so the file is stored as-is
	let thumbnail_path = series.thumbnail_path.clone().unwrap();
	assert_eq!(
		std::fs::read(&thumbnail_path).unwrap(),
		std::fs::read(&cover).unwrap()
	);

	library.assert_rescan_is_noop().await;
}

#[tokio::test]
async fn cover_file_changes_are_detected() {
	let library = collections_library().await;
	let cover = library.path(&format!("{SERIES}/Cover.PNG"));
	let folder = library.path(&format!("{SERIES}/folder.jpg"));
	write_image(&cover, [255, 0, 0]);
	write_image(&folder, [0, 0, 255]);

	library.scan().await;
	let initial = library.series_named(SERIES).await;
	assert_eq!(cover_tag(&initial).as_deref(), Some("cover-png"));

	// replaced in place (newer mtime)
	write_image(&cover, [0, 255, 0]);
	touch_future(&cover);
	let output = library.scan().await;
	assert_eq!(output.updated_series, 1, "{output:#?}");
	let replaced = library.series_named(SERIES).await;
	assert_eq!(
		std::fs::read(replaced.thumbnail_path.as_deref().unwrap()).unwrap(),
		std::fs::read(&cover).unwrap()
	);
	assert!(
		replaced.updated_at > initial.updated_at,
		"URL should be busted"
	);

	// replaced in place by a file with an older mtime (e.g. a copy which kept its mtime)
	write_image(&cover, [0, 0, 255]);
	set_mtime(&cover, SystemTime::now() - Duration::from_secs(3600));
	let output = library.scan().await;
	assert_eq!(output.updated_series, 1, "{output:#?}");
	let replaced = library.series_named(SERIES).await;
	assert_eq!(
		std::fs::read(replaced.thumbnail_path.as_deref().unwrap()).unwrap(),
		std::fs::read(&cover).unwrap()
	);
	library.assert_rescan_is_noop().await;

	// highest priority removed, the next candidate takes over even though it is older
	std::fs::remove_file(&cover).unwrap();
	library.scan().await;
	let fallback = library.series_named(SERIES).await;
	assert_eq!(cover_tag(&fallback).as_deref(), Some("folder-jpg"));
	assert!(
		!Path::new(replaced.thumbnail_path.as_deref().unwrap()).exists(),
		"the stale cover thumbnail should be removed"
	);

	// all covers removed, back to the default behavior
	std::fs::remove_file(&folder).unwrap();
	library.scan().await;
	let reverted = library.series_named(SERIES).await;
	assert_eq!(reverted.thumbnail_path, None);
	assert!(!Path::new(fallback.thumbnail_path.as_deref().unwrap()).exists());

	library.assert_rescan_is_noop().await;
}

#[tokio::test]
async fn manual_thumbnail_is_never_overwritten() {
	let library = collections_library().await;
	write_image(&library.path(&format!("{SERIES}/cover.png")), [255, 0, 0]);
	library.scan().await;

	let series = library.series_named(SERIES).await;
	let manual = library.thumbnails_dir().join(format!("{}.png", series.id));
	write_image(&manual, [1, 2, 3]);
	let manual = manual.to_string_lossy().to_string();
	series::Entity::update_many()
		.filter(series::Column::Id.eq(&series.id))
		.col_expr(
			series::Column::ThumbnailPath,
			Expr::value(Some(manual.clone())),
		)
		.exec(library.conn())
		.await
		.unwrap();

	touch_future(&library.path(&format!("{SERIES}/cover.png")));
	library.scan().await;
	assert_eq!(
		library.series_named(SERIES).await.thumbnail_path.as_deref(),
		Some(manual.as_str())
	);

	// nor by a forced regeneration
	library
		.set_thumbnail_config(Some(small_webp_options()))
		.await;
	library
		.generate_series_thumbnails(vec![series.id.clone()], small_webp_options(), true)
		.await;
	assert_eq!(
		library.series_named(SERIES).await.thumbnail_path.as_deref(),
		Some(manual.as_str())
	);
	assert!(Path::new(&manual).exists());
}

#[tokio::test]
async fn disabling_the_option_reverts_to_default_behavior() {
	let library = collections_library().await;
	write_image(&library.path(&format!("{SERIES}/cover.png")), [255, 0, 0]);
	library.scan().await;
	assert!(cover_tag(&library.series_named(SERIES).await).is_some());

	library
		.update_config(library_config::ActiveModel {
			use_series_cover_image: Set(false),
			..Default::default()
		})
		.await;
	library.scan().await;
	assert_eq!(library.series_named(SERIES).await.thumbnail_path, None);
	library.assert_rescan_is_noop().await;
}

#[tokio::test]
async fn series_based_libraries_use_the_folder_containing_books() {
	let library = TestLibrary::new().await;
	library.add_epub("Series A", "a");
	write_image(&library.path("Series A/folder.png"), [255, 0, 0]);
	// a folder with only a cover image is not a series
	write_image(&library.path("Just A Cover/cover.png"), [0, 255, 0]);

	library.scan().await;

	let snapshot = library.snapshot().await;
	snapshot.assert_unique_media(1);
	assert!(snapshot
		.series_at(&library.path_str("Just A Cover"))
		.is_none());
	assert_eq!(
		cover_tag(&library.series_named("Series A").await).as_deref(),
		Some("folder-png")
	);
}

#[tokio::test]
async fn cover_file_goes_through_thumbnail_settings() {
	let library = collections_library().await;
	library
		.set_thumbnail_config(Some(small_webp_options()))
		.await;
	let cover = library.path(&format!("{SERIES}/cover.png"));
	write_image(&cover, [255, 0, 0]);

	library.scan().await;
	let series = library.series_named(SERIES).await;
	// the scan defers the processing to a (follow-up) thumbnail generation job
	let logs = library
		.generate_series_thumbnails(vec![series.id.clone()], small_webp_options(), false)
		.await;
	assert!(logs.is_empty(), "{logs:?}");

	let series = library.series_named(SERIES).await;
	assert_eq!(cover_tag(&series).as_deref(), Some("cover-png"));
	let thumbnail_path = series.thumbnail_path.clone().unwrap();
	assert!(thumbnail_path.ends_with(".webp"), "{thumbnail_path}");
	let thumbnail = image::open(&thumbnail_path).unwrap();
	assert_eq!((thumbnail.width(), thumbnail.height()), (4, 6));
	assert!(
		series.thumbnail_meta.is_some(),
		"placeholder colors expected"
	);

	library.assert_rescan_is_noop().await;

	// a forced regeneration refreshes generated thumbnails from the cover file
	write_image(&cover, [0, 255, 0]);
	library
		.generate_series_thumbnails(vec![series.id.clone()], small_webp_options(), true)
		.await;
	let regenerated = library.series_named(SERIES).await;
	assert_eq!(cover_tag(&regenerated).as_deref(), Some("cover-png"));
	assert!(
		regenerated.updated_at > series.updated_at,
		"URL should be busted"
	);
}

#[tokio::test]
async fn corrupt_cover_falls_back_without_failing() {
	let library = collections_library().await;
	library
		.set_thumbnail_config(Some(small_webp_options()))
		.await;
	std::fs::write(
		library.path(&format!("{SERIES}/cover.jpg")),
		b"not an image",
	)
	.unwrap();

	// the scan itself must not fail
	library.scan().await;

	let series = library.series_named(SERIES).await;
	library
		.generate_series_thumbnails(vec![series.id.clone()], small_webp_options(), false)
		.await;

	// falls back to the first book, i.e. a generated thumbnail which isn't from the cover
	let series = library.series_named(SERIES).await;
	assert_eq!(cover_tag(&series), None);
	if let Some(path) = series.thumbnail_path.as_deref() {
		assert!(is_generated_thumbnail(Path::new(path)), "{path}");
	}
}
