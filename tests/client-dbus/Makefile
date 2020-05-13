PYTEST_OPTS = --verbose

.PHONY: lint
lint:
	./check.py src/stratisd_client_dbus
	./check.py tests

.PHONY: dbus-tests
dbus-tests:
	py.test-3 ${PYTEST_OPTS} ./tests/dbus

.PHONY: misc-tests
misc-tests:
	py.test-3 ${PYTEST_OPTS} ./tests/misc

.PHONY: fmt
fmt:
	isort --recursive check.py setup.py src tests
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --recursive --diff --check-only check.py setup.py src tests
	black . --check

.PHONY: udev-tests
udev-tests:
	python3 -m unittest --verbose tests.udev.test_udev

.PHONY: tests
tests: dbus-tests misc-tests udev-tests
