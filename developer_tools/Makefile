.PHONY: lint
lint:
	./check.py check.py
	./check.py stratis_id

.PHONY: fmt
fmt:
	isort --recursive check.py stratis_id
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --recursive --diff --check-only check.py stratis_id
	black . --check
