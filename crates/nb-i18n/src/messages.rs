//! Message catalogs compiled into the binary.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

use crate::Lang;

const AR: &str = include_str!("../locales/ar.ftl");
const FR: &str = include_str!("../locales/fr.ftl");
const EN: &str = include_str!("../locales/en.ftl");

fn parse(src: &str) -> HashMap<&str, &str> {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect()
}

static CATALOGS: LazyLock<[HashMap<&'static str, &'static str>; 3]> =
    LazyLock::new(|| [parse(AR), parse(FR), parse(EN)]);

fn catalog(lang: Lang) -> &'static HashMap<&'static str, &'static str> {
    let i = match lang {
        Lang::Ar => 0,
        Lang::Fr => 1,
        Lang::En => 2,
    };
    &CATALOGS[i]
}

/// The message `key` in `lang`, falling back to English, then to the key.
pub fn tr(lang: Lang, key: &str) -> Cow<'static, str> {
    catalog(lang)
        .get(key)
        .or_else(|| catalog(Lang::En).get(key))
        .map(|s| Cow::Borrowed(*s))
        .unwrap_or_else(|| Cow::Owned(key.to_owned()))
}

/// The message `key` with `{name}` placeholders replaced.
pub fn tr_args(lang: Lang, key: &str, args: &[(&str, &str)]) -> String {
    let mut s = tr(lang, key).into_owned();
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

/// Every key defined in any language.
pub fn all_keys() -> BTreeSet<&'static str> {
    Lang::ALL
        .iter()
        .flat_map(|&l| catalog(l).keys().copied())
        .collect()
}

/// Keys missing from `lang` (should be empty for every language).
pub fn missing_keys(lang: Lang) -> Vec<&'static str> {
    all_keys()
        .into_iter()
        .filter(|k| !catalog(lang).contains_key(k))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_every_key() {
        for lang in Lang::ALL {
            assert!(
                missing_keys(lang).is_empty(),
                "{:?} is missing {:?}",
                lang,
                missing_keys(lang)
            );
        }
    }

    /// Placeholders are `{name}`: Fluent's `{ $name }` would show as such.
    #[test]
    fn placeholders_are_plain_names() {
        for lang in Lang::ALL {
            for key in all_keys() {
                let t = tr(lang, key);
                for p in t.split('{').skip(1).filter_map(|p| p.split_once('}')) {
                    assert!(
                        // `{shop.name}`: a template variable shown as an example.
                        p.0.chars()
                            .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.'),
                        "{key} in {lang:?}: {{{}}}",
                        p.0
                    );
                }
            }
        }
    }

    #[test]
    fn placeholders_match_across_languages() {
        let placeholders = |s: &str| -> BTreeSet<String> {
            s.split('{')
                .skip(1)
                .filter_map(|p| p.split_once('}'))
                .map(|(n, _)| n.to_owned())
                .collect()
        };
        for key in all_keys() {
            let en = placeholders(&tr(Lang::En, key));
            for lang in [Lang::Ar, Lang::Fr] {
                assert_eq!(
                    placeholders(&tr(lang, key)),
                    en,
                    "placeholders differ for {key} in {lang:?}"
                );
            }
        }
    }

    #[test]
    fn falls_back() {
        assert_eq!(tr(Lang::Ar, "no.such.key"), "no.such.key");
        assert_eq!(
            tr_args(Lang::Fr, "dashboard.greeting", &[("name", "Ahmed")]),
            "Bonjour, Ahmed"
        );
    }
}
