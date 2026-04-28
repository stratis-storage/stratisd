UNITTEST_OPTS = --verbose

.PHONY: lint
lint:
	ruff check
	pyright

.PHONY: startup-tests
startup-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_startup

.PHONY: start-tests
start-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_start

.PHONY: extend-tests
extend-tests:
	python3 -m unittest ${UNITTEST_OPTS} tests.udev.test_extend

.PHONY: fmt
fmt:
	ruff check --fix --select I
	ruff format

.PHONY: fmt-ci
fmt-ci:
	ruff check --select I
	ruff format --check

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
