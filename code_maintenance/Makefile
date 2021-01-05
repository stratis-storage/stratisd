.PHONY: lint
lint:
	./check.py check.py
	./check.py update_cargo_crates.py

.PHONY: fmt
fmt:
	isort --recursive check.py update_cargo_crates.py
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --recursive --diff --check-only .
	black . --check
