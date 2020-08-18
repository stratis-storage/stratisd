PYTEST_OPTS = --verbose

.PHONY: lint
lint:
	./check.py stratis_id
	./check.py cargo

.PHONY: fmt
fmt:
	isort --recursive check.py cargo stratis_id 
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --recursive --diff --check-only check.py cargo stratis_id 
	black . --check
