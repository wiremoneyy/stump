/// Punctuation which should hug the word before it, e.g. the `!` in `Prophet_!`
const TRAILING_PUNCTUATION: [char; 8] = ['!', '?', ',', '.', ';', ':', ')', ']'];

#[derive(Clone, Copy, PartialEq)]
enum Gap {
	None,
	/// Whitespace which was present in the original name
	Whitespace,
	/// A separator character (`_` or `-`) which was removed
	Separator,
}

/// Cleans up a name derived from a file or folder on disk by removing the `_` and `-`
/// characters which are commonly used in place of spaces, e.g.:
///
/// - `100-million-years-later-the-ultimate-weapons-return` -> `100 million years later the ultimate weapons return`
/// - `A Cadet Becomes a Prophet_!` -> `A Cadet Becomes a Prophet!`
///
/// Underscores are always removed. Hyphens are only removed when they are evidently
/// standing in for spaces, i.e. when the name has no whitespace or underscores at all,
/// so that names like `Spider-Man 001` or `Batman - Year One` are left intact.
///
/// If cleaning would result in an empty name, the original name is returned.
pub fn clean_name(name: &str) -> String {
	let hyphens_are_separators =
		!name.contains('_') && !name.chars().any(char::is_whitespace);

	let mut cleaned = String::with_capacity(name.len());
	let mut gap = Gap::None;

	for c in name.chars() {
		if c.is_whitespace() {
			gap = Gap::Whitespace;
		} else if c == '_' || (c == '-' && hyphens_are_separators) {
			if gap == Gap::None {
				gap = Gap::Separator;
			}
		} else {
			let needs_space = match gap {
				Gap::None => false,
				Gap::Whitespace => true,
				Gap::Separator => !TRAILING_PUNCTUATION.contains(&c),
			};
			if needs_space && !cleaned.is_empty() {
				cleaned.push(' ');
			}
			cleaned.push(c);
			gap = Gap::None;
		}
	}

	if cleaned.is_empty() {
		name.to_string()
	} else {
		cleaned
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_clean_name_hyphenated_slug() {
		assert_eq!(
			clean_name("100-million-years-later-the-ultimate-weapons-return"),
			"100 million years later the ultimate weapons return"
		);
	}

	#[test]
	fn test_clean_name_stray_underscore() {
		assert_eq!(
			clean_name("A Cadet Becomes a Prophet_!"),
			"A Cadet Becomes a Prophet!"
		);
	}

	#[test]
	fn test_clean_name_underscores_as_spaces() {
		assert_eq!(clean_name("My_Comic_Vol_01"), "My Comic Vol 01");
		assert_eq!(clean_name("My_Comic_(2020)"), "My Comic (2020)");
		assert_eq!(clean_name("__My__Comic__"), "My Comic");
	}

	#[test]
	fn test_clean_name_keeps_meaningful_hyphens() {
		assert_eq!(clean_name("Spider-Man 001"), "Spider-Man 001");
		assert_eq!(clean_name("Batman - Year One"), "Batman - Year One");
		assert_eq!(clean_name("Spider-Man_001"), "Spider-Man 001");
	}

	#[test]
	fn test_clean_name_untouched() {
		assert_eq!(clean_name("Already Clean"), "Already Clean");
		assert_eq!(clean_name(""), "");
	}

	#[test]
	fn test_clean_name_never_empty() {
		assert_eq!(clean_name("___"), "___");
		assert_eq!(clean_name("-"), "-");
	}
}
