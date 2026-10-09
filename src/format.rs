pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if value < 1024 {
        return format!("{value} B");
    }
    let mut n = value as f64;
    let mut unit = 0;
    while n >= 1024.0 && unit < UNITS.len() - 1 {
        n /= 1024.0;
        unit += 1;
    }
    format!("{n:.1} {}", UNITS[unit])
}

pub fn parse_human_bytes(input: &str) -> Option<u64> {
    let clean = input.split_whitespace().next()?;
    let split = clean.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let number: f64 = clean[..split].parse().ok()?;
    let unit = clean[split..].to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "b" => 1_f64,
        "kb" | "kib" => 1024_f64,
        "mb" | "mib" => 1024_f64.powi(2),
        "gb" | "gib" => 1024_f64.powi(3),
        "tb" | "tib" => 1024_f64.powi(4),
        _ => return None,
    };
    Some((number * multiplier) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_sizes() {
        assert_eq!(parse_human_bytes("1.5GB (40%)"), Some(1_610_612_736));
        assert_eq!(parse_human_bytes("0B"), Some(0));
    }
}
