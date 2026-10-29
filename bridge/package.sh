#!/bin/sh
# Rebuild the exact Bridge binary bundled by deepslate-core.
set -eu
cd "$(dirname "$0")"
gradle_bin=${DEEPSLATE_GRADLE_BIN:-./gradlew}
if [ "${DEEPSLATE_GRADLE_OFFLINE:-0}" = 1 ]; then
    "$gradle_bin" --offline build
else
    "$gradle_bin" build
fi
mkdir -p ../core/assets
cp build/libs/deepslate-bridge-0.5.0.jar ../core/assets/deepslate-bridge-0.5.0.jar
