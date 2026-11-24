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

    fun languageTag(): String
    fun setLanguage(tag: String)

    fun versionName(): String
    fun openUrl(url: String)

    /** Configurações fechou; [toMenu] = voltou pela seta (reabre o menu). */
    fun onSettingsClosed(toMenu: Boolean)
}
