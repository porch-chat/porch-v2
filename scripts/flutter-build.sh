#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -p porch-flutter --locked
cd apps/porch_flutter
flutter pub get --enforce-lockfile
flutter build linux --release
# CMake installs libporch_flutter.so beside Flutter and WebRTC shared libraries.
test -s build/linux/x64/release/bundle/lib/libporch_flutter.so
