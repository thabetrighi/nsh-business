//! What forms share: reading amounts typed by people, and the message of a
//! rule code coming back from the use cases.

use nb_core::{CoreError, Currency, Money};
use nb_i18n::tr;

use crate::state::AppState;

/// Parses a price typed by a person: `1250`, `1 250,50`, `1250.5`.
pub fn parse_price(input: &str, currency: Currency) -> Result<Money, CoreError> {
    let normalized: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{00A0}' && *c != '\u{202F}')
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    Money::parse(&normalized, currency)
}

/// The message for a rule code (`error.<code>`), or a generic one.
pub fn error_text(st: AppState, code: &str) -> String {
    let key = format!("error.{code}");
    let t = tr(st.lang(), &key);
    if t == key {
        tr(st.lang(), "error.internal").into_owned()
    } else {
        t.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_typed_by_people() -> Result<(), CoreError> {
        let da = Currency::DZD;
        assert_eq!(parse_price("1250", da)?.minor(), 125_000);
        assert_eq!(parse_price("1 250,50", da)?.minor(), 125_050);
        assert_eq!(parse_price("1250.5", da)?.minor(), 125_050);
        assert!(matches!(
            parse_price("12,345", da),
            Err(CoreError::TooManyDecimals { .. })
        ));
        assert!(parse_price("abc", da).is_err());
        assert!(parse_price("", da).is_err());
        Ok(())
    }
}
