//! Identificadores opacos: 48 bits de tiempo y 80 de azar, en base32 de
//! Crockford, como un ULID.
//!
//! El identificador de una org no puede salir de su nombre ni del mail de
//! nadie: sería adivinable y contaría de quién es cada cosa. Se genera uno
//! nuevo por entidad y no se deriva de nada.

const ALPHABET: [u8; 32] = *b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const LENGTH: usize = 26;
const RANDOM: usize = 10;

/// Un identificador nuevo. Los 48 bits de tiempo van adelante, así que ordenar
/// por el texto es ordenar por cuándo se creó.
pub fn new() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(&ms.to_be_bytes()[2..8]);
    bytes[6..].copy_from_slice(&crate::random::bytes(RANDOM));
    encode(&bytes)
}

/// 128 bits en 26 caracteres de 5 bits: los dos últimos quedan de relleno.
fn encode(bytes: &[u8; 16]) -> String {
    let mut out = String::with_capacity(LENGTH);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in bytes {
        buffer = ((buffer << 8) | *byte as u32) & ((1 << (bits + 8)) - 1);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// Los milisegundos con los que se generó. Sólo para tests y para ordenar.
#[cfg(test)]
fn millis(id: &str) -> u64 {
    let mut value: u64 = 0;
    for c in id.chars().take(10) {
        let digit = ALPHABET.iter().position(|a| *a as char == c).unwrap() as u64;
        value = (value << 5) | digit;
    }
    value >> 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiene_el_largo_y_el_alfabeto_de_un_ulid() {
        let id = new();
        assert_eq!(id.len(), LENGTH);
        assert!(id.chars().all(|c| ALPHABET.contains(&(c as u8))));
    }

    #[test]
    fn no_se_repiten() {
        let ids: std::collections::HashSet<String> = (0..1000).map(|_| new()).collect();
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn adelante_va_el_tiempo() {
        let antes = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let id = new();
        let despues = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!((antes..=despues).contains(&millis(&id)), "{}", millis(&id));
    }

    #[test]
    fn el_de_despues_ordena_mas_arriba() {
        let primero = new();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let segundo = new();
        assert!(primero < segundo, "{primero} < {segundo}");
    }
}
