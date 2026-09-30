//! El modelo, del otro lado.
//!
//! Lo que corre adentro de un sandbox no ve al proveedor: ve al control plane,
//! que le pone la clave y le pasa la respuesta tal como vino. Así la clave no
//! viaja a ningún lado y el sandbox lleva un pase que sólo sirve para pedirle
//! turnos al modelo de su org.

use crate::store::{Store, Uso};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};

pub struct Modelo {
    /// La base del proveedor, la que usa el control plane para hablarle.
    base: String,
    /// La clave del proveedor: no sale de acá.
    key: String,
    /// Cómo se llega a este control plane desde afuera: es lo que el sandbox
    /// llama.
    publico: String,
    /// El puerto del control plane acá adentro, para los turnos que corren en
    /// este mismo proceso: le piden el modelo a la misma puerta, así todo el
    /// consumo pasa por un solo lugar.
    local: Option<String>,
    pases: Mutex<HashMap<String, String>>,
    /// Dónde se anota lo que se consumió.
    store: OnceLock<Arc<Store>>,
}

impl Modelo {
    pub fn new(base: String, key: String, publico: String, local: Option<String>) -> Modelo {
        Modelo {
            base,
            key,
            publico: publico.trim_end_matches('/').to_string(),
            local,
            pases: Mutex::new(HashMap::new()),
            store: OnceLock::new(),
        }
    }

    /// El consumo se anota acá: es lo que después se mira para saber cuánto
    /// gastó una org, y por dónde pasa todo lo que habla con el modelo.
    pub fn set_store(&self, store: Arc<Store>) {
        let _ = self.store.set(store);
    }

    /// La base que se le da al que corre adentro: pasa por acá.
    pub fn url(&self) -> String {
        format!("{}/modelo", self.publico)
    }

    /// Lo mismo, para un turno que corre en este mismo proceso: la vuelta es
    /// la puerta de casa.
    pub fn url_local(&self) -> Option<String> {
        self.local.clone()
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
    pub fn responder(&self, org: &str, body: &[u8], stream: &mut TcpStream) -> std::io::Result<()> {
        let model = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|pedido| pedido["model"].as_str().map(str::to_string))
            .unwrap_or_else(|| "el modelo".to_string());
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
        let mut pendiente: Vec<u8> = Vec::new();
        loop {
            let leidos = lector.read(&mut buffer)?;
            if leidos == 0 {
                break;
            }
            // El consumo viene en el último pedazo y puede caer partido entre dos
            // lecturas, así que se miran las líneas enteras mientras pasan.
            pendiente.extend_from_slice(&buffer[..leidos]);
            while let Some(fin) = pendiente.iter().position(|byte| *byte == b'\n') {
                let linea: Vec<u8> = pendiente.drain(..=fin).collect();
                self.anotar(org, &model, &linea);
            }
            write!(stream, "{leidos:x}\r\n")?;
            stream.write_all(&buffer[..leidos])?;
            stream.write_all(b"\r\n")?;
            stream.flush()?;
        }
        stream.write_all(b"0\r\n\r\n")?;
        stream.flush()
    }

    /// Lo que diga una línea sobre el consumo, si dice algo.
    fn anotar(&self, org: &str, model: &str, linea: &[u8]) {
        let Some(store) = self.store.get() else {
            return;
        };
        let Some(datos) = linea.strip_prefix(b"data: ") else {
            return;
        };
        let Ok(trozo) = serde_json::from_slice::<serde_json::Value>(datos) else {
            return;
        };
        let uso = &trozo["usage"];
        if uso.is_null() {
            return;
        }
        let tokens = |clave: &str| uso[clave].as_u64().unwrap_or(0);
        let anotado = Uso {
            prompt: tokens("prompt_tokens"),
            completion: tokens("completion_tokens"),
            cached: tokens("prompt_cache_hit_tokens").max(
                uso["prompt_tokens_details"]["cached_tokens"]
                    .as_u64()
                    .unwrap_or(0),
            ),
            calls: 1,
        };
        if anotado.total() == 0 {
            return;
        }
        if let Err(error) = store.sumar_uso(org, model, &anotado) {
            eprintln!("jimmy: no pude anotar el consumo de {org}: {error}");
        }
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
            Some("http://127.0.0.1:8080/modelo".into()),
        );
        assert_eq!(modelo.url(), "https://jimmy.ejemplo/modelo");
        assert_eq!(
            modelo.url_local().as_deref(),
            Some("http://127.0.0.1:8080/modelo")
        );
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
