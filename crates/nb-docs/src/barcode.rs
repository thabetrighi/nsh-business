//! Code 128 (set B) for what documents number: a receipt's number printed
//! as bars so the till can scan it back (reprint, return, edit).

/// Bar and space widths of the 107 symbols (values 0–106); each adds up
/// to 11 modules, the stop (106) to 13.
const PATTERNS: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

const START_B: usize = 104;
const STOP: usize = 106;

fn push(out: &mut Vec<bool>, value: usize) {
    let mut bar = true;
    for w in PATTERNS[value].bytes() {
        for _ in 0..(w - b'0') {
            out.push(bar);
        }
        bar = !bar;
    }
}

/// The modules of `text` in Code 128 B (`true` = bar), with quiet zones
/// of ten modules; `None` when a character is outside printable ASCII.
pub fn code128(text: &str) -> Option<Vec<bool>> {
    if text.is_empty() {
        return None;
    }
    let values: Vec<usize> = text
        .bytes()
        .map(|b| (32..127).contains(&b).then(|| usize::from(b - 32)))
        .collect::<Option<_>>()?;
    let mut out = vec![false; 10];
    push(&mut out, START_B);
    let mut check = START_B;
    for (i, v) in values.iter().enumerate() {
        push(&mut out, *v);
        check += (i + 1) * v;
    }
    push(&mut out, check % 103);
    push(&mut out, STOP);
    out.extend([false; 10]);
    Some(out)
}

/// Reads modules made by [`code128`] back to text (tests; checks the
/// check symbol).
pub fn decode128(modules: &[bool]) -> Option<String> {
    let start = modules.iter().position(|&m| m)?;
    let end = modules.iter().rposition(|&m| m)? + 1;
    let bits = &modules[start..end];
    // Widths of runs.
    let mut runs = Vec::new();
    let mut i = 0;
    while i < bits.len() {
        let b = bits[i];
        let mut n = 0;
        while i < bits.len() && bits[i] == b {
            n += 1;
            i += 1;
        }
        runs.push(n);
    }
    if runs.len() < 6 + 6 + 7 || (runs.len() - 7) % 6 != 0 {
        return None;
    }
    let sym = |chunk: &[usize]| -> Option<usize> {
        let s: String = chunk.iter().map(|n| char::from(b'0' + *n as u8)).collect();
        PATTERNS.iter().position(|p| *p == s)
    };
    let symbols: Vec<usize> = runs[..runs.len() - 7]
        .chunks(6)
        .map(sym)
        .collect::<Option<_>>()?;
    if sym(&runs[runs.len() - 7..])? != STOP || symbols.first() != Some(&START_B) {
        return None;
    }
    let (data, check) = symbols[1..].split_at(symbols.len() - 2);
    let sum = START_B
        + data
            .iter()
            .enumerate()
            .map(|(i, v)| (i + 1) * v)
            .sum::<usize>();
    if sum % 103 != *check.first()? {
        return None;
    }
    Some(data.iter().map(|v| char::from((v + 32) as u8)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_symbol_is_eleven_modules_and_unique() {
        for (i, p) in PATTERNS.iter().enumerate() {
            let sum: u32 = p.bytes().map(|b| u32::from(b - b'0')).sum();
            assert_eq!(sum, if i == STOP { 13 } else { 11 }, "symbol {i}");
            assert_eq!(PATTERNS.iter().filter(|q| *q == p).count(), 1, "symbol {i}");
            // Code 128's own check: the bars of a symbol add up to an even width.
            let bars: u32 = p.bytes().step_by(2).map(|b| u32::from(b - b'0')).sum();
            assert_eq!(bars % 2, 0, "symbol {i}");
        }
    }

    #[test]
    fn receipt_numbers_round_trip() {
        for text in ["FV-01-26-A42", "261108010166", "a b~"] {
            let m = code128(text).unwrap_or_default();
            assert_eq!(decode128(&m).as_deref(), Some(text));
        }
        assert!(code128("é").is_none());
        assert!(code128("").is_none());
    }
}
