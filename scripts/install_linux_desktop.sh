#!/usr/bin/env sh
# Instala o atalho do AuroraGBA no menu/dock do Linux (só pro usuário atual).
#
# No Wayland o compositor ignora o ícone que a janela informa e usa o do arquivo
# `.desktop` cujo nome bate com o `app_id` da janela ("auroragba"). Sem isso a
# dock mostra um ícone genérico.
#
# Uso: scripts/install_linux_desktop.sh [caminho/do/binário]
#      (padrão: target/release/auroragba deste repositório)
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=$(realpath "${1:-$ROOT/target/release/auroragba}")
[ -x "$BIN" ] || { echo "binário não encontrado: $BIN (rode cargo build --release)"; exit 1; }

DATA=${XDG_DATA_HOME:-$HOME/.local/share}
install -Dm644 "$ROOT/crates/desktop/assets/auroragba.png" \
  "$DATA/icons/hicolor/256x256/apps/auroragba.png"
sed "s|^Exec=auroragba|Exec=\"$BIN\"|" "$ROOT/crates/desktop/assets/linux/auroragba.desktop" \
  > "$DATA/applications/auroragba.desktop"

command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$DATA/icons/hicolor" 2>/dev/null || true
command -v update-desktop-database >/dev/null && update-desktop-database -q "$DATA/applications" 2>/dev/null || true
echo "instalado: $DATA/applications/auroragba.desktop → $BIN"
