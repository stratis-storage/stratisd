.PHONY: lint
lint:
	pylint create_release.py

.PHONY: fmt
fmt:
	isort .
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --diff --check-only .
	black . --check
