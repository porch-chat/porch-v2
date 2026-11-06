$ErrorActionPreference = "Stop"
Set-Location "$PSScriptRoot/.."
cargo build -p porch-flutter --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Set-Location apps/porch_flutter
flutter pub get --enforce-lockfile
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
flutter build windows --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
if (!(Test-Path build/windows/x64/runner/Release/porch_flutter.dll)) {
  throw "Rust bridge DLL missing from bundle"
}
