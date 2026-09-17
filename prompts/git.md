## Proyectos y git

`projects/<nombre>/` es una carpeta por trabajo. El trabajo puede ser código o no, y no necesita repo. Si es un clon de un repo, el nombre es el del repo. Git solo aplica cuando hay un repo:

- Clon de un repo (el de jimmy es `projects/jimmy`): seguí el flujo de abajo.
- Carpeta sin git (un documento, una presentación, archivos generados): es solo una carpeta. No le corras `git reset` ni `git clean`, ni la conviertas en repo salvo que haga falta.

El clon es descartable: el flujo resetea y limpia sin piedad, así que nada que no esté pusheado sobrevive. No dejes trabajo sin pushear en `projects/<nombre>/`.

Flujo para un repo:

1. Reusá el clon si existe. Si no: `git clone https://github.com/<owner>/<nombre> projects/<nombre>`.
2. Dejalo limpio y basado en el último `main`: `git fetch origin && git checkout main && git reset --hard origin/main && git clean -fd`, adentro de `projects/<nombre>`.
3. Rama nueva desde el último `main`: `git checkout -b <tema> origin/main`.
4. Un cambio lógico por rama. Commits chicos, en imperativo y en inglés, como los del repo.
5. Probá y formateá antes de pushear: `cargo +nightly fmt`, `cargo +nightly clippy -- -D warnings`, `cargo +nightly test`. O `make fmt lint test`.
6. `git status` para revisar. No commitees secretos ni artefactos (`target/`, `.env`, `data/`).
7. `git commit`, `git push -u origin <tema>`, `gh pr create`. Avisale a {{usuario}} con el link.

Después de que mergean: `git checkout main && git pull --ff-only && git branch -d <tema>`.

Nunca pushees a `main` ni a repos ajenos. No dejes ramas viejas ni clones a medias.
