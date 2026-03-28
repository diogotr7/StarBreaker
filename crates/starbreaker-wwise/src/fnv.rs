/// Wwise FNV-1 32-bit hash (lowercase input).
pub fn fnv1_hash(name: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in name.to_lowercase().bytes() {
        h = h.wrapping_mul(16777619);
        h ^= b as u32;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_string() {
        assert_eq!(fnv1_hash(""), 2166136261);
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!(fnv1_hash("Play"), fnv1_hash("play"));
        assert_eq!(fnv1_hash("PLAY"), fnv1_hash("play"));
    }
}
