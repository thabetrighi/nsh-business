//! Bidi-safe text: isolates runs whose direction differs from the paragraph.
//!
//! Renderers that only reorder at paragraph level (NSH's web runtime, many
//! thermal printers) break mixed text such as `حليب Soummam 1L` or
//! `الهاتف 0550 00 00 00`. Wrapping each opposite-direction run in an
//! explicit isolate (LRI…PDI or RLI…PDI) makes every renderer agree.
//!
//! A run starts at a strong or numeric character of the opposite direction
//! and extends across neutrals (spaces, `-`, `/`, `:`, `.`, `×`) until the next
//! strong character of the paragraph's direction, so `0550 00 00 00` or
//! `D1-2026-0001234` stay whole.

use unicode_bidi::{BidiClass, bidi_class};

const LRI: char = '\u{2066}';
const RLI: char = '\u{2067}';
const FSI: char = '\u{2068}';
const PDI: char = '\u{2069}';

/// Paragraph direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Strong in the paragraph's direction.
    Base,
    /// Strong opposite, or a number (numbers never take an RTL order).
    Opposite,
    Neutral,
    /// Already an isolate/embedding control: leave the text as authored.
    Control,
}

fn kind(c: char, dir: Direction) -> Kind {
    use BidiClass::*;
    match (bidi_class(c), dir) {
        (LRI | RLI | FSI | PDI | LRE | RLE | LRO | RLO | PDF, _) => Kind::Control,
        (R | AL, Direction::Rtl) | (L, Direction::Ltr) => Kind::Base,
        (L | EN | AN, Direction::Rtl) | (R | AL, Direction::Ltr) => Kind::Opposite,
        _ => Kind::Neutral,
    }
}

/// Returns `text` with every opposite-direction run wrapped in an isolate.
/// Text that already contains explicit bidi controls is returned unchanged.
pub fn isolate_runs(text: &str, dir: Direction) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.iter().any(|&c| kind(c, dir) == Kind::Control) {
        return text.to_owned();
    }
    let open = match dir {
        Direction::Rtl => LRI,
        Direction::Ltr => RLI,
    };
    let mut out = String::with_capacity(text.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        if kind(chars[i], dir) != Kind::Opposite {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // Extend the run over opposite and neutral characters until a base-direction strong one.
        let start = i;
        let mut end = i; // last opposite character index
        let mut j = i;
        while j < chars.len() {
            match kind(chars[j], dir) {
                Kind::Opposite => {
                    end = j;
                    j += 1;
                }
                Kind::Neutral => j += 1,
                _ => break,
            }
        }
        // Trailing neutrals (spaces, brackets) stay outside so the paragraph pairs and mirrors them.
        let stop = end + 1;
        out.push(open);
        out.extend(&chars[start..stop]);
        out.push(PDI);
        i = stop;
    }
    out
}

/// Wraps a whole value (number, phone, code) so it keeps its own order inside any paragraph.
pub fn isolate_value(value: &str) -> String {
    format!("{FSI}{value}{PDI}")
}

/// Wraps a left-to-right value (number, amount, SKU) so it always reads left to right.
pub fn isolate_ltr(value: &str) -> String {
    format!("{LRI}{value}{PDI}")
}

/// Removes bidi control characters (for search, export, comparisons).
pub fn strip_controls(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{2066}'..='\u{2069}' | '\u{202A}'..='\u{202E}' | '\u{200E}' | '\u{200F}'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(s: &str) -> String {
        s.replace(LRI, "<").replace(RLI, "{").replace(PDI, ">")
    }

    #[test]
    fn isolates_latin_and_numbers_in_arabic() {
        assert_eq!(
            show(&isolate_runs("حليب كامل الدسم Soummam 1L", Direction::Rtl)),
            "حليب كامل الدسم <Soummam 1L>"
        );
        assert_eq!(
            show(&isolate_runs("الهاتف 0550 00 00 00", Direction::Rtl)),
            "الهاتف <0550 00 00 00>"
        );
        assert_eq!(
            show(&isolate_runs("فاتورة رقم D1-2026-0001234", Direction::Rtl)),
            "فاتورة رقم <D1-2026-0001234>"
        );
        assert_eq!(
            show(&isolate_runs("زيت مائدة (Elio 5L)", Direction::Rtl)),
            "زيت مائدة (<Elio 5L>)"
        );
        assert_eq!(
            show(&isolate_runs("3 × كرتون", Direction::Rtl)),
            "<3> × كرتون"
        );
    }

    #[test]
    fn isolates_arabic_in_french() {
        assert_eq!(
            show(&isolate_runs("Client : أحمد بن علي", Direction::Ltr)),
            "Client : {أحمد بن علي>"
        );
    }

    #[test]
    fn leaves_pure_text_and_authored_controls_alone() {
        assert_eq!(isolate_runs("السلة", Direction::Rtl), "السلة");
        assert_eq!(isolate_runs("Panier", Direction::Ltr), "Panier");
        let authored = format!("الإجمالي {}", isolate_ltr("120.00"));
        assert_eq!(isolate_runs(&authored, Direction::Rtl), authored);
    }

    #[test]
    fn strips_controls_for_search() {
        assert_eq!(
            strip_controls(&isolate_runs("حليب Soummam", Direction::Rtl)),
            "حليب Soummam"
        );
    }
}
