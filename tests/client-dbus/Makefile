PYTEST_OPTS = --verbose

TOX=tox
.PHONY: lint
lint:
	$(TOX) -c tox.ini -e lint

.PHONY: dbus-tests
dbus-tests:
	py.test-3 ${PYTEST_OPTS} ./tests/dbus

.PHONY: misc-tests
misc-tests:
	py.test-3 ${PYTEST_OPTS} ./tests/misc

.PHONY: fmt
fmt:
	black .

.PHONY: fmt-travis
fmt-travis:
	black . --check

.PHONY: udev-tests
udev-tests:
	py.test-3 ${PYTEST_OPTS} ./tests/udev

.PHONY: tests
tests: dbus-tests misc-tests udev-tests
