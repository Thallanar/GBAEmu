# Bugs conhecidos

> Lista de bugs identificados que **precisam ser atacados**, mas ainda **não**
> foram corrigidos. Ao pegar um item, mova-o para "Em progresso" ou remova-o
> quando resolvido (referenciando o PR que fechou). Ao descobrir um bug novo,
> adicione aqui.
>
> _Última atualização: 2026-07-17_

## Abertos

### 1. Rotação de tela com dimensionador ligado → tela preta
- **Onde:** Android, ao rotacionar o aparelho.
- **Sintoma:** se algum dimensionador (upscaler/shader) estiver ligado, a tela
  fica preta após a rotação. É preciso desligar e ligar o dimensionador de novo
  para a imagem voltar.
- **Suspeita:** o contexto/recursos de GL (FBOs, texturas, pipeline multipass)
  não são recriados no ciclo de recriação da `Surface`/`Activity` na mudança de
  orientação; o estado do dimensionador precisa ser reinicializado.

### 2. Shiny Hunter em modo selvagens abre a bag em vez de sair
- **Onde:** modo Shiny Hunter, encontros de selvagens.
- **Sintoma:** ao tentar sair do encontro, a automação acidentalmente abre a
  bag em vez de fugir/sair.
- **Suspeita:** sequência/timing de inputs errada — o botão enviado (ou o frame
  em que é enviado) cai no item de menu da bag em vez da opção de saída.

### 3. Shader BLUR deixa a imagem de cabeça pra baixo
- **Onde:** shader BLUR (motor multipass).
- **Sintoma:** com o BLUR ativo o emulador renderiza a imagem invertida
  verticalmente (de cabeça pra baixo).
- **Suspeita:** flip de coordenadas de textura (origem Y) entre os passes de
  FBO — o passe do blur não compensa a inversão de eixo Y do ping-pong de FBO.

### 4. Sprite do Shiny Hunter fica "?" no Android (modo selvagem)
- **Onde:** Android, Shiny Hunter em modo selvagens.
- **Sintoma:** o sprite do Pokémon exibido aparece como uma interrogação (`?`),
  não capturando/identificando qual Pokémon está na tela do encontro.
- **Suspeita:** a leitura da espécie do encontro (ou o carregamento do sprite
  correspondente) falha só no Android — espécie não resolvida, asset de sprite
  ausente no pacote Android, ou timing de leitura antes do dado estar pronto.

## Em progresso

_(vazio)_
