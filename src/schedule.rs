//! La agenda del control plane: qué tareas hay, a cuál le toca y qué contestó.
//!
//! El estado vive en la base, no en el workspace de la org: el control plane
//! tiene que poder leerla y escribirla sin despertar el sandbox. Cada vuelta
//! reclama lo vencido —lo reclama uno solo, y ese es el que lo corre— y lo
//! corre con el agente apuntado a la org de la tarea.

use crate::agent::Agent;
use crate::store::{NewTask, Store, Task};
use crate::transport::{Null, Session, Transport};
use crate::workspace::{place, Place};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TICK: Duration = Duration::from_secs(60);
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

/// La agenda la mueve el control plane. El canal no lleva la tarea: sólo
/// despierta al reloj cuando la web pide una corrida y no vale la pena esperar
/// al próximo minuto.
pub fn spawn(
    transport: Arc<dyn Transport>,
    agent: Agent,
    store: Arc<Store>,
    root: PathBuf,
    workspace: PathBuf,
) -> Sender<()> {
    let (sender, runner) = std::sync::mpsc::channel();
    let offset = tz_offset();
    std::thread::spawn(move || {
        let agenda = Agenda {
            transport: transport.as_ref(),
            agent: &agent,
            store: &store,
            base: Place {
                root,
                workspace,
                org: None,
            },
            offset,
        };
        loop {
            agenda.tick();
            let _ = runner.recv_timeout(TICK);
        }
    });
    sender
}

/// Lo que hace falta para correr lo que le toca a alguien: de dónde salen las
/// tareas, quién las corre y en qué lugar base vive cada org.
struct Agenda<'a> {
    transport: &'a dyn Transport,
    agent: &'a Agent,
    store: &'a Store,
    base: Place,
    offset: i64,
}

impl Agenda<'_> {
    fn tick(&self) {
        let now = now_secs();
        let due = match self.store.claim_due(now) {
            Ok(due) => due,
            Err(error) => {
                eprintln!("jimmy: agenda: no pude ver qué le toca a cada tarea: {error}");
                return;
            }
        };
        for task in due {
            self.run(&task, now);
        }
    }

    fn run(&self, task: &Task, now: i64) {
        let org = match self.store.org(&task.org_id) {
            Ok(Some(org)) => org,
            Ok(None) => {
                eprintln!("jimmy: agenda: {} quedó sin org", task.name);
                return;
            }
            Err(error) => {
                eprintln!(
                    "jimmy: agenda: no pude leer la org de {}: {error}",
                    task.name
                );
                return;
            }
        };
        let home = place(&self.base.root, &self.base.workspace, &org);
        eprintln!("jimmy: agenda: corriendo {} de {}", task.name, org.name);
        let (outbound, session, warning) = outbound(self.transport, task);
        let started = Instant::now();
        let result = self
            .agent
            .at(&home)
            .run_task(outbound, &session, &task.prompt, task.silent);
        let (ok, mut text) = match result {
            Ok(reply) => (true, reply),
            Err(error) => (false, error),
        };
        if let Some(warning) = warning {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&warning);
        }
        let next = next_run(
            task.when_at.as_deref(),
            task.at.as_deref(),
            task.every.as_deref(),
            now,
            self.offset,
        );
        let ms = started.elapsed().as_millis() as i64;
        if let Err(error) = self.store.record_run(task, now, next, ms, ok, &text) {
            eprintln!(
                "jimmy: agenda: no pude anotar la corrida de {}: {error}",
                task.name
            );
        }
    }
}

/// A dónde sale la corrida: al chat del target si el transporte lo conoce, y si
/// no a ningún lado. Un target que no existe no se lleva puesta la corrida: se
/// hace igual, queda en el historial y ahí dice por qué no salió.
fn outbound<'a>(
    transport: &'a dyn Transport,
    task: &Task,
) -> (&'a dyn Transport, Session, Option<String>) {
    let Some(target) = task.target.as_deref() else {
        return (&Null, Session::channel(task.name.as_str()), None);
    };
    match transport.parse_target(target) {
        Ok(session) => (transport, session, None),
        Err(error) => (
            &Null,
            Session::channel(task.name.as_str()),
            Some(format!("⚠️ no salió a {target}: {error}")),
        ),
    }
}

/// Programa una tarea nueva: valida el horario y le calcula la primera vez que
/// le toca. Una tarea sin horario queda para correr a mano.
pub fn program(store: &Store, org: &str, new: NewTask) -> Result<Task, String> {
    let horarios = [new.when_at.is_some(), new.at.is_some(), new.every.is_some()]
        .iter()
        .filter(|hay| **hay)
        .count();
    if horarios > 1 {
        return Err("una tarea corre con un solo horario".into());
    }
    if new.prompt.trim().is_empty() {
        return Err("la tarea necesita algo que hacer".into());
    }
    let name = new.name.trim().to_string();
    if !name.is_empty() && store.task_named(org, &name)?.is_some() {
        return Err(format!("ya hay una tarea que se llama {name}"));
    }
    let offset = tz_offset();
    let next = match (
        new.when_at.as_deref(),
        new.at.as_deref(),
        new.every.as_deref(),
    ) {
        (Some(when), _, _) if when_secs(when, offset).is_none() => {
            return Err("esa fecha no se entiende: se escribe como 2026-09-14T15:00".into())
        }
        (_, Some(at), _) if hhmm(at).is_none() => {
            return Err("esa hora no se entiende: se escribe como 05:00".into())
        }
        (_, _, Some(every)) if period(every).is_none() => {
            return Err("ese intervalo no se entiende: se escribe como 30m, 6h o 2d".into())
        }
        _ => first_run(
            new.when_at.as_deref(),
            new.at.as_deref(),
            new.every.as_deref(),
            now_secs(),
            offset,
        ),
    };
    store.create_task(
        org,
        NewTask {
            name,
            next_run_at: next,
            ..new
        },
    )
}

/// Cuándo le toca la primera vez. Una tarea `when` con la hora ya pasada corre
/// apenas se cree: es una cita que se pidió una sola vez.
fn first_run(
    when_at: Option<&str>,
    at: Option<&str>,
    every: Option<&str>,
    now: i64,
    offset: i64,
) -> Option<i64> {
    if let Some(when) = when_at {
        return when_secs(when, offset);
    }
    if let Some(at) = at {
        return next_at(at, now, offset);
    }
    if every.is_some() {
        return Some(now);
    }
    None
}

/// Cuándo le toca después de correr. `None` es una tarea que ya no vuelve:
/// `when` corre una sola vez, y una tarea sin horario sólo corre a mano.
fn next_run(
    when_at: Option<&str>,
    at: Option<&str>,
    every: Option<&str>,
    now: i64,
    offset: i64,
) -> Option<i64> {
    if when_at.is_some() {
        return None;
    }
    if let Some(at) = at {
        return next_at(at, now, offset);
    }
    every.and_then(period).map(|period| now + period)
}

/// La próxima vez que el reloj local marque esa hora, hoy o mañana.
fn next_at(at: &str, now: i64, offset: i64) -> Option<i64> {
    let (hour, minute) = hhmm(at)?;
    let day = (now + offset * HOUR).div_euclid(DAY);
    let mut candidate = day * DAY + hour * HOUR + minute * 60 - offset * HOUR;
    if candidate <= now {
        candidate += DAY;
    }
    Some(candidate)
}

/// Una cita suelta, en hora local, como instante.
fn when_secs(when: &str, offset: i64) -> Option<i64> {
    let when = when.trim().replace(' ', "T");
    let (date, time) = when.split_once('T')?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.trim().parse().ok()?;
    let month: i64 = parts.next()?.trim().parse().ok()?;
    let day: i64 = parts.next()?.trim().parse().ok()?;
    let (hour, minute) = hhmm(time)?;
    Some(days_from_civil(year, month, day) * DAY + hour * HOUR + minute * 60 - offset * HOUR)
}

fn tz_offset() -> i64 {
    std::env::var("JIMMY_TZ_OFFSET")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn hhmm(s: &str) -> Option<(i64, i64)> {
    let (hour, minute) = s.trim().split_once(':')?;
    let hour: i64 = hour.trim().parse().ok()?;
    let minute: i64 = minute.trim().parse().ok()?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return None;
    }
    Some((hour, minute))
}

fn period(s: &str) -> Option<i64> {
    let s = s.trim();
    let unit = s.chars().last()?;
    let value: i64 = s[..s.len() - unit.len_utf8()].trim().parse().ok()?;
    let factor = match unit {
        's' => 1,
        'm' => 60,
        'h' => HOUR,
        'd' => DAY,
        _ => return None,
    };
    Some(value * factor)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Msg;
    use std::path::Path;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jimmy-agenda-{tag}-{}", crate::random::hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tarea(name: &str, next: Option<i64>) -> NewTask {
        NewTask {
            name: name.into(),
            prompt: "p".into(),
            when_at: None,
            at: None,
            every: Some("1h".into()),
            target: None,
            silent: false,
            next_run_at: next,
        }
    }

    struct Reject;

    impl Transport for Reject {
        fn parse_target(&self, key: &str) -> Result<Session, String> {
            Err(format!("no conozco el chat {key}"))
        }

        fn progress(&self, _: &Session) -> Option<Msg> {
            None
        }

        fn answer(&self, _: &Session, _: Option<Msg>, _: &str) {}

        fn note(&self, _: &Session, _: &str) {}

        fn fail(&self, _: &Session, _: Option<Msg>, _: &str) {}

        fn download(&self, _: &str) -> Result<(String, Vec<u8>), String> {
            Err("no".into())
        }

        fn send_media(&self, _: &Session, _: &Path, _: Option<&str>) -> Result<Msg, String> {
            Err("no".into())
        }
    }

    #[test]
    fn una_cita_corre_una_sola_vez() {
        let when = "2026-09-14T15:00";
        let now = when_secs("2026-09-14T14:00", 0).unwrap();
        assert_eq!(
            first_run(Some(when), None, None, now, 0),
            when_secs(when, 0),
            "la cita es a su hora"
        );
        assert_eq!(next_run(Some(when), None, None, now, 0), None);
        assert_eq!(
            first_run(Some("ayer"), None, None, now, 0),
            None,
            "y una rota no corre"
        );
    }

    #[test]
    fn el_diario_apunta_a_la_proxima_hora() {
        let medianoche = days_from_civil(2026, 9, 14) * DAY;
        let hoy_a_las_9 = medianoche + 9 * HOUR;
        assert_eq!(
            first_run(None, Some("09:00"), None, medianoche, 0),
            Some(hoy_a_las_9)
        );
        assert_eq!(
            first_run(None, Some("09:00"), None, hoy_a_las_9 + 60, 0),
            Some(hoy_a_las_9 + DAY),
            "la de hoy ya pasó"
        );
        assert_eq!(
            next_run(None, Some("09:00"), None, hoy_a_las_9 + 60, 0),
            Some(hoy_a_las_9 + DAY)
        );
        assert_eq!(first_run(None, Some("25:00"), None, medianoche, 0), None);
    }

    #[test]
    fn el_diario_es_la_hora_local() {
        let medianoche = days_from_civil(2026, 9, 14) * DAY;
        let offset = 3;
        assert_eq!(
            first_run(None, Some("09:00"), None, medianoche, offset),
            Some(medianoche + 6 * HOUR),
            "las 9 de UTC-3 son las 12 UTC"
        );
    }

    #[test]
    fn el_de_cada_tanto_suma_su_periodo() {
        let now = 1_000;
        assert_eq!(
            first_run(None, None, Some("6h"), now, 0),
            Some(now),
            "la primera es ya"
        );
        assert_eq!(
            next_run(None, None, Some("6h"), now, 0),
            Some(now + 6 * HOUR)
        );
        assert_eq!(next_run(None, None, Some("30m"), now, 0), Some(now + 1_800));
        assert_eq!(first_run(None, None, Some("raro"), now, 0), Some(now));
        assert_eq!(next_run(None, None, Some("raro"), now, 0), None);
    }

    #[test]
    fn una_tarea_sin_horario_solo_corre_a_mano() {
        assert_eq!(first_run(None, None, None, 1_000, 0), None);
        assert_eq!(next_run(None, None, None, 1_000, 0), None);
    }

    #[test]
    fn parses_hhmm_and_periods() {
        assert_eq!(hhmm("5:07"), Some((5, 7)));
        assert_eq!(hhmm(" 23:59 "), Some((23, 59)));
        assert_eq!(hhmm("24:00"), None);
        assert_eq!(hhmm("nueve"), None);
        assert_eq!(period("30m"), Some(1_800));
        assert_eq!(period("2h"), Some(7_200));
        assert_eq!(period("1d"), Some(86_400));
        assert_eq!(period("45s"), Some(45));
        assert_eq!(period("45"), None);
    }

    #[test]
    fn civil_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 9, 14), 20_710);
        assert_eq!(civil_from_days(20_710), (2026, 9, 14));
        assert_eq!(days_from_civil(2024, 2, 29), 19_782, "año bisiesto");
    }

    fn guardada(name: &str, target: Option<&str>) -> Task {
        Task {
            id: crate::ulid::new(),
            org_id: "01ORG".into(),
            name: name.into(),
            when_at: None,
            at: None,
            every: Some("1h".into()),
            target: target.map(String::from),
            prompt: "p".into(),
            silent: false,
            paused: false,
            next_run_at: Some(1),
            last_read_at: 0,
        }
    }

    #[test]
    fn a_target_the_transport_does_not_know_still_runs_and_says_so() {
        let task = guardada("memoria", Some("123"));
        let (outbound, session, warning) = outbound(&Reject, &task);
        assert!(outbound.progress(&session).is_none());
        assert_eq!(session.key(), "memoria");
        assert!(
            warning.unwrap().contains("no conozco el chat 123"),
            "el historial dice por qué no salió"
        );
    }

    #[test]
    fn a_task_without_a_target_has_nothing_to_report() {
        let task = guardada("memoria", None);
        let (_, session, warning) = outbound(&Reject, &task);
        assert!(warning.is_none());
        assert_eq!(session.key(), "memoria");
    }

    fn wait_runs(store: &Store, task: &str) -> Vec<crate::store::Run> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let runs = store.runs_of(task, 5).unwrap();
            if !runs.is_empty() || Instant::now() > deadline {
                return runs;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// La agenda es una sola y el reloj también, pero cada tarea corre donde
    /// vive su org: una de cada lado, y cada una anota su corrida.
    #[test]
    fn cada_org_corre_sus_tareas() {
        let base = scratch("orgs");
        let root = base.join("root");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = Arc::new(Store::open(&root.join("jimmy.db")).unwrap());
        let (_, bob) = store.register("bob@ejemplo.com").unwrap();
        let (_, ana) = store.register("ana@ejemplo.com").unwrap();
        assert_ne!(bob.id, ana.id);

        let de_bob = store
            .create_task(&bob.id, tarea("resumen", Some(1_000)))
            .unwrap();
        let de_ana = store
            .create_task(&ana.id, tarea("resumen", Some(1_000)))
            .unwrap();

        let agent = Agent::new(
            "http://127.0.0.1:1".into(),
            "model".into(),
            "key".into(),
            None,
            root.clone(),
            workspace.display().to_string(),
            String::new(),
        );
        let _agenda = spawn(
            Arc::new(Null),
            agent,
            store.clone(),
            root.clone(),
            workspace.clone(),
        );

        let runs = wait_runs(&store, &de_bob.id);
        assert_eq!(runs.len(), 1, "la de bob corrió");
        assert!(!runs[0].ok, "sin modelo, y queda dicho");
        assert_eq!(
            wait_runs(&store, &de_ana.id).len(),
            1,
            "y la de ana también, con el mismo nombre"
        );
        let despues = store.task_named(&bob.id, "resumen").unwrap().unwrap();
        assert!(
            despues.next_run_at.unwrap() > 1_000,
            "y la tarea se reprograma"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
