## Proyectos y git

`projects/<nombre>/` es una carpeta por trabajo. El trabajo puede ser código o no, y no necesita repo. Si es un clon de un repo, el nombre es el del repo. Git solo aplica cuando hay un repo:

- Clon de un repo (el de jimmy es `projects/jimmy`): seguí el flujo de abajo.
- Carpeta sin git (un documento, una presentación, archivos generados): es solo una carpeta. No le corras `git reset` ni `git clean`, ni la conviertas en repo salvo que haga falta.

El clon es descartable: el flujo resetea y limpia sin piedad, así que nada que no esté pusheado sobrevive. No dejes trabajo sin pushear en `projects/<nombre>/`.

En el repo de jimmy hay dos ramas de deploy: `dev` es la mía y `main` es producción, la que deploya la instancia estable. Todo cambio mío va a `dev`; `main` solo recibe promociones que decide {{usuario}}. En el resto de los repos la base sigue siendo `main`.

Flujo para un repo:

1. Reusá el clon si existe. Si no: `git clone https://github.com/<owner>/<nombre> projects/<nombre>`.
2. Antes de **cada** rama, no una vez por sesión: `git fetch origin && git checkout dev && git reset --hard origin/dev && git clean -fd`, adentro de `projects/<nombre>`. El clon queda viejo entre tareas y una rama que sale de un `dev` viejo nace con conflictos.
3. Rama nueva desde la última base: `git checkout -b <tema> origin/dev`, nunca desde la rama del PR anterior.
4. Un cambio lógico por rama. Commits chicos, en imperativo y en inglés, como los del repo.
5. Probá y formateá antes de pushear: `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`. O `make fmt lint test`.
6. `git status` para revisar. No commitees secretos ni artefactos (`target/`, `.env`, `data/`).
7. `git commit`, `git push -u origin <tema>`, `gh pr create --base dev`. Avisale a {{usuario}} con el link.

Después de que mergean: `git checkout dev && git pull --ff-only && git branch -d <tema>`.

Nunca pushees a `main` ni a `dev` directo, ni a repos ajenos. Para promover `dev` a `main`, abrí el PR `dev → main` y esperá el ok de {{usuario}}. No dejes ramas viejas ni clones a medias.
