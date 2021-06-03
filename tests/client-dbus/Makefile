UNITTEST_OPTS = --verbose

.PHONY: lint
lint:
	pylint src/stratisd_client_dbus --ignore=_introspect.py
	pylint tests

.PHONY: misc-tests
misc-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.misc.test_startup

.PHONY: fmt
fmt:
	isort --recursive check.py src tests
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --recursive --diff --check-only check.py src tests
	black . --check

.PHONY: udev-tests
udev-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_udev
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_predict

.PHONY: tests
tests: udev-tests misc-tests
