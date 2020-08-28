PYTEST_OPTS = --verbose

.PHONY: lint
lint:
	./check.py stratis_id
	./check.py update_cargo_crates.py

.PHONY: fmt
fmt:
	isort --recursive check.py update_cargo_crates.py stratis_id 
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --recursive --diff --check-only check.py update_cargo_crates.py stratis_id 
	black . --check
