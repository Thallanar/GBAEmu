# Vídeos do "❓ Como usar" do Shiny Hunter

Exemplos gravados do Hunter em uso, mostrados no modal de ajuda do desktop e do
Android. O texto de cada modo e o mapeamento alvo → vídeo ficam em
`crates/shiny/src/help.rs` (`HelpVideo`).

| Vídeo (`<stem>`) | Mostra |
|---|---|
| `starters_rse` | Iniciais de Hoenn (Ruby/Sapphire/Emerald), bolsa do Birch na Rota 101 |
| `starters_frlg` | Iniciais de Kanto (FireRed/LeafGreen), Poké Balls na mesa do Oak |
| `wild` | Selvagens na grama |

Ainda **sem vídeo**: presentes (Beldum, iniciais de Johto, Lapras, Eevee,
Hitmonlee/Hitmonchan), fósseis, lendários estáticos, Groudon/Kyogre e a pesca.

## Formatos

- `desktop/<stem>.h264`: H.264 **Annex B** cru, embutido no binário e decodificado
  com o `openh264` (só aceita o perfil *Constrained Baseline*).
- `android/hunter_help/<stem>.mp4`: o mesmo bitstream em MP4, empacotado nos assets
  do APK (`sourceSets` em `android/app/build.gradle.kts`) e tocado num `VideoView`.

O player do desktop assume **15 fps** (`FPS` em `crates/desktop/src/help_video.rs`).

## Adicionar/atualizar um vídeo

Use o ffmpeg **do sistema** (`/usr/bin/ffmpeg`, com libx264). O ffmpeg do Anaconda
não tem o libx264.

```sh
SRC=gravacao.mp4; STEM=starters_rse
/usr/bin/ffmpeg -i "$SRC" -an \
  -vf "fps=15,scale=640:-2:flags=lanczos,format=yuv420p" \
  -c:v libx264 -profile:v baseline -level 3.1 -preset slow -crf 30 -g 150 \
  -movflags +faststart "android/hunter_help/$STEM.mp4"
/usr/bin/ffmpeg -i "android/hunter_help/$STEM.mp4" -c copy \
  -bsf:v h264_mp4toannexb -f h264 "desktop/$STEM.h264"
```

Um vídeo novo também precisa de uma variante em `HelpVideo` (`help.rs`), do
`include_bytes!` em `help_video.rs` e do mapeamento em `help_for`.
