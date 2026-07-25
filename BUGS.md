# Bugs conhecidos

> Lista de bugs identificados que **precisam ser atacados**, mas ainda **não**
> foram corrigidos. Ao pegar um item, mova-o para "Em progresso" ou remova-o
> quando resolvido (referenciando o PR que fechou). Ao descobrir um bug novo,
> adicione aqui.
>
> _Última atualização: 2026-07-24_

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

### 5. Shiny Hunter no Beldum trava na pergunta do apelido
- **Onde:** Shiny Hunter, método StaticGift (Beldum pós-E4).
- **Sintoma:** ao pegar o Beldum, aparecem duas perguntas em sequência: (1) se
  quer pegar o Pokémon — o A repetido do ciclo resolve bem; (2) se quer dar um
  apelido ao Pokémon — aqui o ciclo quebra, porque os A's caem na tela de
  nomeação e acabam batizando o Pokémon como `AAAAAAA`.
- **Suspeita:** a automação não trata a caixa de diálogo de apelido — precisa
  detectar/dispensar a pergunta do apelido (escolher "Não", ou navegar para
  confirmar o nome vazio/padrão) em vez de só marteladar A. Falta um estado no
  fluxo do StaticGift para o prompt de nickname.

## Em progresso

_(vazio)_
