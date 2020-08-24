#!/bin/bash

KEY_DESC=test

plymouth ask-for-password \
	--command="stratis-min key set --capture-key --no-tty $KEY_DESC" \
	--prompt="Enter passphrase"

stratis-min pool setup
