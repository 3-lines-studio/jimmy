.PHONY: build run fmt lint test

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
