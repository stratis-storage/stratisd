UNITTEST_OPTS = --verbose

# Ignore bandit B404 errors. Any import of the subprocess module causes this
# error. We know what we are doing when we import that module and do not
# need to be warned.
BANDIT_SKIP = --skip B404,B603,B311

.PHONY: lint
lint:
	pylint src/stratisd_client_dbus --ignore=_introspect.py
	pylint tests --disable=R0801
	bandit --recursive ./src ${BANDIT_SKIP}
	bandit --recursive ./tests ${BANDIT_SKIP},B101
	pyright

.PHONY: startup-tests
startup-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_startup

.PHONY: fmt
fmt:
	isort src tests
	black .

.PHONY: fmt-ci
fmt-ci:
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

.PHONY: dump-metadata-tests
dump-metadata-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_dump

.PHONY: revert-tests
revert-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_revert
