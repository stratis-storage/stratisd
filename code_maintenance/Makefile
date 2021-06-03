.PHONY: lint
lint:
	pylint create_release.py
	pylint set_lower_bounds
	pylint _utils.py

.PHONY: fmt
fmt:
	isort --recursive .
	black .
	black set_lower_bounds

.PHONY: fmt-travis
fmt-travis:
	isort --recursive --diff --check-only .
	black . --check
	black set_lower_bounds --check
