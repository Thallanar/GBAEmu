#!/usr/bin/env bash
# Baixa as ROMs de teste públicas do jsmolka/gba-tests.
# São livres pra distribuir (autor publica para uso por desenvolvedores de emulador).
set -euo pipefail

DIR="$(dirname "$0")/../test-roms"
mkdir -p "$DIR"

base="https://github.com/jsmolka/gba-tests/raw/master"
for rom in arm/arm.gba thumb/thumb.gba memory/memory.gba; do
    name="$(basename "$rom")"
    if [ ! -f "$DIR/$name" ]; then
        echo "Baixando $name..."
        curl -sL -o "$DIR/$name" "$base/$rom"
    fi
done

ls -la "$DIR"
