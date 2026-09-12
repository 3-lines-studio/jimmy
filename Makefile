.PHONY: build run fmt lint test

build:
	docker build -t jimmy .

run:
	mkdir -p data
	docker run --rm -it --env-file .env -v $(PWD)/data:/data jimmy

fmt:
	cargo +nightly fmt

lint:
	cargo +nightly clippy -- -D warnings

test:
	cargo +nightly test
