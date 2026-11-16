use models::entity::library_config;
use sea_orm::ActiveValue::Set;

use super::common::TestLibrary;

const SLUG: &str = "100-million-years-later";
const CLEAN: &str = "100 Million Years Later";

async fn clean_names_library() -> TestLibrary {
	let library = TestLibrary::new().await;
	library
		.update_config(library_config::ActiveModel {
			clean_names: Set(true),
			..Default::default()
		})
		.await;
	library
}

#[tokio::test]
async fn names_are_cleaned_when_enabled() {
	let library = clean_names_library().await;
	library.add_epub(SLUG, "chapter-0001");
	library.add_epub(SLUG, "A Cadet Becomes a Prophet_!");

	library.scan().await;

	let snapshot = library.snapshot().await;
	let series = snapshot.series_at(&library.path_str(SLUG)).unwrap();
	assert_eq!(series.name, "100 million years later");
	let mut names: Vec<_> = snapshot.media.iter().map(|m| m.name.as_str()).collect();
	names.sort();
	assert_eq!(names, vec!["A Cadet Becomes a Prophet!", "chapter 0001"]);
}

#[tokio::test]
async fn names_are_untouched_when_disabled() {
	let library = TestLibrary::new().await;
	library.add_epub(SLUG, "chapter-0001");
	library.add_epub(CLEAN, "Chapter 0001");

	library.scan().await;

	let snapshot = library.snapshot().await;
	assert_eq!(snapshot.series.len(), 2, "{snapshot:#?}");
	let series = snapshot.series_at(&library.path_str(SLUG)).unwrap();
	assert_eq!(series.name, SLUG);
}

#[tokio::test]
async fn duplicate_with_fewest_books_is_never_created() {
	let library = clean_names_library().await;
	library.add_epub(SLUG, "Chapter 0001");
	library.add_epub(CLEAN, "Chapter 0001");
	library.add_epub(CLEAN, "Chapter 0002");

	library.scan().await;

	let snapshot = library.snapshot().await;
	assert_eq!(snapshot.series.len(), 1, "{snapshot:#?}");
	assert!(snapshot.series_at(&library.path_str(CLEAN)).is_some());
	snapshot.assert_unique_media(2);
	assert!(
		library.path(SLUG).exists(),
		"files on disk must be untouched"
	);

	library.assert_rescan_is_noop().await;
}

#[tokio::test]
async fn existing_series_is_removed_when_a_bigger_duplicate_appears() {
	let library = clean_names_library().await;
	library.add_epub(SLUG, "Chapter 0001");
	library.scan().await;
	assert!(library
		.snapshot()
		.await
		.series_at(&library.path_str(SLUG))
		.is_some());

	library.add_epub(CLEAN, "Chapter 0001");
	library.add_epub(CLEAN, "Chapter 0002");
	library.scan().await;

	let snapshot = library.snapshot().await;
	assert_eq!(snapshot.series.len(), 1, "{snapshot:#?}");
	let kept = snapshot.series_at(&library.path_str(CLEAN)).unwrap();
	assert_eq!(snapshot.books_in(&kept.id), 2);
	snapshot.assert_unique_media(2);
	assert!(
		library.path(SLUG).exists(),
		"files on disk must be untouched"
	);

	library.assert_rescan_is_noop().await;
}

#[tokio::test]
async fn dropped_duplicate_takes_over_once_it_has_more_books() {
	let library = clean_names_library().await;
	library.add_epub(SLUG, "Chapter 0001");
	library.add_epub(CLEAN, "Chapter 0001");
	library.add_epub(CLEAN, "Chapter 0002");
	library.scan().await;

	library.add_epub(SLUG, "Chapter 0002");
	library.add_epub(SLUG, "Chapter 0003");
	library.scan().await;

	let snapshot = library.snapshot().await;
	assert_eq!(snapshot.series.len(), 1, "{snapshot:#?}");
	let kept = snapshot.series_at(&library.path_str(SLUG)).unwrap();
	assert_eq!(snapshot.books_in(&kept.id), 3);
	snapshot.assert_unique_media(3);
}
