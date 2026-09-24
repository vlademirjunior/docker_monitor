#!/usr/bin/env bash
# Empacota binários distribuíveis do docker_monitor para Linux e Windows.
#
# Gera em dist/:
#   docker_monitor-<versão>-linux-x86_64.tar.gz     (binário ELF + LEIA-ME + checksums)
#   docker_monitor-<versão>-windows-x86_64.zip       (binário .exe + LEIA-ME + checksums)
#   sha256sums.txt                                   (checksums dos dois pacotes)
#
# Requisitos: cargo, alvo x86_64-pc-windows-gnu (`rustup target add ...`),
# linker MinGW no PATH (`x86_64-w64-mingw32-gcc`), python3, sha256sum, tar.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
ALVO_WIN="x86_64-pc-windows-gnu"

command -v cargo >/dev/null 2>&1 || { echo "ERRO: 'cargo' não encontrado." >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "ERRO: 'python3' não encontrado (usado para montar o .zip)." >&2; exit 1; }
command -v sha256sum >/dev/null 2>&1 || { echo "ERRO: 'sha256sum' não encontrado." >&2; exit 1; }
rustup target list --installed 2>/dev/null | grep -q "$ALVO_WIN" || {
    echo "ERRO: alvo '$ALVO_WIN' não instalado. Rode: rustup target add $ALVO_WIN" >&2
    exit 1
}
command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || {
    echo "ERRO: linker MinGW não encontrado no PATH." >&2
    echo "Debian/Ubuntu: sudo apt install gcc-mingw-w64-x86-64" >&2
    exit 1
}

VERSAO="$(grep -m1 '^version *= *"' "$ROOT/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')"
[ -n "$VERSAO" ] || { echo "ERRO: não foi possível ler a versão em Cargo.toml." >&2; exit 1; }
echo "==> Versão: $VERSAO"

echo "==> Compilando release Linux..."
cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml"
echo "==> Compilando release Windows ($ALVO_WIN)..."
cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml" --target "$ALVO_WIN"

LIN_BIN="$ROOT/target/release/docker_monitor"
WIN_BIN="$ROOT/target/$ALVO_WIN/release/docker_monitor.exe"
[ -x "$LIN_BIN" ] || { echo "ERRO: binário Linux não gerado." >&2; exit 1; }
[ -f "$WIN_BIN" ] || { echo "ERRO: binário Windows não gerado." >&2; exit 1; }

LIN_PACOTE="docker_monitor-$VERSAO-linux-x86_64.tar.gz"
WIN_PACOTE="docker_monitor-$VERSAO-windows-x86_64.zip"

rm -rf "$DIST"
mkdir -p "$DIST"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

montar_stage() { # <origem-template> <nome-pacote> <dir-stage> — gera LEIA-ME.txt no stage
    sed -e "s/__VERSAO__/$VERSAO/g" -e "s/__ARQUIVO__/$2/g" "$1" > "$3/LEIA-ME.txt"
}

# --- Pacote Linux ---
SLIN="$STAGE/linux"
mkdir -p "$SLIN"
cp -p "$LIN_BIN" "$SLIN/docker_monitor"
montar_stage "$ROOT/scripts/LEIA-ME-linux.txt" "$LIN_PACOTE" "$SLIN"
(cd "$SLIN" && sha256sum docker_monitor LEIA-ME.txt > sha256sums.txt)
tar -czf "$DIST/$LIN_PACOTE" -C "$SLIN" docker_monitor LEIA-ME.txt sha256sums.txt
echo "==> Pacote Linux: dist/$LIN_PACOTE"

# --- Pacote Windows ---
SWIN="$STAGE/windows"
mkdir -p "$SWIN"
cp -p "$WIN_BIN" "$SWIN/docker_monitor.exe"
montar_stage "$ROOT/scripts/LEIA-ME-windows.txt" "$WIN_PACOTE" "$SWIN"
# Bloco de notas do Windows agradece: LEIA-ME com quebras CRLF.
sed -i 's/$/\r/' "$SWIN/LEIA-ME.txt"
(cd "$SWIN" && sha256sum docker_monitor.exe LEIA-ME.txt > sha256sums.txt)
python3 - "$SWIN" "$DIST/$WIN_PACOTE" <<'EOF'
import sys, zipfile
origem, destino = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(destino, "w", zipfile.ZIP_DEFLATED) as zf:
    for nome in ("docker_monitor.exe", "LEIA-ME.txt", "sha256sums.txt"):
        zf.write(f"{origem}/{nome}", nome)
EOF
echo "==> Pacote Windows: dist/$WIN_PACOTE"

# --- Checksums dos pacotes + verificação ---
(cd "$DIST" && sha256sum "$LIN_PACOTE" "$WIN_PACOTE" > sha256sums.txt)
echo "==> Verificando pacotes..."
file "$DIST/$LIN_PACOTE" | grep -q "gzip compressed data" || { echo "ERRO: $LIN_PACOTE não é gzip." >&2; exit 1; }
python3 - "$DIST/$WIN_PACOTE" <<'EOF'
import sys, zipfile
nomes = set(zipfile.ZipFile(sys.argv[1]).namelist())
assert nomes == {"docker_monitor.exe", "LEIA-ME.txt", "sha256sums.txt"}, nomes
EOF
VDIR="$(mktemp -d)"
trap 'rm -rf "$STAGE" "$VDIR"' EXIT
tar -xzf "$DIST/$LIN_PACOTE" -C "$VDIR"
file "$VDIR/docker_monitor" | grep -q "ELF 64-bit" || { echo "ERRO: binário Linux não é ELF 64-bit." >&2; exit 1; }
(cd "$VDIR" && sha256sum -c sha256sums.txt --quiet)
mkdir -p "$VDIR/win"
python3 - "$DIST/$WIN_PACOTE" "$VDIR/win" <<'EOF'
import sys, zipfile
zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])
EOF
file "$VDIR/win/docker_monitor.exe" | grep -q "PE32+" || { echo "ERRO: binário Windows não é PE32+." >&2; exit 1; }
(cd "$VDIR/win" && sha256sum -c sha256sums.txt --quiet)
(cd "$DIST" && sha256sum -c sha256sums.txt --quiet)

echo "==> OK: pacotes verificados em dist/:"
ls -la "$DIST"
