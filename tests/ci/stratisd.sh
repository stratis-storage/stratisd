#!/bin/bash
set -e

export PATH="$HOME/.cargo/bin:$PATH"
export STRATIS_DEPS_DIR=$WORKSPACE/stratis-deps
export RUST_LOG=libstratis=debug
export TEST_BLOCKDEVS_FILE=~/test_config.json

TARGET=$1

if [ -z $TARGET ]
then
   echo "Usage: $0 test-loop | test-real"
   exit 1
fi

# Set WORKSPACE to the top level directory that contains the stratisd git repo
if [ -z $WORKSPACE ]
then
    echo "Required WORKSPACE variable not set. Set WORKSPACE to directory that contains"
    echo "stratisd git repo."
    exit 1
fi

if [ ! -d $WORKSPACE/tests/ ]
then
    echo "$WORKSPACE/tests/ does not exist.  Verify WORKSPACE is set to correct directory."
    exit 1
fi

# Each CI system must have a TEST_BLOCKDEVS_FILE file populated with
# block devices that are safe to use/overwrite on the system.
# Only check for this for "make test-real".
if [ $TARGET == "test-real" ]
then
    if [ -s "$TEST_BLOCKDEVS_FILE" ]
    then
        cp $TEST_BLOCKDEVS_FILE $WORKSPACE/tests/.
    else
        echo "Required file $TEST_BLOCKDEVS_FILE not found."
        exit 1
    fi
fi

cd $WORKSPACE
rustup default 1.31.0
cargo clean
make build
make $TARGET

# If there is a stale STRATIS_DEPS_DIR remove it
if [ -d $STRATIS_DEPS_DIR ]
then
    rm -rf $STRATIS_DEPS_DIR
fi

if [ -d $WORKSPACE/tests/client-dbus ]
then
    echo "Running client-dbus tests"
    export STRATISD=$WORKSPACE/target/x86_64-unknown-linux-gnu/debug/stratisd

    if [ ! -f  /etc/dbus-1/system.d/stratisd.conf ]
    then
        cp $WORKSPACE/stratisd.conf /etc/dbus-1/system.d/
    fi


    if [ ! -x $STRATISD ]
    then
        echo "Required $STRATISD not not found or not executable"
        exit 1
    fi

    mkdir $STRATIS_DEPS_DIR
    cd $STRATIS_DEPS_DIR

    # Clone the python dependencies
    git clone https://github.com/stratis-storage/dbus-python-client-gen.git
    git clone https://github.com/stratis-storage/dbus-client-gen.git
    git clone https://github.com/stratis-storage/into-dbus-python.git
    git clone https://github.com/stratis-storage/dbus-signature-pyparsing.git

    for STRATIS_DEP in dbus-client-gen dbus-signature-pyparsing dbus-python-client-gen into-dbus-python
    do
        cd $STRATIS_DEPS_DIR/$STRATIS_DEP
        git fetch --tags
        LATEST_TAG=$(git describe --tags `git rev-list --tags --max-count=1`)
        echo "checking out $STRATIS_DEP $LATEST_TAG"
        git checkout $LATEST_TAG
    done
    # Set the PYTHONPATH to use the dependencies
    export PYTHONPATH=src:$STRATIS_DEPS_DIR/dbus-client-gen/src:$STRATIS_DEPS_DIR/dbus-python-client-gen/src:$STRATIS_DEPS_DIR/into-dbus-python/src:$STRATIS_DEPS_DIR/dbus-signature-pyparsing/src
    export STRATISD=$WORKSPACE/target/x86_64-unknown-linux-gnu/debug/stratisd
    cd $STRATIS_DEPS_DIR/dbus-client-gen

    cd $WORKSPACE/tests/client-dbus
    make tests
else
    echo "client-dbus directory does not exist: $WORKSPACE/tests/client-dbus"
fi
