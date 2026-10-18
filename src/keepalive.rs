use crate::agent::Agent;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const TICK: Duration = Duration::from_secs(60);
const TIMEOUT: Duration = Duration::from_secs(5);

pub fn spawn(agent: Agent) {
    let Ok(domain) = std::env::var("RAILWAY_PUBLIC_DOMAIN") else {
        return;
    };
    let domain = domain.trim().to_string();
    if domain.is_empty() {
        return;
    }
    let target = format!("{domain}:443");
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        keep(&target, agent.busy());
    });
}

fn keep(target: &str, busy: bool) -> bool {
    if !busy {
        return false;
    }
    if wake(target) {
        return true;
    }
    eprintln!("jimmy: no pude mantener despierta la instancia con {target}");
    false
}

fn wake(target: &str) -> bool {
    let Ok(addrs) = target.to_socket_addrs() else {
        return false;
    };
    addrs
        .filter_map(|addr| TcpStream::connect_timeout(&addr, TIMEOUT).ok())
        .next()
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn listener() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        (listener, addr)
    }

    #[test]
    fn a_live_port_is_woken() {
        let (_guard, addr) = listener();
        assert!(wake(&addr));
    }

    #[test]
    fn a_dead_port_is_not() {
        assert!(!wake("127.0.0.1:1"));
    }

    #[test]
    fn a_broken_name_is_not() {
        assert!(!wake("nadie.invalid:443"));
    }

    #[test]
    fn a_quiet_instance_sends_nothing() {
        let (listener, addr) = listener();
        assert!(!keep(&addr, false));
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            listener.accept().is_err(),
            "quieta no puede abrir conexiones"
        );
    }

    #[test]
    fn a_busy_instance_touches_the_port_every_tick() {
        let (listener, addr) = listener();
        assert!(keep(&addr, true));
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            listener.accept().is_ok(),
            "con un turno vivo tiene que golpear"
        );
    }
}
