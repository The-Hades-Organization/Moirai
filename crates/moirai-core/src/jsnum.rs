//! `Number(texto)` do JS para as bordas que leem número de texto (`src/lib/moirai.ts`, `commands/moirai.ts`).

/// `Number(s)` (ECMA-262 §7.1.4.1.1): vazio e só espaço valem 0; `0x`/`0o`/`0b` e `Infinity` são aceitos;
/// `inf`, `nan` e `_` do Rust não. `None` é `NaN`.
pub fn js_number(text: &str) -> Option<f64> {
    let s = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if s.is_empty() {
        return Some(0.0);
    }
    let radix = match s.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = &s[2..];
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return None;
        }
        return Some(digits.chars().fold(0.0, |acc, c| {
            acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
        }));
    }
    match s {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let numeric = body
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        && body
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_digit() || b == b'.');
    if !numeric {
        return None;
    }
    s.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::js_number;

    #[test]
    fn dado_texto_numerico_entao_converte_como_o_js() {
        assert_eq!(js_number("5"), Some(5.0));
        assert_eq!(js_number(" 7 "), Some(7.0));
        assert_eq!(js_number(""), Some(0.0));
        assert_eq!(js_number("1e2"), Some(100.0));
        assert_eq!(js_number(".5"), Some(0.5));
        assert_eq!(js_number("5."), Some(5.0));
        assert_eq!(js_number("+3"), Some(3.0));
        assert_eq!(js_number("0x10"), Some(16.0));
        assert_eq!(js_number("0b11"), Some(3.0));
        assert_eq!(js_number("-Infinity"), Some(f64::NEG_INFINITY));
    }

    #[test]
    fn dado_texto_nao_numerico_entao_nan() {
        for s in [
            "abc", "inf", "nan", "1_0", "0x", "--1", "1 2", "5%", "e5", ".",
        ] {
            assert_eq!(js_number(s), None, "{s}");
        }
    }
}
