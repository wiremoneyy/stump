mod generate;
mod generation_job;
mod placeholder;
mod placeholder_job;
mod series_cover;
mod utils;

pub use generate::{
	bump_media_thumbnail_fallbacks, bump_series_thumbnail_fallbacks,
	generate_thumbnail_from_book, GenerateThumbnailOptions, ThumbnailGenerateError,
};
pub use generation_job::{
	ThumbnailGenerationJob, ThumbnailGenerationJobParams, ThumbnailGenerationJobScope,
	ThumbnailGenerationOutput,
};
pub use placeholder::*;
pub use placeholder_job::{
	PlaceholderGenerationJob, PlaceholderGenerationJobConfig,
	PlaceholderGenerationJobScope, PlaceholderGenerationOutput,
};
pub use series_cover::{
	find_series_cover_file, generate_thumbnail_from_series_cover,
	resolve_series_cover_action, series_cover_priority, series_cover_thumbnail_tag,
	sync_series_cover_thumbnail, SeriesCoverAction, SeriesCoverFile, SeriesCoverPicker,
	SeriesCoverSyncOutcome, SeriesCoverSyncParams, SERIES_COVER_FILE_EXTENSIONS,
	SERIES_COVER_FILE_STEMS,
};
pub use utils::*;
