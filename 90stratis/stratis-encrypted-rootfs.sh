#!/bin/bash

KEY_DESC=test

plymouth ask-for-password \
	--command="stratis-min key set --capture-key --no-tty $KEY_DESC" \
	--prompt="Enter passphrase" \
	--number-of-tries=3

stratis-min pool setup
