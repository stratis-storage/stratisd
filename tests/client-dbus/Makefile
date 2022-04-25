UNITTEST_OPTS = --verbose

.PHONY: lint
lint:
	pylint src/stratisd_client_dbus --ignore=_introspect.py
	pylint tests --disable=R0801

.PHONY: misc-tests
misc-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.misc.test_startup

.PHONY: fmt
fmt:
	isort src tests
	black .

.PHONY: fmt-travis
fmt-travis:
	isort --diff --check-only src tests
	black . --check

.PHONY: tang-tests
tang-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_bind

.PHONY: udev-tests
udev-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_udev

.PHONY: predict-tests
predict-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_predict

.PHONY: filesystem-predict-tests
filesystem-predict-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.misc.test_predict

.PHONY: tests
tests: udev-tests misc-tests
