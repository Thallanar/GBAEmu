# Bugs conhecidos

> Lista de bugs identificados que **precisam ser atacados**, mas ainda **não**
> foram corrigidos. Ao pegar um item, mova-o para "Em progresso" ou remova-o
> quando resolvido (referenciando o PR que fechou). Ao descobrir um bug novo,
> adicione aqui.
>
> _Última atualização: 2026-11-23_

## Abertos

### 1. Shader BLUR deixa a imagem de cabeça pra baixo
- **Onde:** shader BLUR (motor multipass).
- **Sintoma:** com o BLUR ativo o emulador renderiza a imagem invertida
  verticalmente (de cabeça pra baixo).
- **Suspeita:** flip de coordenadas de textura (origem Y) entre os passes de
  FBO — o passe do blur não compensa a inversão de eixo Y do ping-pong de FBO.

### 2. Shiny Hunter no Beldum trava na pergunta do apelido
- **Onde:** Shiny Hunter, método StaticGift (Beldum pós-E4).
- **Sintoma:** ao pegar o Beldum, aparecem duas perguntas em sequência: (1) se
  quer pegar o Pokémon — o A repetido do ciclo resolve bem; (2) se quer dar um
  apelido ao Pokémon — aqui o ciclo quebra, porque os A's caem na tela de
  nomeação e acabam batizando o Pokémon como `AAAAAAA`.
- **Suspeita:** a automação não trata a caixa de diálogo de apelido — precisa
  detectar/dispensar a pergunta do apelido (escolher "Não", ou navegar para
  confirmar o nome vazio/padrão) em vez de só marteladar A. Falta um estado no
  fluxo do StaticGift para o prompt de nickname.

### 3. Desktop perde vsyncs (~130 de 144 atualizações/s)
- **Onde:** desktop, monitor de 144 Hz (medido com `AURORAGBA_PACING_TRACE=1`).
- **Sintoma:** a janela atualiza ~130 vezes/s em vez de 144; cada vsync perdido
  vira um intervalo fora do padrão 2/3 entre frames do jogo (judder leve).
- **Suspeita:** alguma atualização passa de ~6,9 ms (emulação + upload da
  textura + UI do egui). Medir o tempo de cada parte antes de mexer.

## Em progresso

_(vazio)_

## Resolvidos

- **Rotação com upscale/shader ligado → tela preta** (Android) — #75. A rotação
  recria o contexto GL e o `texFactor` antigo sobrevivia: o frame ampliado era
  enviado a uma textura 240×160 e o GL descartava.
- **Shiny Hunter abre a bag em vez de fugir** (selvagem) — #75. A fuga às cegas
  (B ▼ ► A) perdia o ▼ quando o menu ativava no meio, e o ► levava pra BAG. Agora
  o A só sai com o cursor do menu (`gActionSelectionCursor`) em CORRER.
- **Sprite "?" no Shiny Hunter do Android** (selvagem) — #75. O sprite era
  buscado uma vez pela espécie do alvo (0 = qualquer); agora acompanha a espécie
  do último encontro, como no desktop.
