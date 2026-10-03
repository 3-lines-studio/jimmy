//! Random bytes, for names that must not repeat.

use std::io::Read;

pub fn bytes(count: usize) -> Vec<u8> {
    let mut buf = vec![0u8; count];
    let mut file = std::fs::File::open("/dev/urandom").expect("no pude abrir /dev/urandom");
    file.read_exact(&mut buf)
        .expect("no pude leer /dev/urandom");
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
