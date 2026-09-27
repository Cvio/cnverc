//! Languages and their varieties (dialects), by BCP 47 tag.
//!
//! A tag is a language, optionally with a region: `es` is Spanish with no
//! particular dialect, `es-MX` is Mexican Spanish. Everywhere Volis stores a
//! language it may store a variety instead, and old settings with plain codes
//! keep working.
//!
//! Adding a dialect means adding one row to [`TABLE`], not code. A tag that
//! isn't in the table is an error shown to the user, not a silent fallback: a
//! translation prompt that says "into xx-YY" works worse than one that names
//! the dialect.

/// One language or variety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variety {
    /// The BCP 47 tag, e.g. `es-MX`.
    pub tag: &'static str,
    /// Shown in the window, e.g. "Spanish (Mexico)".
    pub display: &'static str,
    /// Written into the translation prompt, e.g. "Mexican Spanish".
    pub prompt: &'static str,
}

/// Every language and variety Volis knows. Languages first, each followed by
/// its varieties.
pub const TABLE: &[Variety] = &[
    Variety {
        tag: "en",
        display: "English",
        prompt: "English",
    },
    Variety {
        tag: "en-US",
        display: "English (US)",
        prompt: "American English",
    },
    Variety {
        tag: "es",
        display: "Spanish",
        prompt: "Spanish",
    },
    Variety {
        tag: "es-MX",
        display: "Spanish (Mexico)",
        prompt: "Mexican Spanish",
    },
    Variety {
        tag: "es-ES",
        display: "Spanish (Spain)",
        prompt: "Peninsular Spanish",
    },
    Variety {
        tag: "ar",
        display: "Arabic",
        prompt: "Arabic",
    },
    Variety {
        tag: "ar-IQ",
        display: "Arabic (Iraq)",
        prompt: "Iraqi Arabic",
    },
    Variety {
        tag: "ar-JO",
        display: "Arabic (Jordan)",
        prompt: "Jordanian Arabic",
    },
    // Languages the translation prompt already knew by name, kept so they
    // don't stop working. They have no varieties yet.
    Variety {
        tag: "de",
        display: "German",
        prompt: "German",
    },
    Variety {
        tag: "fr",
        display: "French",
        prompt: "French",
    },
    Variety {
        tag: "it",
        display: "Italian",
        prompt: "Italian",
    },
    Variety {
        tag: "pt",
        display: "Portuguese",
        prompt: "Portuguese",
    },
    Variety {
        tag: "ru",
        display: "Russian",
        prompt: "Russian",
    },
];

/// The table's entry for `tag`, matched without regard to case (`es-mx` is
/// `es-MX`).
pub fn lookup(tag: &str) -> Option<&'static Variety> {
    let tag = tag.trim();
    TABLE.iter().find(|v| v.tag.eq_ignore_ascii_case(tag))
}

/// The same, as an error a person can act on.
pub fn require(tag: &str) -> Result<&'static Variety, String> {
    lookup(tag).ok_or_else(|| {
        format!(
            "\"{}\" is not a language or variety Volis knows. Known: {}. To add one, add a row \
             to src/varieties.rs.",
            tag.trim(),
            TABLE.iter().map(|v| v.tag).collect::<Vec<_>>().join(", ")
        )
    })
}

/// The language part of a tag: `es` for both `es` and `es-MX`. This is what a
/// recognizer is told, and what `engine.toml`'s `languages` lists.
pub fn language_of(tag: &str) -> &str {
    let tag = tag.trim();
    tag.split('-').next().unwrap_or(tag)
}

/// Whether a tag names a region as well as a language.
pub fn has_variety(tag: &str) -> bool {
    tag.trim().contains('-')
}

/// The varieties the table lists for a language, not counting the plain
/// language itself: for `es`, just `es-MX`.
pub fn varieties_of(language: &str) -> Vec<&'static Variety> {
    TABLE
        .iter()
        .filter(|v| has_variety(v.tag) && language_of(v.tag).eq_ignore_ascii_case(language))
        .collect()
}

/// How to show a tag in the window. An unknown tag is shown as itself, marked
/// so, rather than hidden.
pub fn display_name(tag: &str) -> String {
    match lookup(tag) {
        Some(v) => v.display.to_string(),
        None => format!("{} (unknown)", tag.trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_well_formed() {
        for v in TABLE {
            // Each variety's language is itself a row, so a dialect is never
            // offered for a language the table doesn't know.
            assert!(
                lookup(language_of(v.tag)).is_some(),
                "{} has no language row",
                v.tag
            );
            assert!(!v.display.is_empty() && !v.prompt.is_empty(), "{}", v.tag);
            let duplicates = TABLE
                .iter()
                .filter(|w| w.tag.eq_ignore_ascii_case(v.tag))
                .count();
            assert_eq!(duplicates, 1, "{} is listed twice", v.tag);
        }
    }

    #[test]
    fn tags_split_into_language_and_variety() {
        assert_eq!(language_of("es-MX"), "es");
        assert_eq!(language_of("es"), "es");
        assert!(has_variety("ar-IQ"));
        assert!(!has_variety("ar"));
    }

    #[test]
    fn lookups_give_the_display_and_prompt_names() {
        let v = require("es-MX").expect("known");
        assert_eq!(
            (v.display, v.prompt),
            ("Spanish (Mexico)", "Mexican Spanish")
        );
        assert_eq!(
            require("ar-iq").expect("case-insensitive").prompt,
            "Iraqi Arabic"
        );
        assert_eq!(require("ar").expect("Arabic is known now").prompt, "Arabic");
    }

    #[test]
    fn an_unknown_tag_is_an_error_not_a_fallback() {
        let why = require("xx-YY").expect_err("unknown");
        assert!(
            why.contains("xx-YY") && why.contains("varieties.rs"),
            "{why}"
        );
        assert_eq!(display_name("xx-YY"), "xx-YY (unknown)");
    }

    #[test]
    fn a_language_lists_only_its_own_varieties() {
        let arabic: Vec<_> = varieties_of("ar").iter().map(|v| v.tag).collect();
        assert_eq!(arabic, vec!["ar-IQ", "ar-JO"]);
        assert!(varieties_of("de").is_empty());
    }
}
