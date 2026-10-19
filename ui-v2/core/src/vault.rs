#[derive(Clone, PartialEq)]
pub struct Environment {
    pub project: String,
    pub env: String,
    pub keys: u64,
}

#[derive(Clone, PartialEq)]
pub struct Group {
    pub project: String,
    pub environments: Vec<Environment>,
}

pub fn key(project: &str, env: &str) -> String {
    format!("{project}/{env}")
}

pub fn split(key: &str) -> Option<(String, String)> {
    let (project, env) = key.split_once('/')?;
    Some((project.to_string(), env.to_string()))
}

pub fn scope(admin: bool, project: &str, env: &str) -> String {
    match admin {
        true => "admin".to_string(),
        false => format!("{project}/{env}"),
    }
}

pub fn projects(environments: &[Environment]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for environment in environments {
        match groups
            .iter_mut()
            .find(|group| group.project == environment.project)
        {
            Some(group) => group.environments.push(environment.clone()),
            None => groups.push(Group {
                project: environment.project.clone(),
                environments: vec![environment.clone()],
            }),
        }
    }
    for group in groups.iter_mut() {
        group.environments.sort_by(|a, b| a.env.cmp(&b.env));
    }
    groups.sort_by(|a, b| {
        let weight = |project: &str| usize::from(project != "general");
        weight(&a.project)
            .cmp(&weight(&b.project))
            .then_with(|| a.project.cmp(&b.project))
    });
    groups
}

pub fn count(keys: u64) -> String {
    match keys {
        1 => "1 clave".to_string(),
        keys => format!("{keys} claves"),
    }
}

pub fn mask(value: &str) -> String {
    "•".repeat(value.chars().count().min(16))
}

pub fn action(action: &str) -> String {
    let text = match action {
        "get-secrets" => "leyó",
        "set" => "escribió",
        "unset" => "borró",
        "env-create" => "creó el entorno",
        "env-drop" => "borró el entorno",
        "token-create" => "creó un token",
        "token-revoke" => "revocó un token",
        other => other,
    };
    text.to_string()
}

pub fn clock(day: &str, clock: &str) -> String {
    let parts: Vec<&str> = day.split('-').collect();
    match (parts.len(), parts.get(1), parts.get(2)) {
        (3, Some(month), Some(date)) => format!("{date}/{month} {clock}"),
        _ => format!("{day} {clock}"),
    }
}

pub fn value(text: &str) -> String {
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_is_named_project_and_env() {
        assert_eq!(key("picsel", "dev"), "picsel/dev");
        assert_eq!(split("picsel/dev"), Some(("picsel".into(), "dev".into())));
        assert_eq!(split("picsel"), None);
    }

    #[test]
    fn a_token_scope_reads_without_opening_it() {
        assert_eq!(scope(false, "*", "dev"), "*/dev");
        assert_eq!(scope(true, "_", "_"), "admin");
    }

    #[test]
    fn environments_group_by_project_with_general_first() {
        let environments = vec![
            Environment { project: "zeta".into(), env: "prd".into(), keys: 0 },
            Environment { project: "general".into(), env: "dev".into(), keys: 1 },
            Environment { project: "alfa".into(), env: "prd".into(), keys: 2 },
            Environment { project: "alfa".into(), env: "dev".into(), keys: 0 },
        ];
        let groups = projects(&environments);
        let names: Vec<&str> = groups.iter().map(|group| group.project.as_str()).collect();
        assert_eq!(names, ["general", "alfa", "zeta"]);
        let alfa: Vec<&str> = groups[1]
            .environments
            .iter()
            .map(|environment| environment.env.as_str())
            .collect();
        assert_eq!(alfa, ["dev", "prd"]);
        assert!(projects(&[]).is_empty());
    }

    #[test]
    fn keys_are_counted_in_singular_and_plural() {
        assert_eq!(count(0), "0 claves");
        assert_eq!(count(1), "1 clave");
        assert_eq!(count(12), "12 claves");
    }

    #[test]
    fn a_hidden_value_does_not_say_how_long_it_is() {
        assert_eq!(mask("abc"), "•••");
        assert_eq!(mask(&"x".repeat(40)), "•".repeat(16));
        assert_eq!(mask(""), "");
    }

    #[test]
    fn what_jimmy_did_reads_in_spanish() {
        assert_eq!(action("get-secrets"), "leyó");
        assert_eq!(action("token-create"), "creó un token");
        assert_eq!(action("otra-cosa"), "otra-cosa");
    }

    #[test]
    fn the_audit_date_reads_at_a_glance() {
        assert_eq!(clock("2026-09-26", "05:07"), "26/09 05:07");
    }

    #[test]
    fn values_are_trimmed() {
        assert_eq!(value("  AUTHOR  "), "AUTHOR");
    }
}
