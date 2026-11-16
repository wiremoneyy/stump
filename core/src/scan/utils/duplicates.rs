//! Detection of series folders which collapse to the same name once names are cleaned,
//! e.g. `my-comic-name` and `My Comic Name` sitting next to each other on disk

use std::{
	cmp::Reverse,
	collections::HashMap,
	path::{Path, PathBuf},
};

use globset::GlobSet;
use walkdir::WalkDir;

use crate::fs_utils::{clean_name, PathUtils};

/// A series folder on disk which is being considered for duplicate detection
#[derive(Debug, Clone)]
pub(crate) struct SeriesCandidate {
	pub path: PathBuf,
	/// Whether the series already exists in the database
	pub exists: bool,
	/// The number of books found on disk for the series
	pub book_count: usize,
}

/// A series folder which lost out to another folder with the same cleaned name
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DuplicateSeries {
	/// The path of the series which should be dropped
	pub path: PathBuf,
	pub book_count: usize,
	/// The path of the series which was kept instead
	pub kept_path: PathBuf,
	pub kept_book_count: usize,
}

/// Counts the books on disk for the series at `path`. If `deep` is true, books nested
/// in subdirectories are counted as well (i.e. for collection-based libraries)
pub(crate) fn count_books_on_disk(
	path: &Path,
	ignore_rules: &GlobSet,
	deep: bool,
) -> usize {
	let mut walker = WalkDir::new(path).min_depth(1);
	if !deep {
		walker = walker.max_depth(1);
	}
	walker
		.into_iter()
		.filter_map(Result::ok)
		.filter(|entry| entry.file_type().is_file())
		.filter(|entry| {
			let entry_path = entry.path();
			!entry_path.is_default_ignored() && !ignore_rules.is_match(entry_path)
		})
		.count()
}

fn folder_name(path: &Path) -> Option<String> {
	path.file_name().map(|n| n.to_string_lossy().to_string())
}

/// Finds the series which are duplicates of one another once their names are cleaned,
/// returning the ones which should be dropped. Two series are duplicates when they sit
/// in the same parent folder and their cleaned names match, ignoring case.
///
/// The series with the most books is kept. Ties are broken by preferring a series which
/// already exists in the database, then one whose folder name is already clean.
pub(crate) fn find_duplicate_series(
	candidates: Vec<SeriesCandidate>,
) -> Vec<DuplicateSeries> {
	let mut groups: HashMap<(Option<PathBuf>, String), Vec<SeriesCandidate>> =
		HashMap::new();

	for candidate in candidates {
		let Some(name) = folder_name(&candidate.path) else {
			continue;
		};
		let parent = candidate.path.parent().map(Path::to_path_buf);
		groups
			.entry((parent, clean_name(&name).to_lowercase()))
			.or_default()
			.push(candidate);
	}

	let mut duplicates = vec![];

	for mut group in groups.into_values().filter(|group| group.len() > 1) {
		group.sort_by_key(|candidate| {
			let is_already_clean =
				folder_name(&candidate.path).is_some_and(|n| clean_name(&n) == n);
			(
				Reverse(candidate.book_count),
				!candidate.exists,
				!is_already_clean,
				candidate.path.clone(),
			)
		});

		let mut group = group.into_iter();
		let Some(kept) = group.next() else {
			continue;
		};

		duplicates.extend(group.map(|dropped| DuplicateSeries {
			path: dropped.path,
			book_count: dropped.book_count,
			kept_path: kept.path.clone(),
			kept_book_count: kept.book_count,
		}));
	}

	duplicates.sort_by(|a, b| a.path.cmp(&b.path));
	duplicates
}

#[cfg(test)]
mod tests {
	use super::*;

	fn candidate(path: &str, exists: bool, book_count: usize) -> SeriesCandidate {
		SeriesCandidate {
			path: PathBuf::from(path),
			exists,
			book_count,
		}
	}

	#[test]
	fn test_keeps_series_with_most_books() {
		let duplicates = find_duplicate_series(vec![
			candidate("/lib/my-comic-name", false, 3),
			candidate("/lib/My Comic Name", false, 10),
			candidate("/lib/Something Else", false, 1),
		]);

		assert_eq!(
			duplicates,
			vec![DuplicateSeries {
				path: PathBuf::from("/lib/my-comic-name"),
				book_count: 3,
				kept_path: PathBuf::from("/lib/My Comic Name"),
				kept_book_count: 10,
			}]
		);
	}

	#[test]
	fn test_drops_existing_series_with_fewer_books() {
		let duplicates = find_duplicate_series(vec![
			candidate("/lib/My Comic Name", true, 2),
			candidate("/lib/my_comic_name", false, 5),
		]);

		assert_eq!(duplicates.len(), 1);
		assert_eq!(duplicates[0].path, PathBuf::from("/lib/My Comic Name"));
	}

	#[test]
	fn test_tie_prefers_existing_then_clean_name() {
		let duplicates = find_duplicate_series(vec![
			candidate("/lib/My Comic Name", false, 4),
			candidate("/lib/my-comic-name", true, 4),
		]);
		assert_eq!(duplicates[0].path, PathBuf::from("/lib/My Comic Name"));

		let duplicates = find_duplicate_series(vec![
			candidate("/lib/my-comic-name", false, 4),
			candidate("/lib/my comic name", false, 4),
		]);
		assert_eq!(duplicates[0].path, PathBuf::from("/lib/my-comic-name"));
	}

	#[test]
	fn test_all_but_one_dropped() {
		let duplicates = find_duplicate_series(vec![
			candidate("/lib/a-b", false, 1),
			candidate("/lib/a_b", false, 7),
			candidate("/lib/A B", false, 2),
		]);

		assert_eq!(duplicates.len(), 2);
		assert!(duplicates
			.iter()
			.all(|d| d.kept_path == PathBuf::from("/lib/a_b")));
	}

	#[test]
	fn test_same_name_in_different_parents_is_not_a_duplicate() {
		let duplicates = find_duplicate_series(vec![
			candidate("/lib/Series A/Extras", false, 1),
			candidate("/lib/Series B/Extras", false, 2),
		]);

		assert!(duplicates.is_empty());
	}
}
