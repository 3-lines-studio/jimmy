//! Random bytes, for names that must not repeat.

use std::io::Read;

pub fn bytes(count: usize) -> Vec<u8> {
    let mut buf = vec![0u8; count];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut buf))
        .is_err()
    {
        let seed = std::process::id().to_le_bytes();
        for (index, byte) in buf.iter_mut().enumerate() {
            *byte = seed[index % seed.len()];
        }
    }
    buf
}

pub fn hex(count: usize) -> String {
    bytes(count)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_two_characters_per_byte() {
        assert_eq!(hex(4).len(), 8);
        assert_eq!(hex(0).len(), 0);
        assert_ne!(hex(16), hex(16));
    }
}
