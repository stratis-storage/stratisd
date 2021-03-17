.PHONY: lint
lint:
	./check.py check.py
	./check.py stratis_id.py

.PHONY: fmt
fmt:
	isort *
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --diff --check-only *
	black . --check
