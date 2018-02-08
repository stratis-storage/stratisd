TOX=tox

.PHONY: lint
lint:
	$(TOX) -c tox.ini -e lint

dbus-tests:
	py.test-3 ./tests/dbus
