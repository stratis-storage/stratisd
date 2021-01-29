.PHONY: lint
lint:
	./check.py check.py
	./check.py update_cargo_crates.py
	./check.py create_release.py

.PHONY: fmt
fmt:
	isort --recursive check.py update_cargo_crates.py
	isort --recursive check.py create_release.py
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --recursive --diff --check-only .
	black . --check
