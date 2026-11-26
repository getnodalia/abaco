// SPDX-License-Identifier: MIT OR Apache-2.0
//! Number formatting to a given count of significant digits.

/// Formats `v` with at most `sig` significant digits (1..=17), trimming trailing zeros.
///
/// Uses plain notation for magnitudes in `[1e-4, 1e7)` (and below `10^sig`), scientific
/// notation (`1.5e-7`) otherwise. `NaN` prints as `NaN` and infinities as `inf`/`-inf`.
///
/// ```
/// use nodalia_units::format_sig;
/// assert_eq!(format_sig(98.10000000000001, 15), "98.1");
/// assert_eq!(format_sig(1.0 / 3.0, 6), "0.333333");
/// assert_eq!(format_sig(1.5e-7, 6), "1.5e-7");
/// assert_eq!(format_sig(-2500.0, 6), "-2500");
/// ```
pub fn format_sig(v: f64, sig: usize) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v == 0.0 {
        return "0".into();
    }
    let sig = sig.clamp(1, 17);
    let s = format!("{:.*e}", sig - 1, v);
    let Some((mant, exp)) = s.split_once('e') else {
        return s;
    };
    let exp: i32 = exp.parse().unwrap_or(0);
    let neg = mant.starts_with('-');
    let mut digits: String = mant.chars().filter(char::is_ascii_digit).collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let sign = if neg { "-" } else { "" };
    let fixed_limit = i32::try_from(sig).unwrap_or(17).max(7);
    if (-4..fixed_limit).contains(&exp) {
        let k = digits.len();
        if exp >= 0 {
            let int_len = usize::try_from(exp).unwrap_or(0) + 1;
            if k <= int_len {
                format!("{sign}{digits}{}", "0".repeat(int_len - k))
            } else {
                format!("{sign}{}.{}", &digits[..int_len], &digits[int_len..])
            }
        } else {
            let zeros = usize::try_from(-exp - 1).unwrap_or(0);
            format!("{sign}0.{}{digits}", "0".repeat(zeros))
        }
    } else if digits.len() > 1 {
        format!("{sign}{}.{}e{exp}", &digits[..1], &digits[1..])
    } else {
        format!("{sign}{digits}e{exp}")
    }
}
