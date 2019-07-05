PYTEST_OPTS = --verbose

TOX=tox
.PHONY: lint
lint:
	$(TOX) -c tox.ini -e lint

.PHONY: dbus-tests
dbus-tests:
	py.test-3 ${PYTEST_OPTS} ./tests/dbus

.PHONY: fmt
fmt:
	yapf --style pep8 --recursive --in-place check.py setup.py src tests

.PHONY: fmt-travis
fmt-travis:
	yapf --style pep8 --recursive --diff check.py setup.py src tests

.PHONY: udev-tests
udev-tests:
	py.test-3 ${PYTEST_OPTS} ./tests/udev

.PHONY: tests
tests: dbus-tests udev-tests
