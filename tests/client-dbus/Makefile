UNITTEST_OPTS = --verbose

.PHONY: lint
lint:
	./check.py src/stratisd_client_dbus
	./check.py tests

.PHONY: misc-tests
misc-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.misc.test_startup.TestUniqueInstance.test_unique_instance

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
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_udev

.PHONY: tests
tests: misc-tests udev-tests
