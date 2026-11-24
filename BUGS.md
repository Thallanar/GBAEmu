# Bugs conhecidos

> Lista de bugs identificados que **precisam ser atacados**, mas ainda **não**
> foram corrigidos. Ao pegar um item, mova-o para "Em progresso" ou remova-o
> quando resolvido (referenciando o PR que fechou). Ao descobrir um bug novo,
> adicione aqui.
>
> _Última atualização: 2026-11-24_

## Abertos

_(nenhum)_

## Em progresso

_(vazio)_

## Resolvidos

- **Desktop preso em 48 Hz num monitor de 144 Hz** (pacing) — #79. Anotado
  como "~130 de 144", mas o trace mostrou o app preso em 1 atualização a cada
  3 vsyncs: um vsync perdido fazia o pacer rodar 2 frames na atualização
  seguinte (~10 ms), que perdia também. Agora o monitor rápido roda 1 frame
  por atualização e paga o atraso nas vazias, o período do vsync vem de um
  percentil baixo dos intervalos recentes, e o upscale/envio da textura é
  pulado sem frame novo. Medido: 142–145 atualizações/s.

- **BLUR de cabeça pra baixo** (multipass, desktop e Android) — #78. Os passes
  em FBO usavam o quad da tela (v=0 no topo), mas a linha 0 de um FBO fica
  embaixo: cada passe intermediário invertia a imagem. O BLUR (1 intermediário)
  saía invertido; o ScaleFX (4) saía em pé por sorte, mas o passe 2 lia as
  saídas dos passes 0 e 1 desalinhadas. Agora os passes em FBO usam um quad com
  v invertido.

- **Beldum batizado de `AAAAAAA`** (StaticGift) — #77. Acontecia com o **time
  cheio**: o presente ia pro PC, a caça (que procura no time) nunca o via e o
  A-mash seguia até a tela de apelido. Agora a caça fotografa o time quando o
  save carrega e para com aviso se estiver cheio; e o presente só conta num
  slot que estava vazio.

- **Rotação com upscale/shader ligado → tela preta** (Android) — #75. A rotação
  recria o contexto GL e o `texFactor` antigo sobrevivia: o frame ampliado era
  enviado a uma textura 240×160 e o GL descartava.
- **Shiny Hunter abre a bag em vez de fugir** (selvagem) — #75. A fuga às cegas
  (B ▼ ► A) perdia o ▼ quando o menu ativava no meio, e o ► levava pra BAG. Agora
  o A só sai com o cursor do menu (`gActionSelectionCursor`) em CORRER.
- **Sprite "?" no Shiny Hunter do Android** (selvagem) — #75. O sprite era
  buscado uma vez pela espécie do alvo (0 = qualquer); agora acompanha a espécie
  do último encontro, como no desktop.
