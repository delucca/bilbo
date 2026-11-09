use crate::key::{Value, quote};

pub(crate) const KIB: u64 = 1 << 10;
pub const MIB: u64 = 1 << 20;
pub const GIB: u64 = 1 << 30;

/// Bytes from `8 MiB`, `8MiB`, `512 KiB`, `1 GiB` or a bare `8388608`; no decimal unit, so `8M`
/// and `8 MB` are refused rather than read as 8,000,000.
pub(crate) fn parse(text: &str) -> Option<u64> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let number: u64 = text[..digits].parse().ok()?;
    let rest = &text[digits..];
    let scale = match rest.strip_prefix(' ').unwrap_or(rest) {
        "" if rest.is_empty() => 1,
        "KiB" => KIB,
        "MiB" => MIB,
        "GiB" => GIB,
        _ => return None,
    };
    number.checked_mul(scale)
}

/// A size key's value: a string `parse` reads, or a TOML integer of bytes.
pub fn read(value: &Value<'_>) -> Option<u64> {
    match value.text() {
        Some(text) => parse(text),
        None => value.integer().and_then(|bytes| u64::try_from(bytes).ok()),
    }
}

/// As TOML text: `"8 MiB"` in the largest unit that divides it, else the bare integer.
pub fn show(bytes: u64) -> String {
    let unit = [(GIB, "GiB"), (MIB, "MiB"), (KIB, "KiB")]
        .into_iter()
        .find(|(size, _)| bytes > 0 && bytes.is_multiple_of(*size));
    match unit {
        Some((size, name)) => quote(&format!("{} {name}", bytes / size)),
        None => bytes.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{GIB, KIB, MIB, parse, show};

    #[test]
    fn a_size_is_binary_units_or_bare_bytes() {
        for (text, bytes) in [
            ("8 MiB", 8 * MIB),
            ("8MiB", 8 * MIB),
            ("512 KiB", 512 * KIB),
            ("1 GiB", GIB),
            ("1GiB", GIB),
            ("8388608", 8 * MIB),
            ("08 MiB", 8 * MIB),
            ("0", 0),
        ] {
            assert_eq!(parse(text), Some(bytes), "{text:?}");
        }
    }

    #[test]
    fn decimal_units_case_slips_and_stray_spaces_are_refused() {
        for text in [
            "8M",
            "8MB",
            "8 MB",
            "8 mb",
            "8m",
            "8 mib",
            "8 MIB",
            "8 Mi",
            "8 B",
            "1.5 GiB",
            "-1",
            "+8",
            "0x10",
            "8_388_608",
            "",
            "MiB",
            " 8 MiB",
            "8 MiB ",
            "8388608 ",
            "8  MiB",
            "8\tMiB",
            "99999999999 GiB",
            "18446744073709551616",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_size_shows_in_the_largest_unit_that_divides_it() {
        assert_eq!(show(8 * MIB), "\"8 MiB\"");
        assert_eq!(show(GIB), "\"1 GiB\"");
        assert_eq!(show(1536 * KIB), "\"1536 KiB\"");
        assert_eq!(show(9_000_000), "9000000");
        assert_eq!(show(0), "0");
        for bytes in [MIB, 3 * MIB + 4 * KIB, GIB, 9_000_001] {
            let text = show(bytes);
            let bare = text.trim_matches('"');
            assert_eq!(parse(bare), Some(bytes), "{text}");
        }
    }
}
