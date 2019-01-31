TOX=tox
.PHONY: lint
lint:
	$(TOX) -c tox.ini -e lint

.PHONY: dbus-tests
dbus-tests:
	true

.PHONY: fmt
fmt:
	yapf --style pep8 --recursive --in-place check.py setup.py src tests

.PHONY: fmt-travis
fmt-travis:
	yapf --style pep8 --recursive --diff check.py setup.py src tests

.PHONY: udev-tests
udev-tests:
	true

.PHONY: tests
tests: dbus-tests udev-tests
