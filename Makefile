.PHONY: build run

build:
	docker build -t jimmy .

run:
	mkdir -p data
	docker run --rm -it --env-file .env -v $(PWD)/data:/data jimmy
