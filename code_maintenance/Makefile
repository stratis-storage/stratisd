.PHONY: lint
lint:
	./check.py check.py
	./check.py create_release.py
	./check.py set_lower_bounds 
	./check.py _utils.py

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
