$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
if ($env:PRESENTATIONPOINT_GNU_TOOLCHAIN_BIN) {
    $gnuToolchainBin = (Resolve-Path -LiteralPath $env:PRESENTATIONPOINT_GNU_TOOLCHAIN_BIN).Path
    $env:PATH = "$gnuToolchainBin;$env:PATH"
}
Push-Location $projectRoot
try {
    cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt failed" }

    cargo clippy --offline --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "cargo clippy failed" }

    cargo test --offline
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed" }

    cargo run --offline -- --generate-samples target/check-input
    if ($LASTEXITCODE -ne 0) { throw "sample generation failed" }

    cargo run --offline -- --roundtrip-corpus target/check-input target/check-output
    if ($LASTEXITCODE -ne 0) { throw "generated PPTX round-trip failed" }

    $corpus = Get-ChildItem -LiteralPath (Join-Path $projectRoot "testdata") -File -Filter "*.pptx" -ErrorAction SilentlyContinue
    if ($corpus) {
        cargo run --offline -- --roundtrip-corpus testdata target/corpus-roundtrip
        if ($LASTEXITCODE -ne 0) { throw "testdata corpus round-trip failed" }
    }
}
finally {
    Pop-Location
}
