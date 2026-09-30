.PHONY: build run fmt lint test imagen-sandbox

build:
	docker build -t jimmy .

run:
	mkdir -p data
	docker run --rm -it --env-file .env -v $(PWD)/data:/data jimmy

fmt:
	cargo fmt

lint:
	cargo clippy -- -D warnings

test:
	node --test web/
	cargo test

# El entorno del agente adentro del sandbox. Se corre cuando cambia el entorno,
# no cuando cambia el código: el agente llega publicado desde el control plane.
imagen-sandbox:
	uv run --with tensorlake python deploy/imagen-sandbox.py
