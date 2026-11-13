//! Support for using a cover image file which lives directly in a series folder (e.g. `cover.jpg`)
//! as the thumbnail for that series.
//!
//! The precedence for a series thumbnail is:
//!
//! 1. A thumbnail the user manually uploaded or selected (anything without the `.generated` marker)
//! 2. A cover image file found in the root of the series folder
//! 3. The first image of the first book in the series
//!
//! Thumbnails derived from a cover file are written to the thumbnails directory using the name
//! `{series_id}.seriescover-{stem}-{ext}.generated.{format}`. The `.generated` marker means they are
//! treated like any other generated thumbnail (i.e. safe to overwrite), and the `seriescover-*` tag
//! records which cover file the thumbnail was derived from so that scans can detect when the cover
//! file was added, replaced or removed without needing to persist any additional state.

use std::{
	path::{Path, PathBuf},
	time::SystemTime,
};

use chrono::Utc;
use models::{
	entity::series,
	shared::{
		image::ImageMetadata,
		image_processor_options::{ImageProcessorOptions, SupportedImageFormat},
	},
};
use sea_orm::{prelude::*, Condition, ConnectionTrait};
use tokio::{fs, task::spawn_blocking};

use crate::{
	config::StumpConfig,
	image::{
		is_generated_thumbnail,
		process::ImageProcessor,
		processor::{GenericImageProcessor, WebpProcessor},
		thumbnail::{
			generate::ThumbnailGenerateError,
			placeholder::{generate_image_metadata, generate_image_metadata_from_bytes},
		},
	},
};

/// The accepted base names (file stems) for a series cover file, in priority order. Matching
/// is case-insensitive.
pub const SERIES_COVER_FILE_STEMS: [&str; 3] = ["cover", "folder", "poster"];

/// The accepted extensions for a series cover file. Matching is case-insensitive. When multiple
/// files share the same stem (e.g. `cover.jpg` and `cover.png`), this order breaks the tie.
pub const SERIES_COVER_FILE_EXTENSIONS: [&str; 6] =
	["webp", "jpg", "jpeg", "png", "avif", "gif"];

/// Extensions which are accepted as cover files but can't be decoded by the image pipeline, so
/// they are always stored as-is rather than processed with the library's thumbnail settings.
const PASSTHROUGH_EXTENSIONS: [&str; 1] = ["avif"];

const COVER_THUMBNAIL_TAG_PREFIX: &str = "seriescover-";

/// Returns the priority of a file name as a series cover candidate, where a lower value is a
/// higher priority. Returns `None` if the file name is not a cover candidate.
pub fn series_cover_priority(file_name: &str) -> Option<usize> {
	let (stem, extension) = file_name.rsplit_once('.')?;
	let stem_rank = SERIES_COVER_FILE_STEMS
		.iter()
		.position(|s| stem.eq_ignore_ascii_case(s))?;
	let extension_rank = SERIES_COVER_FILE_EXTENSIONS
		.iter()
		.position(|e| extension.eq_ignore_ascii_case(e))?;
	Some(stem_rank * SERIES_COVER_FILE_EXTENSIONS.len() + extension_rank)
}

/// A small accumulator which keeps track of the best series cover candidate seen so far. This
/// allows callers which are already iterating a directory (e.g. the series walker) to find a
/// cover without an additional read of the directory.
#[derive(Debug, Default)]
pub struct SeriesCoverPicker {
	best: Option<(usize, String, PathBuf)>,
}

impl SeriesCoverPicker {
	/// Offer a path to the picker. The caller is responsible for only offering files which are
	/// direct children of the series root.
	pub fn offer(&mut self, path: &Path) {
		let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
			return;
		};
		let Some(priority) = series_cover_priority(file_name) else {
			return;
		};

		// Note: The file name is part of the key so that the result is deterministic on
		// case-sensitive filesystems which could have e.g. both `cover.jpg` and `Cover.jpg`
		let is_better = self
			.best
			.as_ref()
			.is_none_or(|(best_priority, best_name, _)| {
				(priority, file_name) < (*best_priority, best_name.as_str())
			});
		if is_better {
			self.best = Some((priority, file_name.to_string(), path.to_path_buf()));
		}
	}

	/// The highest priority path which was offered, if any
	pub fn finish(self) -> Option<PathBuf> {
		self.best.map(|(_, _, path)| path)
	}
}

/// A cover image file found in the root of a series folder
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesCoverFile {
	/// The full path to the cover file
	pub path: PathBuf,
	/// The modified time of the cover file, if it could be read
	pub modified: Option<SystemTime>,
}

impl SeriesCoverFile {
	/// Create a [`SeriesCoverFile`] from a path, reading the modified time from disk. This is
	/// blocking and should only be called from a blocking context.
	pub fn from_path_blocking(path: PathBuf) -> Self {
		let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
		Self { path, modified }
	}

	/// The (lowercased) extension of the cover file
	fn extension(&self) -> String {
		self.path
			.extension()
			.and_then(|e| e.to_str())
			.unwrap_or_default()
			.to_ascii_lowercase()
	}

	/// Whether the cover file can be decoded by the image pipeline. If not, it is stored as-is.
	pub fn is_processable(&self) -> bool {
		!PASSTHROUGH_EXTENSIONS.contains(&self.extension().as_str())
	}

	/// A short, filesystem-safe tag identifying which cover file a thumbnail was derived from,
	/// e.g. `cover-jpg` for `Cover.JPG`
	pub fn source_tag(&self) -> Option<String> {
		let file_name = self.path.file_name()?.to_str()?;
		series_cover_priority(file_name)?;
		let (stem, extension) = file_name.rsplit_once('.')?;
		Some(format!(
			"{}-{}",
			stem.to_ascii_lowercase(),
			extension.to_ascii_lowercase()
		))
	}

	/// The file stem (no extension) used for a thumbnail derived from this cover file
	pub fn thumbnail_file_stem(&self, series_id: &str) -> Option<String> {
		self.source_tag()
			.map(|tag| format!("{series_id}.{COVER_THUMBNAIL_TAG_PREFIX}{tag}.generated"))
	}
}

/// Finds the highest priority cover file directly within `dir`. Nested directories are never
/// searched. Returns `None` if `dir` is not a readable directory (e.g. a oneshot series, whose
/// path is the book file itself) or no cover file exists.
pub async fn find_series_cover_file(dir: &Path) -> Option<SeriesCoverFile> {
	let dir = dir.to_path_buf();
	spawn_blocking(move || {
		let read_dir = std::fs::read_dir(&dir).ok()?;
		let mut picker = SeriesCoverPicker::default();
		for entry in read_dir.filter_map(Result::ok) {
			let path = entry.path();
			// Note: is_file follows symlinks, so a symlinked cover file is still accepted
			if path.is_file() {
				picker.offer(&path);
			}
		}
		picker.finish().map(SeriesCoverFile::from_path_blocking)
	})
	.await
	.ok()
	.flatten()
}

/// If the provided thumbnail path is a thumbnail derived from a series cover file, returns the
/// source tag it was derived from (see [`SeriesCoverFile::source_tag`])
pub fn series_cover_thumbnail_tag(path: &Path) -> Option<&str> {
	if !is_generated_thumbnail(path) {
		return None;
	}
	let stem = path.file_stem()?.to_str()?;
	let (_, tag) = stem.strip_suffix(".generated")?.rsplit_once('.')?;
	tag.strip_prefix(COVER_THUMBNAIL_TAG_PREFIX)
}

/// The action a scan should take for a series thumbnail with respect to a series cover file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesCoverAction {
	/// Leave the current thumbnail alone
	Keep,
	/// (Re)create the series thumbnail from the cover file
	Apply,
	/// The current thumbnail was derived from a cover file which is no longer present (or the
	/// feature was disabled), so it should be dropped in favor of the default behavior
	Revert,
}

/// Determine what should happen to a series thumbnail given its current state and the cover file
/// (if any) discovered in the series folder. This is pure so it can be easily tested.
///
/// - `current_thumbnail`: the thumbnail path currently stored for the series
/// - `current_thumbnail_modified`: the modified time of that file, or `None` if it is missing
/// - `cover`: the cover file discovered in the series root, or `None` if there is none or the
///   feature is disabled for the library
pub fn resolve_series_cover_action(
	current_thumbnail: Option<&Path>,
	current_thumbnail_modified: Option<SystemTime>,
	cover: Option<&SeriesCoverFile>,
) -> SeriesCoverAction {
	// A manually uploaded or selected thumbnail always wins
	if current_thumbnail.is_some_and(|path| !is_generated_thumbnail(path)) {
		return SeriesCoverAction::Keep;
	}

	let current_tag = current_thumbnail.and_then(series_cover_thumbnail_tag);

	match (cover, current_tag) {
		(Some(cover), Some(current_tag)) => {
			let same_source = cover.source_tag().as_deref() == Some(current_tag);
			let is_up_to_date = match (current_thumbnail_modified, cover.modified) {
				(Some(thumbnail_mtime), Some(cover_mtime)) => {
					cover_mtime <= thumbnail_mtime
				},
				// If we can't read the cover mtime, don't thrash regenerating every scan
				(Some(_), None) => true,
				// The thumbnail file is missing
				(None, _) => false,
			};
			if same_source && is_up_to_date {
				SeriesCoverAction::Keep
			} else {
				SeriesCoverAction::Apply
			}
		},
		// There is a cover file, but the current thumbnail (if any) was not derived from it
		(Some(_), None) => SeriesCoverAction::Apply,
		// The cover file went away (or the feature was disabled)
		(None, Some(_)) => SeriesCoverAction::Revert,
		(None, None) => SeriesCoverAction::Keep,
	}
}

/// Generate and write a series thumbnail from a cover file, applying the library's thumbnail
/// configuration. Formats which can't be decoded by the image pipeline are stored as-is. The
/// caller persists the returned path and metadata.
#[tracing::instrument(skip_all, fields(path = ?cover.path))]
pub async fn generate_thumbnail_from_series_cover(
	series_id: &str,
	cover: &SeriesCoverFile,
	core_config: &StumpConfig,
	image_options: ImageProcessorOptions,
) -> Result<(Vec<u8>, PathBuf, Option<ImageMetadata>), ThumbnailGenerateError> {
	let file_stem = cover
		.thumbnail_file_stem(series_id)
		.ok_or(ThumbnailGenerateError::NothingToGenerate)?;
	let cover_data = fs::read(&cover.path).await?;

	if !cover.is_processable() {
		tracing::debug!(
			"Cover format is not supported by the image pipeline, storing as-is"
		);
		let path = write_thumbnail(
			core_config,
			&format!("{file_stem}.{}", cover.extension()),
			&cover_data,
		)
		.await?;
		return Ok((cover_data, path, None));
	}

	let format = image_options.format;
	let thumbnail = spawn_blocking(move || match format {
		SupportedImageFormat::Webp => WebpProcessor::generate(&cover_data, image_options),
		_ => GenericImageProcessor::generate(&cover_data, image_options),
	})
	.await
	.map_err(|e| ThumbnailGenerateError::Unknown(e.to_string()))??;

	let path = write_thumbnail(
		core_config,
		&format!("{file_stem}.{}", format.extension()),
		&thumbnail,
	)
	.await?;

	let metadata = match generate_image_metadata_from_bytes(thumbnail.clone()).await {
		Ok(metadata) => Some(metadata),
		Err(error) => {
			tracing::error!(?error, "Failed to generate thumbnail metadata");
			None
		},
	};

	Ok((thumbnail, path, metadata))
}

async fn write_thumbnail(
	core_config: &StumpConfig,
	file_name: &str,
	data: &[u8],
) -> Result<PathBuf, std::io::Error> {
	let thumbnails_dir = core_config.thumbnails_directory();
	fs::create_dir_all(&thumbnails_dir).await?;
	let path = thumbnails_dir.join(file_name);
	fs::write(&path, data).await?;
	Ok(path)
}

/// Remove a previously generated thumbnail file if it is no longer the one in use. Thumbnails
/// without the `.generated` marker (i.e. user uploads) are never removed.
pub(crate) async fn remove_stale_generated_thumbnail(
	previous: Option<&str>,
	current: Option<&Path>,
) {
	let Some(previous) = previous.map(Path::new) else {
		return;
	};
	if !is_generated_thumbnail(previous) || current == Some(previous) {
		return;
	}
	if let Err(error) = fs::remove_file(previous).await {
		if error.kind() != std::io::ErrorKind::NotFound {
			tracing::warn!(?error, ?previous, "Failed to remove stale thumbnail");
		}
	}
}

/// A filter which only matches series whose thumbnail is safe for the cover logic to change,
/// i.e. it is unset or generated. This guards against overwriting a thumbnail a user uploaded
/// while a scan was running.
fn generated_or_unset_thumbnail() -> Condition {
	Condition::any()
		.add(series::Column::ThumbnailPath.is_null())
		.add(series::Column::ThumbnailPath.like("%.generated.%"))
}

/// The parameters for [`sync_series_cover_thumbnail`]
pub struct SeriesCoverSyncParams<'a> {
	pub series_id: &'a str,
	/// The thumbnail path currently stored for the series
	pub current_thumbnail: Option<&'a str>,
	/// The cover file discovered in the series root. This should be `None` when the feature is
	/// disabled for the library, so that previously applied covers are reverted.
	pub cover: Option<SeriesCoverFile>,
	/// The library's thumbnail configuration, if thumbnail generation is enabled
	pub thumbnail_config: Option<&'a ImageProcessorOptions>,
	/// Whether placeholder colors should be computed when thumbnail generation is disabled
	pub process_colors_without_config: bool,
}

/// The outcome of [`sync_series_cover_thumbnail`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesCoverSyncOutcome {
	/// Nothing changed
	Unchanged,
	/// The series thumbnail was updated in place
	Updated,
	/// The series thumbnail was cleared and needs to be (re)generated by a thumbnail generation
	/// job using the library's thumbnail configuration
	NeedsGeneration,
}

/// Reconcile a series thumbnail with the cover file (if any) in the series folder. This is called
/// by the scanner for each series it walks, and is a no-op when nothing changed.
///
/// When thumbnail generation is enabled, this only invalidates the thumbnail and the caller is
/// expected to enqueue a thumbnail generation job for the series. That way, image processing
/// happens within the generation job and its existing concurrency limits, rather than inline in
/// the scan. When thumbnail generation is disabled, the cover file is copied as-is.
#[tracing::instrument(skip_all, fields(series_id = params.series_id))]
pub async fn sync_series_cover_thumbnail<C>(
	conn: &C,
	core_config: &StumpConfig,
	params: SeriesCoverSyncParams<'_>,
) -> Result<SeriesCoverSyncOutcome, ThumbnailGenerateError>
where
	C: ConnectionTrait,
{
	let SeriesCoverSyncParams {
		series_id,
		current_thumbnail,
		cover,
		thumbnail_config,
		process_colors_without_config,
	} = params;

	// Note: The mtime only matters when comparing a cover file against a thumbnail derived from
	// a cover file, so we avoid the extra stat for every other series in the library
	let current_thumbnail_modified = match current_thumbnail {
		Some(path)
			if cover.is_some()
				&& series_cover_thumbnail_tag(Path::new(path)).is_some() =>
		{
			fs::metadata(path).await.and_then(|m| m.modified()).ok()
		},
		_ => None,
	};

	let action = resolve_series_cover_action(
		current_thumbnail.map(Path::new),
		current_thumbnail_modified,
		cover.as_ref(),
	);
	tracing::trace!(?action, ?cover, "Resolved series cover action");

	match (action, cover) {
		(SeriesCoverAction::Keep, _) => Ok(SeriesCoverSyncOutcome::Unchanged),
		(SeriesCoverAction::Apply, Some(cover)) if thumbnail_config.is_none() => {
			let file_stem = cover
				.thumbnail_file_stem(series_id)
				.ok_or(ThumbnailGenerateError::NothingToGenerate)?;
			let dest_path = core_config
				.thumbnails_directory()
				.join(format!("{file_stem}.{}", cover.extension()));
			fs::create_dir_all(core_config.thumbnails_directory()).await?;
			fs::copy(&cover.path, &dest_path).await?;

			let metadata = if process_colors_without_config && cover.is_processable() {
				match generate_image_metadata(&dest_path).await {
					Ok(metadata) => Some(metadata),
					Err(error) => {
						tracing::warn!(?error, "Failed to generate cover metadata");
						None
					},
				}
			} else {
				None
			};

			set_series_thumbnail(
				conn,
				series_id,
				Some(dest_path.to_string_lossy().to_string()),
				metadata,
			)
			.await?;
			remove_stale_generated_thumbnail(current_thumbnail, Some(&dest_path)).await;

			Ok(SeriesCoverSyncOutcome::Updated)
		},
		(SeriesCoverAction::Apply | SeriesCoverAction::Revert, _) => {
			set_series_thumbnail(conn, series_id, None, None).await?;
			remove_stale_generated_thumbnail(current_thumbnail, None).await;

			if thumbnail_config.is_some() {
				Ok(SeriesCoverSyncOutcome::NeedsGeneration)
			} else {
				// Without thumbnail generation, the series thumbnail endpoint will fall back to
				// the first book on its own
				Ok(SeriesCoverSyncOutcome::Updated)
			}
		},
	}
}

/// Set (or clear) the thumbnail of a series, bumping `updated_at` so that the versioned
/// thumbnail URL changes. This will not touch a manually uploaded thumbnail.
async fn set_series_thumbnail<C>(
	conn: &C,
	series_id: &str,
	thumbnail_path: Option<String>,
	thumbnail_meta: Option<ImageMetadata>,
) -> Result<(), DbErr>
where
	C: ConnectionTrait,
{
	series::Entity::update_many()
		.filter(series::Column::Id.eq(series_id))
		.filter(generated_or_unset_thumbnail())
		.col_expr(series::Column::ThumbnailPath, Expr::value(thumbnail_path))
		.col_expr(series::Column::ThumbnailMeta, Expr::value(thumbnail_meta))
		.col_expr(
			series::Column::UpdatedAt,
			Expr::value(Some(DateTimeWithTimeZone::from(Utc::now()))),
		)
		.exec(conn)
		.await?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::*;

	fn cover(name: &str, modified: Option<SystemTime>) -> SeriesCoverFile {
		SeriesCoverFile {
			path: PathBuf::from("/library/Series").join(name),
			modified,
		}
	}

	fn pick(names: &[&str]) -> Option<String> {
		let mut picker = SeriesCoverPicker::default();
		for name in names {
			picker.offer(&PathBuf::from("/library/Series").join(name));
		}
		picker
			.finish()
			.and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
	}

	#[test]
	fn test_series_cover_priority_matches_names_and_extensions() {
		for stem in SERIES_COVER_FILE_STEMS {
			for ext in SERIES_COVER_FILE_EXTENSIONS {
				assert!(
					series_cover_priority(&format!("{stem}.{ext}")).is_some(),
					"{stem}.{ext} should be a cover candidate"
				);
			}
		}
	}

	#[test]
	fn test_series_cover_priority_is_case_insensitive() {
		assert!(series_cover_priority("Cover.WEBP").is_some());
		assert!(series_cover_priority("FOLDER.Jpg").is_some());
		assert!(series_cover_priority("poster.JPEG").is_some());
		assert_eq!(
			series_cover_priority("COVER.PNG"),
			series_cover_priority("cover.png")
		);
	}

	#[test]
	fn test_series_cover_priority_rejects_non_candidates() {
		for name in [
			"cover",
			"cover.",
			"cover.cbz",
			"cover.txt",
			"cover.jpg.bak",
			"covers.jpg",
			"my cover.jpg",
			"cover-art.png",
			".cover.jpg",
			"desc.json",
			"Chapter 0001.cbz",
			"thumbnail.jpg",
		] {
			assert_eq!(series_cover_priority(name), None, "{name} should not match");
		}
	}

	#[test]
	fn test_picker_respects_stem_priority() {
		assert_eq!(
			pick(&["poster.jpg", "folder.jpg", "cover.jpg"]).as_deref(),
			Some("cover.jpg")
		);
		assert_eq!(
			pick(&["poster.webp", "folder.png"]).as_deref(),
			Some("folder.png")
		);
		assert_eq!(pick(&["poster.gif"]).as_deref(), Some("poster.gif"));
	}

	#[test]
	fn test_picker_stem_priority_beats_extension_priority() {
		// webp is the preferred extension, but cover beats folder regardless
		assert_eq!(
			pick(&["folder.webp", "cover.gif"]).as_deref(),
			Some("cover.gif")
		);
	}

	#[test]
	fn test_picker_breaks_ties_by_extension_order() {
		assert_eq!(
			pick(&["cover.png", "cover.jpg", "cover.webp"]).as_deref(),
			Some("cover.webp")
		);
		assert_eq!(
			pick(&["cover.png", "cover.jpeg"]).as_deref(),
			Some("cover.jpeg")
		);
	}

	#[test]
	fn test_picker_is_deterministic_for_case_variants() {
		let a = pick(&["cover.jpg", "Cover.jpg"]);
		let b = pick(&["Cover.jpg", "cover.jpg"]);
		assert_eq!(a, b);
	}

	#[test]
	fn test_picker_ignores_non_candidates() {
		assert_eq!(pick(&["desc.json", "Chapter 0001.cbz"]), None);
		assert_eq!(pick(&[]), None);
	}

	#[test]
	fn test_source_tag_is_normalized() {
		assert_eq!(
			cover("Cover.WEBP", None).source_tag().as_deref(),
			Some("cover-webp")
		);
		assert_eq!(
			cover("folder.jpg", None).source_tag().as_deref(),
			Some("folder-jpg")
		);
		assert_eq!(cover("desc.json", None).source_tag(), None);
	}

	#[test]
	fn test_thumbnail_file_stem_round_trips_tag() {
		let cover = cover("Cover.jpg", None);
		let stem = cover.thumbnail_file_stem("abc123").unwrap();
		assert_eq!(stem, "abc123.seriescover-cover-jpg.generated");

		let path = PathBuf::from("/thumbnails").join(format!("{stem}.webp"));
		assert!(is_generated_thumbnail(&path));
		assert_eq!(series_cover_thumbnail_tag(&path), Some("cover-jpg"));
	}

	#[test]
	fn test_series_cover_thumbnail_tag_ignores_other_thumbnails() {
		// first-book generated thumbnail
		assert_eq!(
			series_cover_thumbnail_tag(Path::new("/thumbnails/abc123.generated.webp")),
			None
		);
		// manually uploaded/selected thumbnail
		assert_eq!(
			series_cover_thumbnail_tag(Path::new("/thumbnails/abc123.webp")),
			None
		);
		// looks like a cover tag, but no generated marker means it's a manual upload
		assert_eq!(
			series_cover_thumbnail_tag(Path::new(
				"/thumbnails/abc123.seriescover-cover-jpg.webp"
			)),
			None
		);
	}

	#[test]
	fn test_is_processable() {
		assert!(cover("cover.jpg", None).is_processable());
		assert!(cover("cover.webp", None).is_processable());
		assert!(cover("cover.gif", None).is_processable());
		assert!(!cover("cover.avif", None).is_processable());
		assert!(!cover("Cover.AVIF", None).is_processable());
	}

	#[tokio::test]
	async fn test_find_series_cover_file_only_checks_root() {
		let dir = tempfile::tempdir().unwrap();
		let nested = dir.path().join("1");
		std::fs::create_dir_all(&nested).unwrap();
		std::fs::write(nested.join("cover.jpg"), b"nested").unwrap();
		std::fs::write(dir.path().join("desc.json"), b"{}").unwrap();

		assert_eq!(find_series_cover_file(dir.path()).await, None);

		std::fs::write(dir.path().join("Poster.PNG"), b"root").unwrap();
		std::fs::write(dir.path().join("folder.jpg"), b"root").unwrap();

		let found = find_series_cover_file(dir.path()).await.unwrap();
		assert_eq!(found.path, dir.path().join("folder.jpg"));
		assert!(found.modified.is_some());
	}

	#[tokio::test]
	async fn test_find_series_cover_file_ignores_directories_named_like_covers() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::create_dir_all(dir.path().join("cover.jpg")).unwrap();
		std::fs::write(dir.path().join("poster.webp"), b"root").unwrap();

		let found = find_series_cover_file(dir.path()).await.unwrap();
		assert_eq!(found.path, dir.path().join("poster.webp"));
	}

	#[tokio::test]
	async fn test_find_series_cover_file_handles_non_directories() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("oneshot.cbz");
		std::fs::write(&file, b"not a dir").unwrap();

		assert_eq!(find_series_cover_file(&file).await, None);
		assert_eq!(
			find_series_cover_file(&dir.path().join("missing")).await,
			None
		);
	}

	const T0: SystemTime = SystemTime::UNIX_EPOCH;

	fn at(secs: u64) -> Option<SystemTime> {
		Some(T0 + Duration::from_secs(secs))
	}

	#[test]
	fn test_precedence_manual_thumbnail_wins_over_cover() {
		let manual = Path::new("/thumbnails/abc123.webp");
		for current_mtime in [at(10), None] {
			assert_eq!(
				resolve_series_cover_action(
					Some(manual),
					current_mtime,
					Some(&cover("cover.jpg", at(100)))
				),
				SeriesCoverAction::Keep
			);
		}
		// and is never reverted, even if no cover exists
		assert_eq!(
			resolve_series_cover_action(Some(manual), at(10), None),
			SeriesCoverAction::Keep
		);
	}

	#[test]
	fn test_precedence_cover_wins_over_first_book() {
		let first_book = Path::new("/thumbnails/abc123.generated.webp");
		assert_eq!(
			resolve_series_cover_action(
				Some(first_book),
				at(100),
				// Note: the cover being older than the thumbnail does not matter here
				Some(&cover("cover.jpg", at(10)))
			),
			SeriesCoverAction::Apply
		);
	}

	#[test]
	fn test_cover_applied_when_no_thumbnail_exists() {
		assert_eq!(
			resolve_series_cover_action(None, None, Some(&cover("cover.jpg", at(10)))),
			SeriesCoverAction::Apply
		);
	}

	#[test]
	fn test_first_book_used_when_no_cover_exists() {
		let first_book = Path::new("/thumbnails/abc123.generated.webp");
		assert_eq!(
			resolve_series_cover_action(Some(first_book), at(10), None),
			SeriesCoverAction::Keep
		);
		assert_eq!(
			resolve_series_cover_action(None, None, None),
			SeriesCoverAction::Keep
		);
	}

	#[test]
	fn test_cover_thumbnail_kept_when_unchanged() {
		let current =
			Path::new("/thumbnails/abc123.seriescover-cover-jpg.generated.webp");
		assert_eq!(
			resolve_series_cover_action(
				Some(current),
				at(100),
				Some(&cover("cover.jpg", at(10)))
			),
			SeriesCoverAction::Keep
		);
		// same mtime is considered up to date
		assert_eq!(
			resolve_series_cover_action(
				Some(current),
				at(100),
				Some(&cover("COVER.JPG", at(100)))
			),
			SeriesCoverAction::Keep
		);
	}

	#[test]
	fn test_cover_thumbnail_regenerated_when_cover_replaced() {
		let current =
			Path::new("/thumbnails/abc123.seriescover-cover-jpg.generated.webp");
		assert_eq!(
			resolve_series_cover_action(
				Some(current),
				at(100),
				Some(&cover("cover.jpg", at(200)))
			),
			SeriesCoverAction::Apply
		);
	}

	#[test]
	fn test_cover_thumbnail_regenerated_when_source_changes() {
		// e.g. cover.jpg was removed but folder.jpg (older) remains, or cover.webp was added
		let current =
			Path::new("/thumbnails/abc123.seriescover-cover-jpg.generated.webp");
		for other in ["folder.jpg", "cover.webp"] {
			assert_eq!(
				resolve_series_cover_action(
					Some(current),
					at(100),
					Some(&cover(other, at(10)))
				),
				SeriesCoverAction::Apply,
				"{other}"
			);
		}
	}

	#[test]
	fn test_cover_thumbnail_regenerated_when_file_missing() {
		let current =
			Path::new("/thumbnails/abc123.seriescover-cover-jpg.generated.webp");
		assert_eq!(
			resolve_series_cover_action(
				Some(current),
				None,
				Some(&cover("cover.jpg", at(10)))
			),
			SeriesCoverAction::Apply
		);
	}

	#[test]
	fn test_cover_thumbnail_kept_when_cover_mtime_unknown() {
		let current =
			Path::new("/thumbnails/abc123.seriescover-cover-jpg.generated.webp");
		assert_eq!(
			resolve_series_cover_action(
				Some(current),
				at(100),
				Some(&cover("cover.jpg", None))
			),
			SeriesCoverAction::Keep
		);
	}

	#[test]
	fn test_cover_thumbnail_reverted_when_cover_removed() {
		let current =
			Path::new("/thumbnails/abc123.seriescover-cover-jpg.generated.webp");
		for current_mtime in [at(100), None] {
			assert_eq!(
				resolve_series_cover_action(Some(current), current_mtime, None),
				SeriesCoverAction::Revert
			);
		}
	}
}
