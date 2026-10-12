#!/bin/bash
# the repository root, two levels up from tools/legacy
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )/../.." &> /dev/null && pwd )
source "$SCRIPT_DIR/venv/bin/activate"

export PATH=$SCRIPT_DIR/venv/bin:$SCRIPT_DIR/tippecanoe:$SCRIPT_DIR/valhalla/build:$SCRIPT_DIR/prime_server:$PATH
export OUTPUT_DIR=output
export PLANETILER_JAR=$SCRIPT_DIR/planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
# export JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64
export JAVA_HOME=/Library/Java/JavaVirtualMachines/openjdk-21.jdk/Contents/Home
