#!/usr/bin/env bash
# Setup do docker_monitor: compila em release, instala o binário em
# ~/.cargo/bin (ou $CARGO_HOME/bin), garante esse diretório no PATH do
# shell (bash/zsh/fish, Linux ou macOS) e cria o atalho `dm` para
# `docker_monitor`. Idempotente: pode rodar de novo.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
MARCA="# docker_monitor setup"
MARCA_ALIAS="# docker_monitor alias dm"

command -v cargo >/dev/null 2>&1 || {
    echo "ERRO: 'cargo' não encontrado. Instale o Rust: https://rustup.rs" >&2
    exit 1
}

echo "==> Compilando (release)..."
cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml"

echo "==> Instalando binário em $BIN_DIR ..."
cargo install --path "$ROOT" --quiet

# Detecta o shell do usuário para escolher o arquivo rc.
SHELL_NOME="$(basename "${SHELL:-bash}")"
case "$SHELL_NOME" in
    zsh) RC="$HOME/.zshrc" ;;
    fish) RC="$HOME/.config/fish/config.fish" ;;
    *) RC="$HOME/.bashrc" ;;
esac

if [[ ":$PATH:" == *":$BIN_DIR:"* ]]; then
    echo "==> $BIN_DIR já está no PATH desta sessão."
elif [ "$SHELL_NOME" = "fish" ]; then
    mkdir -p "$(dirname "$RC")"
    if ! grep -qF "$MARCA" "$RC" 2>/dev/null; then
        printf '\n%s\nset -gx PATH %s $PATH\n' "$MARCA" "$BIN_DIR" >>"$RC"
        echo "==> PATH configurado em $RC"
    fi
else
    touch "$RC"
    if ! grep -qF "$MARCA" "$RC" 2>/dev/null; then
        printf '\n%s\nexport PATH="%s:$PATH"\n' "$MARCA" "$BIN_DIR" >>"$RC"
        echo "==> PATH configurado em $RC"
    else
        echo "==> $RC já contém a configuração."
    fi
fi

# Atalho `dm` -> `docker_monitor` no shell (independe do PATH acima, então
# quem já rodou o setup antigo também ganha o atalho ao rodar de novo).
if [ "$SHELL_NOME" = "fish" ]; then
    mkdir -p "$(dirname "$RC")"
    ALIAS_LINHA="alias dm docker_monitor"
    ALIAS_EXISTENTE='^alias dm[[:space:]]'
else
    ALIAS_LINHA='alias dm="docker_monitor"'
    ALIAS_EXISTENTE='^[[:space:]]*alias dm='
fi
touch "$RC"
if grep -qF "$ALIAS_LINHA" "$RC" 2>/dev/null; then
    echo "==> $RC já contém o atalho 'dm'."
elif grep -qE "$ALIAS_EXISTENTE" "$RC" 2>/dev/null; then
    echo "==> AVISO: $RC já tem outro alias 'dm'; mantido como está."
else
    printf '\n%s\n%s\n' "$MARCA_ALIAS" "$ALIAS_LINHA" >>"$RC"
    echo "==> Atalho 'dm' configurado em $RC"
fi

# Atalho imediato (funciona sem reabrir o terminal): link `dm` ao lado do binário.
if [ -e "$BIN_DIR/dm" ] && [ ! -L "$BIN_DIR/dm" ]; then
    echo "==> AVISO: '$BIN_DIR/dm' já existe e não é um link; mantido como está."
else
    ln -sf docker_monitor "$BIN_DIR/dm"
    echo "==> Link '$BIN_DIR/dm' -> docker_monitor criado."
fi

hash -r 2>/dev/null || true
if command -v docker_monitor >/dev/null 2>&1; then
    echo "==> OK: $(command -v docker_monitor) ($(docker_monitor --version))"
else
    echo "==> Instalado. Abra um novo terminal (ou rode: source $RC) e teste:"
    echo "    docker_monitor --version"
fi
if command -v dm >/dev/null 2>&1; then
    echo "==> OK: atalho 'dm' pronto ($(command -v dm))"
else
    echo "==> Para usar o atalho, abra um novo terminal (ou rode: source $RC) e teste:"
    echo "    dm --version"
fi
