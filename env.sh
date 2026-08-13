#!/bin/bash
source ./venv/bin/activate

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

export PATH=$SCRIPT_DIR/venv/bin:$SCRIPT_DIR/tippecanoe:$SCRIPT_DIR/valhalla/build:$SCRIPT_DIR/prime_server:$PATH
export OUTPUT_DIR=alpimaps_mbtiles
export PLANETILER_JAR=$SCRIPT_DIR/planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
# export JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64
export JAVA_HOME=/Library/Java/JavaVirtualMachines/openjdk-21.jdk/Contents/Home
