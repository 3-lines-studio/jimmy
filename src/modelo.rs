//! El modelo, del otro lado.
//!
//! Lo que corre adentro de un sandbox no ve al proveedor: ve al control plane,
//! que le pone la clave y le pasa la respuesta tal como vino. Así la clave no
//! viaja a ningún lado y el sandbox lleva un pase que sólo sirve para pedirle
//! turnos al modelo de su org.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;

pub struct Modelo {
    /// La base del proveedor, la que usa el control plane para hablarle.
    base: String,
    /// La clave del proveedor: no sale de acá.
    key: String,
    /// Cómo se llega a este control plane desde afuera: es lo que el sandbox
    /// llama.
    publico: String,
    pases: Mutex<HashMap<String, String>>,
}

impl Modelo {
    pub fn new(base: String, key: String, publico: String) -> Modelo {
        Modelo {
            base,
            key,
            publico: publico.trim_end_matches('/').to_string(),
            pases: Mutex::new(HashMap::new()),
        }
    }

    /// La base que se le da al que corre adentro: pasa por acá.
    pub fn url(&self) -> String {
        format!("{}/modelo", self.publico)
    }

    /// El pase de una org: el mismo mientras este proceso viva. No es una
    /// sesión ni da acceso a nada más que al modelo de esa org.
    pub fn pase(&self, org: &str) -> String {
        let mut pases = self.pases.lock().unwrap();
        if let Some((pase, _)) = pases.iter().find(|(_, dueno)| dueno.as_str() == org) {
            return pase.clone();
        }
        let pase = crate::random::hex(16);
        pases.insert(pase.clone(), org.to_string());
        pase
    }

    /// De quién es un pase, si es de alguien.
    pub fn org(&self, pase: &str) -> Option<String> {
        self.pases.lock().unwrap().get(pase).cloned()
    }

    /// Lo que pide el sandbox se le pide al proveedor con la clave de verdad, y
    /// lo que contesta vuelve como vino, en pedazos: el turno que se ve
    /// escribirse mientras escribe depende de que acá no se guarde nada.
    pub fn responder(&self, body: &[u8], stream: &mut TcpStream) -> std::io::Result<()> {
        let url = format!("{}/chat/completions", self.base.trim_end_matches('/'));
        let pedido = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.key))
            .set("Content-Type", "application/json");
        let respuesta = match pedido.send_bytes(body) {
            Ok(respuesta) => respuesta,
            Err(ureq::Error::Status(code, respuesta)) => {
                return self.pasar(code, respuesta.into_reader(), stream)
            }
            Err(error) => {
                return write!(
                    stream,
                    "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\n\
                     Content-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .map_err(|_| std::io::Error::other(error.to_string()))
                .and_then(|_| stream.flush());
            }
        };
        let tipo = respuesta
            .header("content-type")
            .unwrap_or("text/event-stream")
            .to_string();
        let status = respuesta.status();
        let mut lector = respuesta.into_reader();
        write!(
            stream,
            "HTTP/1.1 {status} {}\r\nContent-Type: {tipo}\r\n\
             Transfer-Encoding: chunked\r\nCache-Control: no-store\r\n\
             Connection: close\r\n\r\n",
            crate::http::reason(status)
        )?;
        let mut buffer = [0u8; 8192];
        loop {
            let leidos = lector.read(&mut buffer)?;
            if leidos == 0 {
                break;
            }
            write!(stream, "{leidos:x}\r\n")?;
            stream.write_all(&buffer[..leidos])?;
            stream.write_all(b"\r\n")?;
            stream.flush()?;
        }
        stream.write_all(b"0\r\n\r\n")?;
        stream.flush()
    }

    fn pasar(
        &self,
        status: u16,
        mut lector: Box<dyn Read + Send + Sync + 'static>,
        stream: &mut TcpStream,
    ) -> std::io::Result<()> {
        let mut body = Vec::new();
        lector.read_to_end(&mut body)?;
        crate::http::respond(stream, status, "application/json", &[], &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_pase_es_de_una_org_y_es_siempre_el_mismo() {
        let modelo = Modelo::new(
            "https://api.ejemplo/v1/".into(),
            "la-clave".into(),
            "https://jimmy.ejemplo/".into(),
        );
        assert_eq!(modelo.url(), "https://jimmy.ejemplo/modelo");
        let pase = modelo.pase("org-1");
        assert_eq!(
            modelo.pase("org-1"),
            pase,
            "el mismo pase para la misma org"
        );
        assert_ne!(modelo.pase("org-2"), pase, "cada org tiene el suyo");
        assert_eq!(modelo.org(&pase).as_deref(), Some("org-1"));
        assert!(
            modelo.org("cualquiera").is_none(),
            "un pase inventado no sirve"
        );
    }
}
