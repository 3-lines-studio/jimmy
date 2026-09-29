//! La máquina de una org: el volumen y el shell del sandbox, del otro lado.
//!
//! Es lo que le hace falta a las herramientas de axe para trabajar en el
//! workspace de una org sin saber que está lejos. Los archivos y los comandos
//! van por el mismo cliente, así que el path que una herramienta lee es el
//! mismo que ve un comando.

use crate::tensorlake::Tensorlake;
use axe::machine::{Entry, Machine};
use std::sync::Arc;

pub struct Remote {
    cliente: Arc<Tensorlake>,
    sandbox: String,
    dir: String,
}

impl Remote {
    pub fn new(cliente: Arc<Tensorlake>, sandbox: &str, dir: &str) -> Remote {
        Remote {
            cliente,
            sandbox: sandbox.to_string(),
            dir: dir.to_string(),
        }
    }
}

impl Machine for Remote {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.cliente.read_file(&self.sandbox, path)
    }

    fn write(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.cliente.write_file(&self.sandbox, path, bytes)
    }

    fn list(&self, path: &str) -> Result<Vec<Entry>, String> {
        self.cliente.list_files(&self.sandbox, path)
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        self.cliente.remove_file(&self.sandbox, path)
    }

    fn run(&self, command: &str, timeout: u64, progress: &mut dyn FnMut(&str)) -> String {
        match self
            .cliente
            .run(&self.sandbox, &self.dir, command, timeout, progress)
        {
            Ok(texto) => texto,
            Err(error) => format!("error: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axe::machine::Local;

    /// Baja el sandbox aunque el test falle a mitad de camino: el plan de
    /// prueba deja uno solo a la vez.
    struct Guardado(Arc<Tensorlake>, String);

    impl Drop for Guardado {
        fn drop(&mut self) {
            let _ = self.0.terminate(&self.1);
        }
    }

    /// La compuerta: el mismo comando por la máquina local y por la remota, y
    /// el mismo texto. Crea recursos, así que corre a mano con la clave:
    ///
    ///     heimdall run -p jimmy -c dev -- cargo test --bin jimmy -- --ignored el_bash_remoto_se_comporta_igual
    #[test]
    #[ignore]
    fn el_bash_remoto_se_comporta_igual() {
        let Some(cliente) = Tensorlake::from_env() else {
            panic!("falta TENSORLAKE_API_KEY");
        };
        let cliente = Arc::new(cliente);
        let name = format!("igual-{}", crate::random::hex(4));
        let image = std::env::var("TENSORLAKE_IMAGE").unwrap_or_else(|_| "jimmy-min".into());
        let creado = cliente.create(&name, &image, "/work", "jimmy-org").unwrap();
        assert_eq!(creado.status, "running", "{creado:?}");
        let _guardado = Guardado(cliente.clone(), creado.id.clone());

        let dir = std::env::temp_dir().join(format!("axe-igual-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let escrito = dir.join("nota.txt");
        std::fs::write(&escrito, "hola\n").unwrap();
        cliente
            .write_file(&name, "/work/nota.txt", b"hola\n")
            .unwrap();

        let local = Local::new(&dir.display().to_string());
        let remoto = Remote::new(cliente.clone(), &name, "/work");

        for (comando, timeout) in [
            ("echo hola", 30),
            ("echo hola; echo al error >&2", 30),
            ("cat nota.txt", 30),
            ("exit 7", 30),
            ("sleep 30", 2),
        ] {
            let de_local = local.run(comando, timeout, &mut |_| {});
            let de_remoto = remoto.run(comando, timeout, &mut |_| {});
            assert_eq!(de_remoto, de_local, "«{comando}» contestó distinto");
        }

        let grande = "head -c 20000 /dev/zero | tr '\\0' 'x'";
        let de_local = local.run(grande, 30, &mut |_| {});
        let de_remoto = remoto.run(grande, 30, &mut |_| {});
        assert!(de_local.contains("Output truncated"), "{de_local}");
        assert!(de_remoto.contains("Output truncated"), "{de_remoto}");
        let cuerpo = |texto: &str| texto.chars().take(16000).collect::<String>();
        assert_eq!(
            cuerpo(&de_remoto),
            cuerpo(&de_local),
            "el cuerpo del truncado"
        );

        let lento = "for i in 1 2 3; do echo $i; sleep 0.4; done";
        let mut del_local = Vec::new();
        let mut del_remoto = Vec::new();
        let de_local = local.run(lento, 30, &mut |p| del_local.push(p.to_string()));
        let de_remoto = remoto.run(lento, 30, &mut |p| del_remoto.push(p.to_string()));
        assert_eq!(de_remoto, de_local);
        assert!(del_local.len() > 1, "el local no progresó: {del_local:?}");
        assert!(
            del_remoto.len() > 1,
            "el remoto no progresó: {del_remoto:?}"
        );

        remoto.run("sleep 3; echo tarde > /work/tarde.txt", 1, &mut |_| {});
        std::thread::sleep(std::time::Duration::from_secs(5));
        assert!(
            remoto.read("/work/tarde.txt").is_err(),
            "un hijo del comando cortado siguió vivo"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
