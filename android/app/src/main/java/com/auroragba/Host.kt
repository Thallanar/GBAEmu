package com.auroragba

/**
 * O que as telas do app ([MenuSheet], [SettingsScreen]) pedem ao [MainActivity]:
 * textos traduzidos, o estado atual e as ações. Separa a montagem das telas da
 * lógica da emulação, que continua toda no MainActivity.
 */
interface Host {
    /** Texto traduzido de [id], formatado com [args] no idioma do app. */
    fun tr(id: Int, vararg args: Any): String

    /** Plural traduzido de [id] para a quantidade [n] (já formatado com [n]). */
    fun plural(id: Int, n: Int): String

    // ── Menu ─────────────────────────────────────────────────────────────────
    /** Nome do jogo carregado (`null` sem ROM). */
    val gameTitle: String?
    /** O menu pausa o jogo? (não com o link ativo: o parceiro depende do nosso ritmo). */
    val menuPauses: Boolean
    val hunterAvailable: Boolean
    val fastOn: Boolean
    fun setFast(on: Boolean)
    fun onHunter()
    fun onSaveState()
    fun onLoadState()
    fun onLoadRom()
    fun onResetRom()
    fun onScreenshot()
    fun onLink()
    fun onSettings()
    fun onMenuClosed()

    // ── Configurações ────────────────────────────────────────────────────────
    /** Nome da pasta de ROMs (`null` se ainda não escolhida). */
    fun romFolderName(): String?
    fun romCount(): Int
    fun pickRomFolder()

    var soundOn: Boolean
    var volume: Int
    var ffMuted: Boolean
    var fastMultiplier: Int

    fun shaderLabel(): String
    fun upscaleLabel(): String
    fun openShader(onDone: () -> Unit)
    fun openUpscale(onDone: () -> Unit)
    fun openRemap()

    // Controles na tela.
    var padModel: PadModel
    /** Tamanho em % (70–130). */
    var padSize: Int
    /** Opacidade em % (10–60). */
    var padOpacity: Int
    var padHaptics: Boolean
    var padHideWithController: Boolean

    fun languageTag(): String
    fun setLanguage(tag: String)

    fun versionName(): String
    fun openUrl(url: String)

    /** Configurações fechou; [toMenu] = voltou pela seta (reabre o menu). */
    fun onSettingsClosed(toMenu: Boolean)

    // ── Estados ──────────────────────────────────────────────────────────────
    /** Os 8 slots do jogo atual. */
    fun stateSlots(): List<SlotInfo>
    /** Salva no [slot] (0-indexado) no próximo frame, com miniatura. */
    fun saveSlot(slot: Int)
    fun loadSlot(slot: Int)
    /** Data de um slot no idioma do app ("Hoje, 14:32", "12/11, 09:45"). */
    fun slotDate(millis: Long): String
    /** Tela de estados fechou; [toMenu] = voltou pela seta (reabre o menu). */
    fun onStatesClosed(toMenu: Boolean)
}

/** Um slot de save state: se existe, quando foi salvo e a miniatura (se houver). */
class SlotInfo(val exists: Boolean, val modified: Long, val thumb: java.io.File?)
