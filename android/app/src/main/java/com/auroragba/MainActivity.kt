package com.auroragba

import android.app.Activity
import android.app.AlertDialog
import android.content.ContentValues
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Color
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.net.Uri
import android.opengl.GLES20
import android.opengl.GLSurfaceView
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.util.Log
import android.view.Gravity
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.Surface
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import android.widget.VideoView
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import javax.microedition.khronos.egl.EGLConfig
import javax.microedition.khronos.opengles.GL10

/**
 * Tela única do emulador: um [GLSurfaceView] mostra o framebuffer (o core entrega
 * 240×160 RGBA, enviado como textura e escalado pela GPU) e um [ControlsView] por
 * cima trata o toque. O loop de emulação roda **na thread de render do GL**
 * (`onDrawFrame`): **todo** acesso ao ponteiro nativo do [Gba][NativeBridge]
 * acontece nessa thread. A UI passa input (máscara de botões), a ROM escolhida e
 * os comandos do menu por estruturas atômicas.
 *
 * Antes o render era por software (`SurfaceView.lockCanvas` + `drawBitmap`
 * escalando na CPU 60×/s); a GPU fazia nada e o aparelho esquentava. Agora a
 * escala 240×160→tela sai de graça na GPU.
 */
class MainActivity : Activity(), Host {

    private companion object {
        const val W = 240
        const val H = 160
        const val OPEN_ROM = 1
        // Escolha da pasta de ROMs (SAF tree, persistida).
        const val OPEN_TREE = 2
        // Importação de um `.frag` de shader customizado.
        const val OPEN_SHADER = 3
        const val PREFS = "auroragba"
        const val KEY_ROM_FOLDER = "rom_folder_uri"
        const val KEY_PAD_MAP = "pad_map"
        const val KEY_LINK_ADDR = "link_addr"
        const val KEY_SHADER = "shader"
        // Fonte + nome do `.frag` importado (persistidos p/ sobreviver ao restart).
        const val KEY_SHADER_SRC = "shader_src"
        const val KEY_SHADER_NAME = "shader_name"
        const val KEY_UPSCALE = "upscale"
        // Áudio e velocidade (Configurações).
        const val KEY_SOUND = "sound_on"
        const val KEY_VOLUME = "volume"
        const val KEY_FF_MUTED = "ff_muted"
        const val KEY_FF_MULT = "ff_multiplier"
        // Controles na tela (Configurações › Interface).
        const val KEY_PAD_MODEL = "pad_model"
        const val KEY_PAD_SIZE = "pad_size"
        const val KEY_PAD_OPACITY = "pad_opacity"
        const val KEY_PAD_HAPTICS = "pad_haptics"
        const val KEY_PAD_HIDE = "pad_hide_with_controller"
        const val KEY_PAD_LAYOUT_P = "pad_layout_portrait"
        const val KEY_PAD_LAYOUT_L = "pad_layout_landscape"

        // Filtros de upscale. `Triple(key, fator, rótulo)`: `key` casa com
        // `Upscale::key` do nativo (carrega algoritmo + fator); `fator` dimensiona
        // a textura no Kotlin; `rótulo` é a UI. HQ2x e xBRZ2x têm o mesmo fator mas
        // chaves distintas — por isso a seleção é pela chave, não pelo fator.
        // `MAX_UPSCALE` dimensiona o ByteBuffer/textura pro fator máximo.
        val UPSCALES = arrayOf(
            Triple("off", 1, ""),
            Triple("hq2x", 2, "HQ2x"),
            Triple("hq3x", 3, "HQ3x"),
            Triple("hq4x", 4, "HQ4x"),
            Triple("xbrz2x", 2, "xBRZ2x"),
            Triple("xbrz3x", 3, "xBRZ3x"),
            Triple("xbrz4x", 4, "xBRZ4x"),
        )
        const val MAX_UPSCALE = 4

        // Efeitos de shader (formato próprio single-pass; fontes em assets/shaders/,
        // espelhadas de assets/shaders/ na raiz do repo). `key` casa com o arquivo
        // `<key>.frag`; `label` é o rótulo na UI.
        val SHADERS = arrayOf(
            "none" to "",
            "scanlines" to "Scanlines",
            "lcd-grid" to "LCD grid",
            "lcd3x" to "LCD3x",
            "crt" to "CRT",
        )

        // Efeitos multipass: `key` casa com o manifesto `<key>.mpass` nos assets
        // (encadeia vários `.frag`; ver assets/shaders/README.md). Aparecem no mesmo
        // seletor de shader e persistem pela mesma chave (`currentShader`).
        val MULTIPASS = arrayOf("blur" to "Blur", "scalefx" to "ScaleFX")
        // Multipass que exigem framebuffer meio-float (RGBA16F): só aparecem/compilam
        // se o device tiver EXT_color_buffer_half_float + OES_texture_half_float.
        val MULTIPASS_NEEDS_FLOAT = setOf("scalefx")
        // Porta TCP padrão do link (igual ao desktop: `link::DEFAULT_PORT`).
        const val LINK_PORT = 7777

        // Botões GBA remapeáveis do controle físico (D-pad fica fixo). Os três
        // arrays andam juntos pelo índice; o pad_map persiste os keycodes nessa ordem.
        val PAD_BTN_BITS = intArrayOf(Btn.A, Btn.B, Btn.START, Btn.SELECT, Btn.L, Btn.R)
        val PAD_BTN_NAMES = arrayOf("A", "B", "Start", "Select", "L", "R")
        val PAD_DEFAULT_KEYCODES = intArrayOf(
            KeyEvent.KEYCODE_BUTTON_A, KeyEvent.KEYCODE_BUTTON_B,
            KeyEvent.KEYCODE_BUTTON_START, KeyEvent.KEYCODE_BUTTON_SELECT,
            KeyEvent.KEYCODE_BUTTON_L1, KeyEvent.KEYCODE_BUTTON_R1,
        )
        const val TAG = "AuroraGBA"
        // Tipo de pixel meio-float do OES_texture_half_float (ausente na classe GLES20).
        const val GL_HALF_FLOAT_OES = 0x8D61
        // Taxa de quadros real do GBA (≈ 16,78 MHz / 280896 ciclos). Anunciada ao
        // compositor pra ele casar a cadência num painel de refresh alto.
        const val GBA_FPS = 59.7275f
        // Grava o .sav periodicamente quando há alteração (~5 s a 60 fps).
        const val FLUSH_EVERY_FRAMES = 300
        // Frames de emulação por draw durante a caça (acelera o hunt; ~8× a 60Hz).
        const val HUNT_BATCH = 8
        // Orçamento de CPU por draw no fast-forward (emulação + upscale). Fica
        // abaixo dos 16,7 ms do vsync pra sobrar pro upload/draw GL e o swap.
        const val FF_BUDGET_NANOS = 13_000_000L
        // Slots de save state em disco (`<code>.ss1`..`.ss8`), igual ao desktop.
        const val SAVE_SLOTS = 8
        val MATCH = FrameLayout.LayoutParams.MATCH_PARENT
    }

    private lateinit var glView: GLSurfaceView

    // Máscara final lida pela thread GL = união das três fontes de input. Touch e
    // controle físico convivem (OR): `touchMask` vem do ControlsView; `padButtons`
    // dos botões/D-pad do controle (key events); `padAxis` das direções vindas dos
    // eixos analógicos/hat (motion events). Ver dispatchKeyEvent/onGenericMotionEvent.
    private val buttonMask = AtomicInteger(0)
    @Volatile private var touchMask = 0
    @Volatile private var padButtons = 0
    @Volatile private var padAxis = 0

    // Keycode do controle pra cada botão GBA remapeável (índice = PAD_BTN_BITS).
    // Carregado das prefs; editável pelo ☰ menu — controles layout Nintendo
    // reportam A/B no lugar trocado em relação ao GBA. Só mexido na thread de UI.
    private var padKeycodes = PAD_DEFAULT_KEYCODES.copyOf()

    // Multiplicador do fast-forward (1 = velocidade normal). Escrito pela UI
    // (☰ menu), lido pela thread GL a cada draw. Não persiste: o app sempre
    // abre em velocidade normal.
    @Volatile private var ffSpeed = 1

    // Áudio (Configurações): som ligado, volume 0–100 e se a velocidade acelerada
    // fica muda. Viram o ganho do AudioTrack em [applyAudioGain]; o `write`
    // bloqueante segue igual, então o pacing não muda.
    private var soundOnPref = true
    private var volumePref = 100
    private var ffMutedPref = false
    // Multiplicador da velocidade acelerada (2/4/8×), ligada/desligada pelo menu.
    private var fastMultiplierPref = 2

    // Menu/Configurações abertos pausam a emulação (lido pela thread GL). Com o
    // link ativo o jogo segue rodando: o parceiro depende do nosso ritmo.
    @Volatile private var menuPaused = false
    private var settings: SettingsScreen? = null
    private var layoutEditor: LayoutEditor? = null
    private var states: StatesScreen? = null
    // Nome da ROM carregada (sem `.gba`), pro cabeçalho do menu.
    private var currentRomName: String? = null

    // Controles na tela: esconder quando há controle físico conectado (o botão
    // Menu fica). O listener acompanha conectar/desconectar.
    private var padHidePref = true
    private val inputDeviceListener = object : android.hardware.input.InputManager.InputDeviceListener {
        override fun onInputDeviceAdded(deviceId: Int) = updatePadHidden()
        override fun onInputDeviceRemoved(deviceId: Int) = updatePadHidden()
        override fun onInputDeviceChanged(deviceId: Int) = updatePadHidden()
    }

    // Efeito de shader atual (key de SHADERS, ou "custom"). Escrito pela UI, lido
    // pela thread GL ao (re)compilar o programa. Persiste nas prefs.
    @Volatile private var currentShader = "none"

    // Suporte a framebuffer meio-float (RGBA16F), sondado na criação da surface GL.
    // Gate dos multipass que precisam guardar dados sem perder precisão (ScaleFX).
    @Volatile private var halfFloatSupported = false

    // Fonte do shader importado de arquivo (corpo `effect(...)`, mesmo contrato dos
    // embutidos) e nome do arquivo pra rótulo na UI. `null` = nenhum importado.
    @Volatile private var customShaderSrc: String? = null
    private var customShaderName = ""

    // Chave do filtro de upscale atual ("off"/"hq2x"/"xbrz2x"/…). Estado
    // primário (carrega algoritmo + fator); é o que vai pro nativo e persiste.
    @Volatile private var currentUpscale = "off"
    // Fator geométrico (1/2/3/4) derivado da chave, lido pela thread GL pra
    // dimensionar a textura. Mantido em sincronia por `applyUpscale`.
    @Volatile private var upscaleFactor = 1
    private lateinit var renderer: GbaRenderer

    // Bytes da última ROM carregada, pro 🔄 Resetar (re-entrega pro pendingRom).
    // Só tocado na thread de UI.
    private var lastRom: ByteArray? = null
    private val pendingRom = AtomicReference<ByteArray?>(null)

    // Pasta de ROMs (SAF tree). Escolhida uma vez e persistida nas prefs; o app
    // lista as ROMs dela (biblioteca) e não precisa mais re-escolher o arquivo
    // a cada abertura. `null` = ainda não configurada.
    private var romFolderUri: Uri? = null

    // Comandos do menu, executados na thread GL (acesso ao ponteiro): salvar
    // estado no slot pedido (-1 = nenhum) e carregar estado (bytes lidos do
    // arquivo na UI; o slot já foi resolvido lá).
    private val pendingSaveState = AtomicInteger(-1)
    private val pendingLoadState = AtomicReference<ByteArray?>(null)

    // Captura de tela: a thread GL copia o framebuffer atual quando setado; o
    // encode PNG + gravação na galeria roda numa thread à parte (não trava o draw).
    private val pendingScreenshot = AtomicBoolean(false)

    // ── Link cable ──────────────────────────────────────────────────────────
    // Comandos do menu de Link (a thread GL processa, pois mexem no ponteiro):
    // hospedar na porta (-1 = nenhum), conectar no endereço, cancelar, desconectar.
    private val pendingLinkHost = AtomicInteger(-1)
    private val pendingLinkJoin = AtomicReference<String?>(null)
    private val pendingLinkCancel = AtomicBoolean(false)
    private val pendingLinkDisconnect = AtomicBoolean(false)
    // Estado publicado pela thread GL: 0 ocioso / 1 conectando / 2 conectado, e o
    // papel (0 host / 1 convidado / -1). A UI lê pra montar o menu de Link.
    private val linkStatePub = AtomicInteger(0)
    private val linkRolePub = AtomicInteger(-1)
    // Último estado visto na thread GL, pra avisar (toast) só nas transições.
    @Volatile private var lastLinkState = 0
    // Último endereço usado no "Conectar", persistido pra pré-preencher.
    private var lastLinkAddr = ""
    // Trava o Wi-Fi em baixa latência enquanto o link está ativo: sem isso o
    // rádio dorme entre pacotes e a ida-e-volta do lockstep ganha ~100-200 ms,
    // derrubando o fps. Segurada enquanto conectando/conectado.
    private var wifiLock: android.net.wifi.WifiManager.WifiLock? = null

    // Game code do jogo atual (chave dos arquivos de save). Escrito na thread GL
    // ao carregar a ROM, lido na UI pra montar os caminhos.
    @Volatile private var currentGameCode: String? = null

    // Cache da URI do espelho do `.sav` (em `<pastaDeROMs>/saves/<code>.sav`, SAF).
    // Resolver/criar a subpasta + documento custa I/O via ContentResolver; fazemos
    // uma vez por jogo. Invalidado ao trocar a pasta de ROMs. Acessado na thread GL.
    private var savMirrorUri: Uri? = null
    private var savMirrorCode: String? = null

    // Ponteiro nativo: criado pela thread GL, liberado em onDestroy (vive entre
    // recriações do contexto — pause/resume não reseta o jogo).
    @Volatile private var handle = 0L
    @Volatile private var romLoaded = false

    // Áudio: 32768 Hz estéreo (taxa nativa do APU, sem reamostragem). Criado na
    // thread GL; o `write` bloqueante ancora a emulação ao tempo real (pacing
    // pelo consumo de áudio, mesmo modelo do desktop). Pausado/retomado no
    // lifecycle, liberado no fim.
    @Volatile private var audio: AudioTrack? = null

    // ── Shiny Hunter ──────────────────────────────────────────────────────────
    // Comandos do menu (thread GL processa): iniciar caça num alvo (-1 = nenhum)
    // e parar.
    private val pendingHuntStart = AtomicInteger(-1)
    private val pendingHuntStop = AtomicBoolean(false)
    @Volatile private var hunting = false

    // Info do perfil publicada pela thread GL ao carregar a ROM; lida na UI.
    @Volatile private var huntSupported = false
    @Volatile private var huntGameName = ""
    @Volatile private var huntTargets: Array<String> = emptyArray()
    // Grupo de cada alvo (HuntTarget.LEGENDARY/STARTER/GIFT/WILD), mesmo índice.
    @Volatile private var huntKinds: IntArray = IntArray(0)
    // Ajuda "Como usar" de cada alvo (mesmo índice de huntTargets), lida junto
    // com os alvos na thread GL pra a UI não tocar no emulador.
    @Volatile private var huntHelps: Array<HuntHelp> = emptyArray()

    /** Ajuda de um alvo: título do modo, passo a passo e vídeo (nome-base ou ""). */
    private class HuntHelp(val title: String, val body: String, val video: String)

    // Stats da caça publicadas pela thread GL a cada step; lidas pelo updater da UI.
    @Volatile private var statAttempts = 0L
    @Volatile private var statSpecies = 0
    @Volatile private var statBestSV = 0xFFFF
    @Volatile private var currentTargetName = ""
    @Volatile private var currentTargetIndex = 0

    // Vistas da caça (painel do retrato, sobreposição da paisagem, aviso de shiny).
    private lateinit var huntViews: HuntViews
    // Tela de escolha de alvo / aviso de time cheio abertos (mantêm a pausa).
    private var hunterOpen = false
    private val uiHandler = Handler(Looper.getMainLooper())

    // Sprites do alvo (ARGB8888 64×64), decodificados na thread GL ao iniciar a
    // caça e lidos pela UI ao montar o painel do retrato.
    @Volatile private var spriteNormal: IntArray? = null
    @Volatile private var spriteShiny: IntArray? = null
    // Alvo da caça em andamento e espécie cujos sprites estão decodificados
    // (thread GL). No selvagem "qualquer" a espécie muda a cada encontro.
    private var huntTarget = -1
    private var spriteSpecies = -1

    // Views persistentes (reaproveitadas entre paisagem/retrato em [applyLayout]).
    private lateinit var controls: ControlsView
    private var portrait = false

    // Textos no idioma escolhido (ver [L10n]); refeito ao trocar o idioma.
    private lateinit var l10n: android.content.Context

    override fun tr(id: Int, vararg args: Any): String = l10n.getString(id, *args)

    override fun plural(id: Int, n: Int): String = l10n.resources.getQuantityString(id, n, n)

    /** Rótulo de shader/upscale: `""` é o "nenhum", traduzido na hora. */
    private fun noneOr(label: String) = label.ifEmpty { tr(R.string.none) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        l10n = L10n.context(this, L10n.load(this))
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        // Pacing (Etapa 2, raso — zero toque no áudio/loop): avisa o sistema que
        // produzimos ~59,73 fps de cadência fixa. Num painel de 120 Hz o compositor
        // passa a casar isso (cai pra 60 Hz seamless ou paceia uniforme), matando
        // o batimento não-inteiro (120/59,73 = 2,009) que vira a travadinha cíclica
        // no scroll. `preferredRefreshRate` na janela é o fallback amplo (API 21+);
        // o sinal preciso é o `Surface.setFrameRate` no onSurfaceChanged (API 30+).
        window.attributes = window.attributes.apply { preferredRefreshRate = GBA_FPS }
        logRefresh("onCreate")
        loadPadMap()
        loadLinkAddr()
        loadShader()
        loadUpscale()
        loadAudioPrefs()

        glView = GLSurfaceView(this).apply {
            setEGLContextClientVersion(2)
            preserveEGLContextOnPause = true
            renderer = GbaRenderer()
            setRenderer(renderer)
            renderMode = GLSurfaceView.RENDERMODE_CONTINUOUSLY
        }

        controls = ControlsView(this).apply {
            onMask = { touchMask = it; syncButtons() }
            onMenu = { showMenu() }
        }
        loadPadPrefs()

        huntViews = HuntViews(this, this).apply {
            onStop = { pendingHuntStop.set(true) }
            onSaveState = { openStates(load = false) }
        }

        // Monta o layout conforme a orientação atual (rebuild em rotação).
        applyLayout()

        // Recupera a pasta de ROMs persistida (se a permissão ainda valer).
        romFolderUri = loadRomFolder()

        if (savedInstanceState == null) chooseRom()
    }

    // ── Controle físico (gamepad) ────────────────────────────────────────────
    // Botões e D-pad digital chegam como key events; sticks analógicos e hat como
    // motion events. Convertemos pra `padButtons`/`padAxis` e combinamos com o
    // touch (OR) — os dois convivem. Quando há um diálogo aberto (menu/slots), o
    // foco vai pra ele e estes overrides nem recebem o evento, então a navegação
    // do menu pelo D-pad do controle segue funcionando.

    /** Recompõe a máscara final a partir das fontes de input (touch + controle). */
    private fun syncButtons() {
        buttonMask.set(touchMask or padButtons or padAxis)
    }

    /** Bit do botão GBA pra um keycode (0 se não mapeado). D-pad fixo; resto via remap. */
    private fun gamepadBit(keyCode: Int): Int = when (keyCode) {
        KeyEvent.KEYCODE_DPAD_UP -> Btn.UP
        KeyEvent.KEYCODE_DPAD_DOWN -> Btn.DOWN
        KeyEvent.KEYCODE_DPAD_LEFT -> Btn.LEFT
        KeyEvent.KEYCODE_DPAD_RIGHT -> Btn.RIGHT
        else -> {
            val i = padKeycodes.indexOf(keyCode)
            if (i >= 0) PAD_BTN_BITS[i] else 0
        }
    }

    /** Carrega o mapa do controle das prefs (CSV de keycodes na ordem de PAD_BTN_BITS). */
    private fun loadPadMap() {
        val s = getSharedPreferences(PREFS, MODE_PRIVATE).getString(KEY_PAD_MAP, null) ?: return
        val parts = s.split(",").mapNotNull { it.toIntOrNull() }
        if (parts.size == PAD_BTN_BITS.size) padKeycodes = parts.toIntArray()
    }

    private fun savePadMap() {
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
            .putString(KEY_PAD_MAP, padKeycodes.joinToString(",")).apply()
    }

    private fun loadShader() {
        val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
        customShaderSrc = prefs.getString(KEY_SHADER_SRC, null)
        customShaderName = prefs.getString(KEY_SHADER_NAME, "arquivo") ?: "arquivo"
        val s = prefs.getString(KEY_SHADER, null) ?: return
        when {
            s == "custom" -> if (customShaderSrc != null) currentShader = "custom"
            SHADERS.any { it.first == s } -> currentShader = s
            MULTIPASS.any { it.first == s } -> currentShader = s
        }
    }

    private fun saveShader() {
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
            .putString(KEY_SHADER, currentShader)
            .putString(KEY_SHADER_SRC, customShaderSrc)
            .putString(KEY_SHADER_NAME, customShaderName)
            .apply()
    }

    /** Menu de seleção de shader: troca o efeito, persiste e recompila na thread GL. */
    private fun shaderMenu(onDone: () -> Unit = {}) {
        // Embutidos single-pass + multipass + (se importado) a entrada "Custom".
        val entries = SHADERS.toMutableList()
        // Esconde os multipass que exigem meio-float quando o device não suporta.
        entries.addAll(MULTIPASS.filter { it.first !in MULTIPASS_NEEDS_FLOAT || halfFloatSupported })
        if (customShaderSrc != null) entries.add("custom" to tr(R.string.shader_custom, customShaderName))
        val labels = entries.map { noneOr(it.second) }.toTypedArray()
        val cur = entries.indexOfFirst { it.first == currentShader }.coerceAtLeast(0)
        AlertDialog.Builder(this)
            .setTitle(tr(R.string.shader_title))
            .setSingleChoiceItems(labels, cur) { dialog, which ->
                currentShader = entries[which].first
                saveShader()
                glView.queueEvent { renderer.rebuildProgram() }
                toast(tr(R.string.shader_toast, labels[which]))
                dialog.dismiss()
                onDone()
            }
            .setNeutralButton(tr(R.string.shader_import)) { _, _ -> openShaderPicker() }
            .setNegativeButton(tr(R.string.cancel), null)
            .show()
    }

    /** Seletor de um `.frag` de shader customizado (lido como texto). */
    private fun openShaderPicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(intent, OPEN_SHADER)
    }

    /** Lê o `.frag` importado, persiste a fonte e aplica como shader "custom". */
    private fun loadCustomShader(uri: Uri) {
        val src = try {
            contentResolver.openInputStream(uri)?.use { it.reader().readText() }
        } catch (e: Exception) {
            Log.w(TAG, "falha ao ler .frag: $e"); null
        }
        if (src.isNullOrBlank()) {
            toast(tr(R.string.shader_read_failed)); return
        }
        customShaderSrc = src
        customShaderName = displayName(uri) ?: tr(R.string.shader_file_default)
        currentShader = "custom"
        saveShader()
        glView.queueEvent { renderer.rebuildProgram() }
        toast(tr(R.string.shader_toast, customShaderName))
        settings?.refresh()
    }

    /** Nome de exibição de um documento SAF (coluna _display_name), ou `null`. */
    private fun displayName(uri: Uri): String? = try {
        contentResolver.query(
            uri, arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME), null, null, null,
        )?.use { if (it.moveToFirst()) it.getString(0) else null }
    } catch (e: Exception) {
        null
    }

    /** Fixa a chave atual e deriva o fator geométrico dela (mantém os dois em sync). */
    private fun applyUpscale(key: String) {
        currentUpscale = key
        upscaleFactor = UPSCALES.firstOrNull { it.first == key }?.second ?: 1
    }

    private fun loadUpscale() {
        val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
        // Migração: versões antigas guardavam o fator como Int. Se for o caso,
        // mapeia pro HQx equivalente; caso contrário lê a chave nova.
        val key = try {
            prefs.getString(KEY_UPSCALE, "off") ?: "off"
        } catch (e: ClassCastException) {
            when (prefs.getInt(KEY_UPSCALE, 1)) {
                2 -> "hq2x"
                3 -> "hq3x"
                4 -> "hq4x"
                else -> "off"
            }
        }
        if (UPSCALES.any { it.first == key }) applyUpscale(key)
    }

    private fun saveUpscale() {
        getSharedPreferences(PREFS, MODE_PRIVATE).edit().putString(KEY_UPSCALE, currentUpscale).apply()
    }

    /** Menu do filtro de upscale: troca o algoritmo, persiste e avisa o nativo. */
    private fun upscaleMenu(onDone: () -> Unit = {}) {
        val labels = UPSCALES.map { noneOr(it.third) }.toTypedArray()
        val cur = UPSCALES.indexOfFirst { it.first == currentUpscale }.coerceAtLeast(0)
        AlertDialog.Builder(this)
            .setTitle(tr(R.string.upscale_title))
            .setSingleChoiceItems(labels, cur) { dialog, which ->
                applyUpscale(UPSCALES[which].first)
                saveUpscale()
                glView.queueEvent { NativeBridge.setUpscale(handle, currentUpscale) }
                toast(tr(R.string.upscale_toast, labels[which]))
                dialog.dismiss()
                onDone()
            }
            .setNegativeButton(tr(R.string.cancel), null)
            .show()
    }

    /** Nome curto de um keycode pra UI: "BUTTON_X" (sem o prefixo KEYCODE_). */
    private fun keycodeName(code: Int) = KeyEvent.keyCodeToString(code).removePrefix("KEYCODE_")

    /**
     * Lista os botões GBA com o binding atual; tocar um abre a captura. A última
     * entrada restaura o padrão (A→A, B→B, L1, R1).
     */
    private fun remapMenu() {
        val items = Array(PAD_BTN_BITS.size) { "${PAD_BTN_NAMES[it]} — ${keycodeName(padKeycodes[it])}" } +
            tr(R.string.remap_restore)
        AlertDialog.Builder(this)
            .setTitle(tr(R.string.remap_title))
            .setItems(items) { _, which ->
                if (which < PAD_BTN_BITS.size) {
                    captureBinding(which)
                } else {
                    padKeycodes = PAD_DEFAULT_KEYCODES.copyOf()
                    savePadMap()
                    toast(tr(R.string.remap_restored))
                }
            }
            .show()
    }

    /**
     * Captura o próximo botão do controle pro botão GBA [idx]. O diálogo fica com
     * o foco, então o botão capturado não vaza pro jogo (mesma garantia do
     * `configuring` do desktop). Voltar cancela. Se o keycode já estava em outro
     * botão GBA, os dois trocam de lugar (swap) — nunca há keycode duplicado.
     */
    private fun captureBinding(idx: Int) {
        val dialog = AlertDialog.Builder(this)
            .setTitle(tr(R.string.remap_button_title, PAD_BTN_NAMES[idx]))
            .setMessage(tr(R.string.remap_press))
            .create()
        dialog.setOnKeyListener { d, keyCode, event ->
            if (event.action != KeyEvent.ACTION_DOWN) return@setOnKeyListener false
            if (keyCode == KeyEvent.KEYCODE_BACK) {
                d.dismiss()
                return@setOnKeyListener true
            }
            // Só botões de controle servem (D-pad segue fixo; volume etc. ficam com o sistema).
            if (!KeyEvent.isGamepadButton(keyCode)) return@setOnKeyListener false
            val other = padKeycodes.indexOf(keyCode)
            if (other >= 0 && other != idx) padKeycodes[other] = padKeycodes[idx]
            padKeycodes[idx] = keyCode
            savePadMap()
            toast("${PAD_BTN_NAMES[idx]} → ${keycodeName(keyCode)}")
            d.dismiss()
            remapMenu() // volta pra lista atualizada, pra emendar o próximo botão
            true
        }
        dialog.show()
    }

    private fun isDpadKey(keyCode: Int) = keyCode == KeyEvent.KEYCODE_DPAD_UP ||
        keyCode == KeyEvent.KEYCODE_DPAD_DOWN ||
        keyCode == KeyEvent.KEYCODE_DPAD_LEFT ||
        keyCode == KeyEvent.KEYCODE_DPAD_RIGHT

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        val bit = gamepadBit(event.keyCode)
        val fromPad = (event.source and InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD ||
            (event.source and InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK
        // Só consumimos botões nossos vindos de um controle (ou teclas de D-pad). O
        // resto (volume, back, etc.) segue o fluxo normal.
        if (bit != 0 && (fromPad || isDpadKey(event.keyCode))) {
            when (event.action) {
                KeyEvent.ACTION_DOWN -> padButtons = padButtons or bit
                KeyEvent.ACTION_UP -> padButtons = padButtons and bit.inv()
            }
            syncButtons()
            return true
        }
        return super.dispatchKeyEvent(event)
    }

    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        if ((event.source and InputDevice.SOURCE_JOYSTICK) != InputDevice.SOURCE_JOYSTICK ||
            event.action != MotionEvent.ACTION_MOVE
        ) {
            return super.onGenericMotionEvent(event)
        }
        // Direções: hat (D-pad digital de muitos controles) ou stick esquerdo (deadzone).
        val hx = event.getAxisValue(MotionEvent.AXIS_HAT_X)
        val hy = event.getAxisValue(MotionEvent.AXIS_HAT_Y)
        val sx = event.getAxisValue(MotionEvent.AXIS_X)
        val sy = event.getAxisValue(MotionEvent.AXIS_Y)
        val dz = 0.5f
        var m = 0
        if (hx < -0.5f || sx < -dz) m = m or Btn.LEFT
        if (hx > 0.5f || sx > dz) m = m or Btn.RIGHT
        if (hy < -0.5f || sy < -dz) m = m or Btn.UP
        if (hy > 0.5f || sy > dz) m = m or Btn.DOWN
        padAxis = m
        syncButtons()
        return true
    }

    /** Cria o painel completo do hunter (usado no retrato): sprites + stats + parar. */
    /**
     * (Re)monta a hierarquia conforme a orientação. Paisagem: tela cheia + controles
     * por cima (overlay) + status compacto. Retrato: tela no topo (aspecto 240:160)
     * e, abaixo, os controles OU o painel do hunter (durante a caça). As views são
     * reaproveitadas — só mudam de pai —, então o ponteiro nativo e o áudio seguem
     * vivos na rotação (a Activity não recria, graças ao `configChanges`).
     */
    private fun applyLayout() {
        portrait = resources.configuration.orientation == Configuration.ORIENTATION_PORTRAIT
        controls.portrait = portrait
        // Solta as views dos pais antigos antes de re-parentar.
        for (v in listOf<View>(glView, controls, huntViews.panel, huntViews.overlay, huntViews.banner)) {
            (v.parent as? ViewGroup)?.removeView(v)
        }

        val root: View
        if (portrait) {
            // Tela no topo com a altura exata do aspecto 240:160 (sem letterbox).
            val screenW = resources.displayMetrics.widthPixels
            val glH = screenW * H / W
            val col = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
            col.addView(glView, LinearLayout.LayoutParams(MATCH, glH))
            val bottom = FrameLayout(this).apply {
                setBackgroundColor(Color.BLACK)
                layoutParams = LinearLayout.LayoutParams(MATCH, 0, 1f)
            }
            bottom.addView(controls, FrameLayout.LayoutParams(MATCH, MATCH))
            bottom.addView(huntViews.panel, FrameLayout.LayoutParams(MATCH, MATCH))
            col.addView(bottom)
            root = col
        } else {
            val frame = FrameLayout(this)
            frame.addView(glView, FrameLayout.LayoutParams(MATCH, MATCH))
            frame.addView(controls, FrameLayout.LayoutParams(MATCH, MATCH))
            frame.addView(huntViews.overlay, FrameLayout.LayoutParams(MATCH, MATCH))
            root = frame
        }
        // Aviso de shiny encontrado: por cima de tudo, no topo.
        val top = FrameLayout(this)
        top.addView(root, FrameLayout.LayoutParams(MATCH, MATCH))
        val m = (12 * resources.displayMetrics.density).toInt()
        top.addView(huntViews.banner, FrameLayout.LayoutParams(MATCH, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.TOP).apply {
            setMargins(m, m / 2, m, 0)
        })
        setContentView(top)
        refreshHuntViews()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        applyLayout()
        layoutEditor?.relayout()
    }

    /**
     * Mostra/esconde controles e vistas da caça conforme caça × orientação: durante
     * a caça os controles somem (o Hunter dirige o input) e entra o painel
     * (retrato) ou a sobreposição (paisagem), ambos com "Parar caça".
     */
    private fun refreshHuntViews() {
        controls.visibility = if (hunting) View.GONE else View.VISIBLE
        huntViews.panel.visibility = if (portrait && hunting) View.VISIBLE else View.GONE
        huntViews.overlay.visibility = if (!portrait && hunting) View.VISIBLE else View.GONE
        if (hunting) updateHuntViews()
    }

    override fun onResume() {
        super.onResume()
        glView.onResume()
        if (!menuPaused) audio?.play()
        getSystemService(android.hardware.input.InputManager::class.java)
            ?.registerInputDeviceListener(inputDeviceListener, uiHandler)
        updatePadHidden()
    }

    override fun onPause() {
        super.onPause()
        // Garante a gravação do .sav ao sair (a fila do GL é drenada na thread do
        // ponteiro); o flush periódico é a rede de segurança.
        glView.queueEvent { flushBackup() }
        audio?.pause()
        glView.onPause()
        getSystemService(android.hardware.input.InputManager::class.java)
            ?.unregisterInputDeviceListener(inputDeviceListener)
    }

    override fun onDestroy() {
        super.onDestroy()
        releaseWifiLock()
        if (handle != 0L) {
            NativeBridge.destroy(handle)
            handle = 0L
        }
        audio?.release()
        audio = null
    }

    /**
     * Ponto de entrada do "Carregar ROM". Se há uma pasta de ROMs configurada,
     * lista as `.gba` dela num diálogo (a biblioteca) + opções de trocar a pasta
     * ou abrir um arquivo avulso. Sem pasta ainda, abre direto o seletor de pasta.
     */
    private fun chooseRom() {
        val folder = romFolderUri
        if (folder == null) {
            pickRomFolderIntent()
            return
        }
        val roms = listRoms(folder)
        val labels = roms.map { it.first }.toMutableList()
        labels.add(tr(R.string.rom_change_folder))
        labels.add(tr(R.string.rom_open_file))
        val title = if (roms.isEmpty()) tr(R.string.rom_folder_empty) else tr(R.string.rom_list_title, roms.size)
        AlertDialog.Builder(this)
            .setTitle(title)
            .setItems(labels.toTypedArray()) { _, which ->
                when {
                    which < roms.size -> loadRomFromUri(roms[which].second)
                    which == roms.size -> pickRomFolderIntent()
                    else -> openSingleFilePicker()
                }
            }
            .show()
    }

    /** Abre o seletor de PASTA (SAF tree). A permissão é persistida no resultado. */
    private fun pickRomFolderIntent() {
        startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), OPEN_TREE)
    }

    /** Seletor de UM arquivo (fallback, pra ROMs fora da pasta). */
    private fun openSingleFilePicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(intent, OPEN_ROM)
    }

    /** Lista as ROMs `.gba` da pasta (SAF tree), ordenadas por nome. */
    private fun listRoms(folder: Uri): List<Pair<String, Uri>> {
        val out = mutableListOf<Pair<String, Uri>>()
        val treeId = DocumentsContract.getTreeDocumentId(folder)
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(folder, treeId)
        val proj = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
        )
        try {
            contentResolver.query(children, proj, null, null, null)?.use { c ->
                while (c.moveToNext()) {
                    val name = c.getString(1) ?: continue
                    if (!name.lowercase(Locale.ROOT).endsWith(".gba")) continue
                    out.add(name to DocumentsContract.buildDocumentUriUsingTree(folder, c.getString(0)))
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "falha ao listar ROMs da pasta: $e")
        }
        out.sortBy { it.first.lowercase(Locale.ROOT) }
        return out
    }

    /** Lê os bytes de uma ROM (URI do SAF) e entrega pra thread GL carregar. */
    private fun loadRomFromUri(uri: Uri) {
        val bytes = try {
            contentResolver.openInputStream(uri)?.use { it.readBytes() }
        } catch (e: Exception) {
            Log.w(TAG, "falha ao ler ROM: $e"); null
        }
        if (bytes != null) {
            lastRom = bytes // guardado pro 🔄 Resetar (recarrega a mesma ROM)
            currentRomName = displayName(uri)?.removeSuffix(".gba")?.removeSuffix(".GBA")
            pendingRom.set(bytes)
        } else {
            toast(tr(R.string.rom_read_failed))
        }
    }

    /**
     * Reset "de console" (recarrega a mesma ROM do zero), com confirmação. O
     * caminho do [pendingRom] já grava o `.sav` antes de trocar e o recarrega
     * no boot — só o progresso desde o último save in-game se perde.
     */
    private fun resetFromMenu() {
        val rom = lastRom
        if (!romLoaded || rom == null) {
            toast(tr(R.string.reset_need_rom))
            return
        }
        AlertDialog.Builder(this)
            .setTitle(tr(R.string.reset_title))
            .setMessage(tr(R.string.reset_message))
            .setPositiveButton(tr(R.string.reset_title)) { _, _ -> pendingRom.set(rom) }
            .setNegativeButton(tr(R.string.cancel), null)
            .show()
    }

    private fun saveRomFolder(uri: Uri) =
        getSharedPreferences(PREFS, MODE_PRIVATE).edit().putString(KEY_ROM_FOLDER, uri.toString()).apply()

    /** Recupera a pasta persistida, só se a permissão de leitura ainda for válida. */
    private fun loadRomFolder(): Uri? {
        val s = getSharedPreferences(PREFS, MODE_PRIVATE).getString(KEY_ROM_FOLDER, null) ?: return null
        val uri = Uri.parse(s)
        val ok = contentResolver.persistedUriPermissions.any { it.uri == uri && it.isReadPermission }
        return if (ok) uri else null
    }

    /**
     * Acha um documento filho chamado [name] dentro de [parent] (URI de documento
     * no tree), **sem criar**. Retorna a URI do filho, ou `null` se não existe (ou
     * em falha). Usado tanto pelo espelho quanto pela importação.
     */
    private fun findChild(parent: Uri, name: String): Uri? {
        val tree = romFolderUri ?: return null
        val parentId = DocumentsContract.getDocumentId(parent)
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parentId)
        try {
            contentResolver.query(
                children,
                arrayOf(
                    DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                    DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                ),
                null, null, null,
            )?.use { c ->
                while (c.moveToNext()) {
                    if (c.getString(1) == name) {
                        return DocumentsContract.buildDocumentUriUsingTree(tree, c.getString(0))
                    }
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "espelho: falha em findChild($name): $e")
        }
        return null
    }

    /**
     * Acha (ou cria) um documento filho chamado [name] dentro de [parent]. Retorna
     * a URI do filho, ou `null` em falha. Usado pra resolver/criar a subpasta
     * `saves/` e o arquivo `<code>.sav` do espelho.
     */
    private fun findOrCreateChild(parent: Uri, name: String, mime: String): Uri? {
        findChild(parent, name)?.let { return it }
        return try {
            DocumentsContract.createDocument(contentResolver, parent, mime, name)
        } catch (e: Exception) {
            Log.w(TAG, "espelho: falha ao criar $name: $e")
            null
        }
    }

    /**
     * URI da subpasta `saves/` no tree da pasta de ROMs. Com [create], cria se
     * não existir (caminho de escrita); sem, só procura (caminho de importação).
     * `null` se não há pasta de ROMs ou em falha.
     */
    private fun savesDirUri(create: Boolean): Uri? {
        val tree = romFolderUri ?: return null
        val root = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
        return if (create) {
            findOrCreateChild(root, "saves", DocumentsContract.Document.MIME_TYPE_DIR)
        } else {
            findChild(root, "saves")
        }
    }

    /**
     * URI do `.sav` espelhado em `<pastaDeROMs>/saves/<code>.sav` (cacheada por
     * jogo). Resolve/cria a subpasta `saves/` e o arquivo na primeira vez; depois
     * só devolve o cache. `null` se não há pasta de ROMs ou se algo falhou.
     */
    private fun mirrorSavUri(code: String): Uri? {
        if (savMirrorCode == code && savMirrorUri != null) return savMirrorUri
        val savesDir = savesDirUri(create = true) ?: return null
        savMirrorUri = findOrCreateChild(savesDir, "$code.sav", "application/octet-stream")
        savMirrorCode = code
        return savMirrorUri
    }

    /**
     * Espelha o `.sav` na pasta visível `saves/` (SAF), pra backup/portabilidade.
     * **Best-effort**: o save de trabalho fica no `filesDir` (já gravado antes
     * desta chamada); qualquer falha aqui só loga e nunca afeta o jogo. Chamado
     * na thread GL — o I/O caro (criar pasta/arquivo) acontece só na 1ª vez.
     */
    private fun mirrorSav(code: String, bytes: ByteArray) {
        val uri = mirrorSavUri(code) ?: return
        try {
            contentResolver.openOutputStream(uri, "wt")?.use { it.write(bytes) }
            Log.i(TAG, "save .sav espelhado (saves/$code.sav)")
        } catch (e: Exception) {
            Log.w(TAG, "espelho: falha ao gravar saves/$code.sav: $e")
            savMirrorUri = null // força re-resolver na próxima (URI pode ter expirado)
        }
    }

    /**
     * Espelha um save state na pasta visível `saves/` (SAF), como o `.sav`.
     * **Best-effort**: o estado de trabalho já foi gravado no `filesDir`; falha
     * aqui só loga. Sem cache de URI — salvar estado é ação manual e rara.
     */
    private fun mirrorState(code: String, slot: Int, bytes: ByteArray) {
        val name = stateName(code, slot)
        val savesDir = savesDirUri(create = true) ?: return
        val uri = findOrCreateChild(savesDir, name, "application/octet-stream") ?: return
        try {
            contentResolver.openOutputStream(uri, "wt")?.use { it.write(bytes) }
            Log.i(TAG, "estado espelhado (saves/$name)")
        } catch (e: Exception) {
            Log.w(TAG, "espelho: falha ao gravar saves/$name: $e")
        }
    }

    /**
     * Lê o `.sav` do espelho visível (`saves/<code>.sav`) **sem criar nada**, pra
     * importar um save trazido do PC/outro aparelho. Devolve os bytes + o
     * `lastModified` do arquivo (pra decidir quem é mais novo); `null` se não há
     * pasta de ROMs, se o arquivo não existe, ou em falha de leitura. Não mexe no
     * cache do espelho (é só leitura pontual no load).
     */
    private fun importMirrorSav(code: String): Pair<ByteArray, Long>? {
        val savesDir = savesDirUri(create = false) ?: return null
        val file = findChild(savesDir, "$code.sav") ?: return null
        val bytes = readMirror(file, "$code.sav") ?: return null
        return bytes to lastModifiedOf(file)
    }

    /** Lê os bytes de um documento do espelho; `null` em falha (só loga). */
    private fun readMirror(doc: Uri, name: String): ByteArray? = try {
        contentResolver.openInputStream(doc)?.use { it.readBytes() }
    } catch (e: Exception) {
        Log.w(TAG, "import: falha ao ler saves/$name: $e")
        null
    }

    /** `lastModified` (epoch ms) de um documento SAF; 0 se indisponível. */
    private fun lastModifiedOf(doc: Uri): Long = try {
        contentResolver.query(
            doc, arrayOf(DocumentsContract.Document.COLUMN_LAST_MODIFIED), null, null, null,
        )?.use { if (it.moveToFirst() && !it.isNull(0)) it.getLong(0) else 0L } ?: 0L
    } catch (e: Exception) {
        0L
    }

    // ── Menu + saves ─────────────────────────────────────────────────────────

    /**
     * Menu do emulador (botão ☰). Durante a caça vira só "Parar caça"; fora dela,
     * ROM/estados e — se o jogo é suportado — o Shiny Hunter. É o ponto de
     * extensão pras próximas opções (mais slots, configurações, etc.).
     */
    private fun showMenu() {
        if (hunting) {
            AlertDialog.Builder(this)
                .setTitle(tr(R.string.hunter_title))
                .setItems(arrayOf(tr(R.string.hunt_stop_plain))) { _, _ -> pendingHuntStop.set(true) }
                .show()
            return
        }
        pauseForMenu()
        MenuSheet(this, this).show()
    }

    /** Pausa a emulação (e o som) enquanto menu/Configurações estão abertos. */
    private fun pauseForMenu() {
        if (!menuPauses || menuPaused) return
        menuPaused = true
        audio?.pause()
    }

    private fun resumeFromMenu() {
        if (!menuPaused) return
        menuPaused = false
        audio?.play()
    }

    // ── Host: menu ───────────────────────────────────────────────────────────

    override val gameTitle: String?
        get() = if (!romLoaded) null else currentRomName ?: huntGameName.ifEmpty { currentGameCode }
    override val menuPauses: Boolean get() = linkStatePub.get() == 0
    override val hunterAvailable: Boolean get() = romLoaded && huntSupported
    override val fastOn: Boolean get() = ffSpeed > 1

    override fun setFast(on: Boolean) {
        ffSpeed = if (on) fastMultiplierPref else 1
        applyAudioGain()
    }

    override fun onHunter() = startShinyHunter()
    override fun onSaveState() = openStates(load = false)
    override fun onLoadState() = openStates(load = true)
    override fun onLoadRom() = chooseRom()
    override fun onResetRom() = resetFromMenu()
    override fun onScreenshot() = takeScreenshot()
    override fun onLink() = linkMenu()

    override fun onSettings() {
        pauseForMenu()
        settings = SettingsScreen(this, this).also { it.show() }
    }

    override fun onMenuClosed() {
        // Indo pras Configurações/Estados a pausa continua (já pausaram de novo).
        if (settings == null && states == null && !hunterOpen) resumeFromMenu()
    }

    // ── Host: Configurações ──────────────────────────────────────────────────

    override fun romFolderName(): String? {
        val folder = romFolderUri ?: return null
        val doc = DocumentsContract.buildDocumentUriUsingTree(folder, DocumentsContract.getTreeDocumentId(folder))
        return displayName(doc) ?: folder.lastPathSegment
    }

    override fun romCount(): Int = romFolderUri?.let { listRoms(it).size } ?: 0
    override fun pickRomFolder() = pickRomFolderIntent()

    override var soundOn: Boolean
        get() = soundOnPref
        set(v) { soundOnPref = v; saveAudioPrefs(); applyAudioGain() }
    override var volume: Int
        get() = volumePref
        set(v) { volumePref = v.coerceIn(0, 100); saveAudioPrefs(); applyAudioGain() }
    override var ffMuted: Boolean
        get() = ffMutedPref
        set(v) { ffMutedPref = v; saveAudioPrefs(); applyAudioGain() }
    override var fastMultiplier: Int
        get() = fastMultiplierPref
        set(v) {
            fastMultiplierPref = v
            saveAudioPrefs()
            if (ffSpeed > 1) ffSpeed = v // já acelerado: passa a valer na hora
        }

    override fun shaderLabel(): String = if (currentShader == "custom") {
        tr(R.string.shader_custom, customShaderName)
    } else {
        noneOr((SHADERS + MULTIPASS).firstOrNull { it.first == currentShader }?.second ?: "")
    }

    override fun upscaleLabel(): String = noneOr(UPSCALES.firstOrNull { it.first == currentUpscale }?.third ?: "")
    override fun openShader(onDone: () -> Unit) = shaderMenu(onDone)
    override fun openUpscale(onDone: () -> Unit) = upscaleMenu(onDone)
    override fun openRemap() = remapMenu()

    override var padModel: PadModel
        get() = controls.model
        set(v) { controls.model = v; savePadPrefs() }
    override var padSize: Int
        get() = (controls.scale * 100).toInt()
        set(v) { controls.scale = v / 100f; savePadPrefs() }
    override var padOpacity: Int
        get() = Math.round(controls.opacity * 100)
        set(v) { controls.opacity = v / 100f; savePadPrefs() }
    override var padHaptics: Boolean
        get() = controls.haptics
        set(v) { controls.haptics = v; savePadPrefs() }
    override var padHideWithController: Boolean
        get() = padHidePref
        set(v) { padHidePref = v; savePadPrefs(); updatePadHidden() }

    override fun openLayoutEditor(onDone: () -> Unit) {
        layoutEditor = LayoutEditor(this, this, controls, onSave = { p, l ->
            controls.layoutPortrait = p
            controls.layoutLandscape = l
            savePadPrefs()
        }, onClose = {
            layoutEditor = null
            onDone()
        }).also { it.show() }
    }

    // ── Controles na tela ────────────────────────────────────────────────────

    private fun loadPadPrefs() {
        val p = getSharedPreferences(PREFS, MODE_PRIVATE)
        controls.model = PadModel.from(p.getString(KEY_PAD_MODEL, null))
        controls.scale = p.getInt(KEY_PAD_SIZE, 100).coerceIn(70, 130) / 100f
        // Padrão = a opacidade de sempre (alpha 70/255 ≈ 27%).
        controls.opacity = p.getInt(KEY_PAD_OPACITY, 27).coerceIn(10, 60) / 100f
        controls.haptics = p.getBoolean(KEY_PAD_HAPTICS, false)
        padHidePref = p.getBoolean(KEY_PAD_HIDE, true)
        controls.layoutPortrait = PadLayout.decode(p.getString(KEY_PAD_LAYOUT_P, null))
        controls.layoutLandscape = PadLayout.decode(p.getString(KEY_PAD_LAYOUT_L, null))
    }

    private fun savePadPrefs() {
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
            .putString(KEY_PAD_MODEL, controls.model.key)
            .putInt(KEY_PAD_SIZE, Math.round(controls.scale * 100))
            .putInt(KEY_PAD_OPACITY, Math.round(controls.opacity * 100))
            .putBoolean(KEY_PAD_HAPTICS, controls.haptics)
            .putBoolean(KEY_PAD_HIDE, padHidePref)
            .putString(KEY_PAD_LAYOUT_P, controls.layoutPortrait.encode())
            .putString(KEY_PAD_LAYOUT_L, controls.layoutLandscape.encode())
            .apply()
    }

    /**
     * Há um controle físico conectado? Conta dispositivos de gamepad/joystick
     * reais (não virtuais) e, no Android 10+, externos — alguns aparelhos
     * declaram botões internos como "gamepad".
     */
    private fun controllerConnected(): Boolean = InputDevice.getDeviceIds().any { id ->
        val dev = InputDevice.getDevice(id) ?: return@any false
        val src = dev.sources
        val pad = (src and InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD ||
            (src and InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK
        pad && !dev.isVirtual && (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q || dev.isExternal)
    }

    private fun updatePadHidden() {
        controls.padHidden = padHidePref && controllerConnected()
    }

    override fun languageTag(): String = L10n.load(this)

    override fun setLanguage(tag: String) {
        L10n.save(this, tag)
        l10n = L10n.context(this, tag)
        huntViews.relabel()
        if (hunting) updateHuntViews()
        // Nomes dos alvos e a ajuda vêm do Rust, no idioma pedido.
        glView.queueEvent { if (romLoaded) loadHuntInfo() }
    }

    override fun versionName(): String = try {
        packageManager.getPackageInfo(packageName, 0).versionName ?: ""
    } catch (e: Exception) {
        ""
    }

    override fun openUrl(url: String) {
        try {
            startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
        } catch (e: Exception) {
            Log.w(TAG, "sem app pra abrir $url: $e")
        }
    }

    override fun onSettingsClosed(toMenu: Boolean) {
        settings = null
        if (toMenu) {
            MenuSheet(this, this).show() // ainda pausado
        } else {
            resumeFromMenu()
        }
    }

    // ── Host: estados ────────────────────────────────────────────────────────

    override fun stateSlots(): List<SlotInfo> {
        val code = currentGameCode ?: return List(SAVE_SLOTS) { SlotInfo(false, 0L, null) }
        return List(SAVE_SLOTS) { i ->
            val f = stateFile(code, i)
            SlotInfo(f.exists(), f.lastModified(), thumbFile(code, i))
        }
    }

    override fun saveSlot(slot: Int) = pendingSaveState.set(slot)

    override fun loadSlot(slot: Int) {
        val code = currentGameCode ?: return
        val f = stateFile(code, slot)
        if (f.exists()) pendingLoadState.set(f.readBytes())
    }

    override fun slotDate(millis: Long): String {
        val loc = L10n.locale(l10n)
        val time = android.text.format.DateFormat.getBestDateTimePattern(loc, "HHmm")
        val clock = SimpleDateFormat(time, loc).format(Date(millis))
        val day = java.util.Calendar.getInstance()
        val then = java.util.Calendar.getInstance().apply { timeInMillis = millis }
        fun same(c: java.util.Calendar) = c.get(java.util.Calendar.YEAR) == then.get(java.util.Calendar.YEAR) &&
            c.get(java.util.Calendar.DAY_OF_YEAR) == then.get(java.util.Calendar.DAY_OF_YEAR)
        if (same(day)) return tr(R.string.date_today, clock)
        day.add(java.util.Calendar.DAY_OF_YEAR, -1)
        if (same(day)) return tr(R.string.date_yesterday, clock)
        val date = android.text.format.DateFormat.getBestDateTimePattern(loc, "ddMMHHmm")
        return SimpleDateFormat(date, loc).format(Date(millis))
    }

    override val huntGame: String get() = huntGameName

    override fun huntTargetList(): List<HuntTarget> =
        huntTargets.indices.map { HuntTarget(huntTargets[it], huntKinds.getOrElse(it) { HuntTarget.GIFT }) }

    override fun startHunt(i: Int) = beginHunt(i)
    override fun huntHelp(i: Int, onStart: () -> Unit) = showHuntHelp(i, onStart)

    override fun onHunterClosed(toMenu: Boolean) {
        hunterOpen = false
        if (toMenu) MenuSheet(this, this).show() else resumeFromMenu()
    }

    override fun onPartyFullClosed(pickOther: Boolean) {
        hunterOpen = false
        if (pickOther) startShinyHunter() else resumeFromMenu()
    }

    override fun onStatesClosed(toMenu: Boolean) {
        states = null
        if (toMenu) MenuSheet(this, this).show() else resumeFromMenu()
    }

    // ── Áudio ────────────────────────────────────────────────────────────────

    private fun loadAudioPrefs() {
        val p = getSharedPreferences(PREFS, MODE_PRIVATE)
        soundOnPref = p.getBoolean(KEY_SOUND, true)
        volumePref = p.getInt(KEY_VOLUME, 100).coerceIn(0, 100)
        ffMutedPref = p.getBoolean(KEY_FF_MUTED, false)
        fastMultiplierPref = p.getInt(KEY_FF_MULT, 2).takeIf { it in intArrayOf(2, 4, 8) } ?: 2
    }

    private fun saveAudioPrefs() {
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
            .putBoolean(KEY_SOUND, soundOnPref)
            .putInt(KEY_VOLUME, volumePref)
            .putBoolean(KEY_FF_MUTED, ffMutedPref)
            .putInt(KEY_FF_MULT, fastMultiplierPref)
            .apply()
    }

    /**
     * Ganho do AudioTrack conforme as Configurações: mudo se o som está
     * desligado ou se a velocidade acelerada está ligada e configurada como
     * muda. O áudio continua sendo escrito (o `write` bloqueante é o pacing).
     */
    private fun applyAudioGain() {
        val mute = !soundOnPref || (ffMutedPref && ffSpeed > 1)
        audio?.setVolume(if (mute) 0f else volumePref / 100f)
    }

    /**
     * Menu do link cable. O conteúdo depende do estado publicado pela thread GL:
     * ocioso (hospedar/conectar), conectando (cancelar) ou conectado (papel +
     * desconectar). Os comandos viram flags que a thread GL processa.
     */
    private fun linkMenu() {
        when (linkStatePub.get()) {
            2 -> {
                val papel = tr(if (linkRolePub.get() == 0) R.string.link_role_host else R.string.link_role_guest)
                AlertDialog.Builder(this)
                    .setTitle(tr(R.string.link_connected_title))
                    .setMessage(tr(R.string.link_connected_msg, papel))
                    .setPositiveButton(tr(R.string.link_disconnect)) { _, _ -> pendingLinkDisconnect.set(true) }
                    .setNegativeButton(tr(R.string.close), null)
                    .show()
            }
            1 -> {
                AlertDialog.Builder(this)
                    .setTitle(tr(R.string.link_connecting_title))
                    .setMessage(tr(R.string.link_waiting))
                    .setPositiveButton(tr(R.string.cancel)) { _, _ -> pendingLinkCancel.set(true) }
                    .setNegativeButton(tr(R.string.close), null)
                    .show()
            }
            else -> {
                AlertDialog.Builder(this)
                    .setTitle(tr(R.string.link_title))
                    .setItems(arrayOf(tr(R.string.link_host, LINK_PORT), tr(R.string.link_join))) { _, which ->
                        when (which) {
                            0 -> {
                                pendingLinkHost.set(LINK_PORT)
                                toast(tr(R.string.link_host_waiting, LINK_PORT))
                            }
                            1 -> linkJoinDialog()
                        }
                    }
                    .show()
            }
        }
    }

    /** Diálogo de "Conectar": pede o IP do host e dispara o join. */
    private fun linkJoinDialog() {
        val input = EditText(this).apply {
            setText(lastLinkAddr)
            hint = "192.168.0.10"
            setSingleLine()
        }
        AlertDialog.Builder(this)
            .setTitle(tr(R.string.link_join_title))
            .setMessage(tr(R.string.link_join_msg, LINK_PORT))
            .setView(input)
            .setPositiveButton(tr(R.string.link_connect)) { _, _ ->
                val raw = input.text.toString().trim()
                if (raw.isEmpty()) {
                    toast(tr(R.string.link_empty_addr))
                    return@setPositiveButton
                }
                lastLinkAddr = raw
                saveLinkAddr()
                // Sem porta explícita → usa a padrão.
                val addr = if (raw.contains(':')) raw else "$raw:$LINK_PORT"
                pendingLinkJoin.set(addr)
                toast(tr(R.string.link_connecting_to, addr))
            }
            .setNegativeButton(tr(R.string.cancel), null)
            .show()
    }

    private fun loadLinkAddr() {
        lastLinkAddr = getSharedPreferences(PREFS, MODE_PRIVATE).getString(KEY_LINK_ADDR, "") ?: ""
    }

    private fun saveLinkAddr() {
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
            .putString(KEY_LINK_ADDR, lastLinkAddr).apply()
    }

    /**
     * Segura o Wi-Fi em baixa latência (modo de jogo) enquanto há link. É só
     * otimização: qualquer falha aqui é engolida (log) pra nunca derrubar o app
     * nem atrapalhar o link — no pior caso, roda com a latência normal do Wi-Fi.
     */
    private fun acquireWifiLock() {
        try {
            if (wifiLock == null) {
                val wm = applicationContext
                    .getSystemService(WIFI_SERVICE) as android.net.wifi.WifiManager
                // LOW_LATENCY (API 29+) é o ideal pra tempo real; antes, HIGH_PERF.
                val mode = if (Build.VERSION.SDK_INT >= 29) {
                    android.net.wifi.WifiManager.WIFI_MODE_FULL_LOW_LATENCY
                } else {
                    @Suppress("DEPRECATION")
                    android.net.wifi.WifiManager.WIFI_MODE_FULL_HIGH_PERF
                }
                wifiLock = wm.createWifiLock(mode, "auroragba-link")
            }
            wifiLock?.let { if (!it.isHeld) it.acquire() }
            val held = wifiLock?.isHeld == true
            val lowLat = Build.VERSION.SDK_INT >= 29
            toast(
                tr(
                    if (held && lowLat) R.string.wifi_low_latency
                    else if (held) R.string.wifi_high_perf
                    else R.string.wifi_lock_missed
                )
            )
        } catch (e: Throwable) {
            Log.w(TAG, "WifiLock indisponível (segue sem): $e")
            toast(tr(R.string.wifi_lock_failed))
        }
    }

    private fun releaseWifiLock() {
        try {
            wifiLock?.let { if (it.isHeld) it.release() }
        } catch (e: Throwable) {
            Log.w(TAG, "WifiLock release falhou: $e")
        }
    }

    /**
     * Abre a escolha de alvo e dispara a caça. O jogo precisa estar **parado na
     * frente do alvo com o save carregado** (igual ao desktop). A caça em si roda
     * na thread GL via [pendingHuntStart].
     */
    private fun startShinyHunter() {
        if (!romLoaded || !huntSupported) {
            toast(tr(R.string.hunt_unsupported))
            return
        }
        if (huntTargets.isEmpty()) {
            toast(tr(R.string.hunt_no_targets))
            return
        }
        pauseForMenu()
        hunterOpen = true
        HuntTargetScreen(this, this).show()
    }

    /** Pede à thread GL pra começar a caça no alvo `i`. */
    private fun beginHunt(i: Int) {
        currentTargetName = huntTargets[i]
        currentTargetIndex = i
        statAttempts = 0
        statSpecies = 0
        statBestSV = 0xFFFF
        pendingHuntStart.set(i)
    }

    /**
     * Modal de ajuda do alvo `i`: passo a passo e, logo abaixo, o vídeo de exemplo
     * em loop (quando o modo tem vídeo). Daqui já dá pra iniciar a caça.
     */
    private fun showHuntHelp(i: Int, onStart: () -> Unit) {
        val help = huntHelps.getOrNull(i) ?: return
        val dp = resources.displayMetrics.density
        val pad = (16 * dp).toInt()
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad / 2, pad, 0)
        }
        column.addView(TextView(this).apply {
            text = huntTargets[i]
            textSize = 16f
            setTypeface(typeface, android.graphics.Typeface.BOLD)
        })
        column.addView(TextView(this).apply {
            text = help.body
            textSize = 14f
            setPadding(0, pad / 2, 0, pad / 2)
        })
        var video: VideoView? = null
        val file = if (help.video.isNotEmpty()) helpVideoFile(help.video) else null
        if (file != null) {
            video = VideoView(this).apply {
                layoutParams = LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                )
                setVideoPath(file.absolutePath)
                setOnPreparedListener { mp ->
                    mp.isLooping = true
                    start()
                }
            }
            column.addView(video)
        } else {
            column.addView(TextView(this).apply {
                text = if (help.video.isNotEmpty()) {
                    tr(R.string.hunt_video_failed)
                } else {
                    tr(R.string.hunt_no_video)
                }
                alpha = 0.7f
                setPadding(0, 0, 0, pad / 2)
            })
        }
        AlertDialog.Builder(this)
            .setTitle(tr(R.string.hunt_how_to_title, help.title))
            .setView(ScrollView(this).apply { addView(column) })
            .setPositiveButton(tr(R.string.hunt_start)) { _, _ -> onStart() }
            .setNegativeButton(tr(R.string.close), null)
            .setOnDismissListener { video?.stopPlayback() }
            .show()
    }

    /**
     * O VideoView não lê direto dos assets do APK: copia `hunter_help/<nome>.mp4`
     * pro cache (uma vez; recopia se o tamanho mudou numa versão nova do app).
     * `null` se o asset não existe ou a cópia falhar.
     */
    private fun helpVideoFile(stem: String): File? = try {
        val asset = "hunter_help/$stem.mp4"
        val out = File(File(cacheDir, "hunter_help"), "$stem.mp4")
        val size = assets.openFd(asset).use { it.length }
        if (!out.isFile || out.length() != size) {
            out.parentFile?.mkdirs()
            assets.open(asset).use { input -> out.outputStream().use { input.copyTo(it) } }
        }
        out
    } catch (e: Exception) {
        Log.w(TAG, "vídeo de ajuda $stem indisponível", e)
        null
    }

    /** Contagem no formato do idioma do app ("1.284", "1,284"). */
    private fun count(n: Long): String = java.text.NumberFormat.getIntegerInstance(L10n.locale(l10n)).format(n)

    /** Atualiza painel (retrato) e sobreposição (paisagem) com as stats publicadas. */
    private fun updateHuntViews() {
        val best = if (statBestSV == 0xFFFF) "—" else statBestSV.toString()
        val prob = 1.0 - Math.pow(1.0 - 1.0 / 8192.0, statAttempts.toDouble())
        val pct = java.text.NumberFormat.getPercentInstance(L10n.locale(l10n)).apply {
            minimumFractionDigits = 1
            maximumFractionDigits = 1
        }.format(prob)
        // Sem sprite (selvagem antes do 1º encontro) limpa a imagem, em vez de
        // deixar a da caça anterior.
        huntViews.update(
            currentTargetName, count(statAttempts), best, prob, pct,
            spriteNormal?.let { spriteCache(it, true) }, spriteShiny?.let { spriteCache(it, false) },
        )
    }

    // Bitmaps dos sprites, refeitos só quando o array muda (o updater roda a 200 ms).
    private var cachedNormal: Pair<IntArray, Bitmap?>? = null
    private var cachedShiny: Pair<IntArray, Bitmap?>? = null

    private fun spriteCache(argb: IntArray, normal: Boolean): Bitmap? {
        val c = if (normal) cachedNormal else cachedShiny
        if (c != null && c.first === argb) return c.second
        val bmp = spriteBitmap(argb)
        if (normal) cachedNormal = argb to bmp else cachedShiny = argb to bmp
        return bmp
    }

    /** Bitmap nítido (sem suavização) de um sprite ARGB 64×64, escalado 4×. */
    private fun spriteBitmap(argb: IntArray): Bitmap? {
        if (argb.size < 64 * 64) return null
        val src = Bitmap.createBitmap(argb, 64, 64, Bitmap.Config.ARGB_8888)
        return Bitmap.createScaledBitmap(src, 256, 256, false)
    }

    /** Atualiza o status visível (painel no retrato, overlay na paisagem) a 200 ms. */
    private val huntStatsUpdater = object : Runnable {
        override fun run() {
            if (!hunting) return
            updateHuntViews()
            uiHandler.postDelayed(this, 200)
        }
    }

    private fun startHuntUI() {
        refreshHuntViews()
        uiHandler.removeCallbacks(huntStatsUpdater)
        uiHandler.post(huntStatsUpdater)
    }

    private fun stopHuntUI() {
        uiHandler.removeCallbacks(huntStatsUpdater)
        refreshHuntViews()
    }

    /** Arquivo `.sav` (backup do cartucho) do jogo, em `filesDir`. */
    private fun savFile(code: String) = File(filesDir, "$code.sav")

    /** Nome do arquivo do save state do `slot` (0-indexado): `<code>.ss1`..`.ss8`. */
    private fun stateName(code: String, slot: Int) = "$code.ss${slot + 1}"

    /** Arquivo do save state do `slot` (0-indexado: `.ss1`..`.ss8`), em `filesDir`. */
    private fun stateFile(code: String, slot: Int) = File(filesDir, stateName(code, slot))

    /** Miniatura do save state do `slot`: PNG 240×160 ao lado do estado, em `filesDir`. */
    private fun thumbFile(code: String, slot: Int) = File(filesDir, stateName(code, slot) + ".png")

    /** Abre a tela de estados na aba Salvar ou Carregar (pausa o jogo). */
    private fun openStates(load: Boolean) {
        if (!romLoaded || currentGameCode == null) {
            toast(tr(if (load) R.string.state_need_rom_load else R.string.state_need_rom_save))
            return
        }
        pauseForMenu()
        states = StatesScreen(this, this, load).also { it.show() }
    }

    /** Pede pra thread GL capturar o frame atual (o encode/gravação vêm depois). */
    private fun takeScreenshot() {
        if (!romLoaded || currentGameCode == null) {
            toast(tr(R.string.screenshot_need_rom))
            return
        }
        pendingScreenshot.set(true)
    }

    /**
     * Codifica o framebuffer RGBA [px] (240×160) em PNG e grava na galeria:
     * Pictures/AuroraGBA via MediaStore (API 29+, sem permissão) ou, em APIs
     * antigas, na pasta externa do app. Roda numa thread à parte — encode + I/O
     * não podem travar o render.
     */
    private fun saveScreenshot(px: ByteArray, w: Int = W, h: Int = H) {
        val code = currentGameCode ?: "rom"
        Thread {
            try {
                val bmp = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
                bmp.copyPixelsFromBuffer(ByteBuffer.wrap(px))
                val name = "auroragba_${code}_${System.currentTimeMillis() / 1000}.png"
                val where = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    val values = ContentValues().apply {
                        put(MediaStore.Images.Media.DISPLAY_NAME, name)
                        put(MediaStore.Images.Media.MIME_TYPE, "image/png")
                        put(MediaStore.Images.Media.RELATIVE_PATH, "Pictures/AuroraGBA")
                    }
                    val uri = contentResolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
                        ?: throw IllegalStateException("MediaStore recusou a inserção")
                    contentResolver.openOutputStream(uri)!!.use {
                        bmp.compress(Bitmap.CompressFormat.PNG, 100, it)
                    }
                    tr(R.string.screenshot_gallery)
                } else {
                    val dir = getExternalFilesDir(Environment.DIRECTORY_PICTURES)
                    val f = File(dir, name)
                    f.outputStream().use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }
                    f.absolutePath
                }
                bmp.recycle()
                toast(tr(R.string.screenshot_saved, where))
            } catch (e: Exception) {
                Log.e(TAG, "falha ao salvar screenshot: $e")
                toast(tr(R.string.screenshot_failed))
            }
        }.start()
    }

    private fun toast(msg: String) =
        runOnUiThread { Toast.makeText(this, msg, Toast.LENGTH_SHORT).show() }

    /// Loga o refresh real do display (logcat -s AuroraGBA) — pra confirmarmos
    /// o que o aparelho fez com o pedido de cadência. Só observabilidade.
    private fun logRefresh(phase: String) {
        val display = if (Build.VERSION.SDK_INT >= 30) display else windowManager.defaultDisplay
        val hz = display?.refreshRate ?: 0f
        Log.i(TAG, "pacing[$phase]: refresh do display = %.3f Hz (alvo %.4f fps)".format(hz, GBA_FPS))
    }

    @Deprecated("API clássica; suficiente para este app de tela única")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        when (requestCode) {
            OPEN_ROM -> loadRomFromUri(uri)
            OPEN_SHADER -> loadCustomShader(uri)
            OPEN_TREE -> {
                // Persiste o acesso à pasta (sobrevive a reboot/reabertura).
                val flags = data.flags and
                    (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                try {
                    contentResolver.takePersistableUriPermission(uri, flags)
                } catch (e: Exception) {
                    Log.w(TAG, "não consegui persistir a permissão da pasta: $e")
                }
                romFolderUri = uri
                saveRomFolder(uri)
                savMirrorUri = null // a subpasta saves/ vive na pasta antiga; re-resolve
                savMirrorCode = null
                // Pelas Configurações: só atualiza a página. Senão, já abre a
                // lista da pasta recém-escolhida.
                if (settings != null) settings?.refresh() else chooseRom()
            }
        }
    }

    // ── Persistência de saves (chamadas na thread GL) ────────────────────────

    /** Após carregar a ROM: game code, `.sav` existente e info do Shiny Hunter. */
    private fun onRomLoaded() {
        val code = NativeBridge.gameCode(handle)
        currentGameCode = code

        // Publica o suporte do Shiny Hunter pra UI (perfil detectado no loadRom).
        hunting = false
        huntSupported = NativeBridge.huntSupported(handle)
        loadHuntInfo()
        runOnUiThread { stopHuntUI() }

        // Save states valem pra qualquer jogo (não dependem de bateria no cartucho).
        reconcileStates(code)

        if (!NativeBridge.hasSave(handle)) return
        val f = savFile(code)
        // Reconcilia o save de trabalho (filesDir) com o espelho visível (saves/),
        // pra importar um save trazido do PC/outro aparelho. Regra:
        //   - sem save local → importa o espelho (semeia);
        //   - bytes diferentes E espelho mais novo → importa (o save de fora venceu);
        //   - bytes iguais, ou espelho mais antigo/igual → mantém o local.
        // O teste de bytes é o que protege contra falso positivo: o próprio app
        // grava o espelho logo após o filesDir (uns ms mais novo), mas com conteúdo
        // idêntico — então nunca reimportamos o nosso próprio espelho.
        importMirrorSav(code)?.let { (mBytes, mTime) ->
            val local = if (f.exists()) f.readBytes() else null
            val differ = local == null || !mBytes.contentEquals(local)
            val mirrorWins = local == null || mTime > f.lastModified()
            when {
                differ && mirrorWins -> try {
                    f.writeBytes(mBytes)
                    Log.i(TAG, "save importado da pasta (saves/$code.sav, ${mBytes.size} bytes, mtime=$mTime)")
                    toast(tr(if (local == null) R.string.sav_imported else R.string.sav_newer_imported))
                } catch (e: Exception) {
                    Log.w(TAG, "falha ao importar .sav do espelho: $e")
                }
                differ -> Log.i(
                    TAG,
                    "espelho difere mas é mais antigo/igual (mtime=$mTime ≤ ${f.lastModified()}); mantido o save local",
                )
                else -> Unit // iguais: nada a fazer
            }
        }
        if (f.exists()) {
            val ok = NativeBridge.loadBackup(handle, f.readBytes())
            Log.i(TAG, "save .sav carregado=$ok (${f.name})")
        } else {
            Log.i(TAG, "sem .sav prévio para $code")
        }
    }

    /**
     * Reconcilia os save states (`filesDir/<code>.ssN`) com o espelho visível
     * (`saves/`), pra trazer estados do PC/outro aparelho — mesma regra do
     * `.sav`, por slot: sem local → importa; bytes diferentes E espelho mais
     * novo → importa; senão mantém o local. Diferente do `.sav`, checa o mtime
     * ANTES de ler os bytes: estados são grandes (~0,5 MB × 8 slots) e o caso
     * comum é o espelho ser o nosso (idêntico) ou mais antigo — o teste de
     * bytes continua sendo o que impede reimportar o próprio espelho.
     */
    private fun reconcileStates(code: String) {
        val savesDir = savesDirUri(create = false) ?: return
        var imported = 0
        for (slot in 0 until SAVE_SLOTS) {
            val name = stateName(code, slot)
            val doc = findChild(savesDir, name) ?: continue
            val local = stateFile(code, slot)
            if (local.exists() && lastModifiedOf(doc) <= local.lastModified()) continue
            val bytes = readMirror(doc, name) ?: continue
            if (local.exists() && bytes.contentEquals(local.readBytes())) continue
            try {
                local.writeBytes(bytes)
                thumbFile(code, slot).delete() // era do estado antigo
                imported++
                Log.i(TAG, "estado importado da pasta (saves/$name, ${bytes.size} bytes)")
            } catch (e: Exception) {
                Log.w(TAG, "falha ao importar saves/$name: $e")
            }
        }
        if (imported > 0) toast(l10n.resources.getQuantityString(R.plurals.states_imported, imported, imported))
    }

    /**
     * Lê do perfil do Hunter (thread GL) o nome do jogo, os alvos com o grupo de
     * cada um e a ajuda "Como usar" — nomes e ajuda no idioma do app.
     */
    private fun loadHuntInfo() {
        if (!huntSupported) {
            huntGameName = ""
            huntTargets = emptyArray()
            huntKinds = IntArray(0)
            huntHelps = emptyArray()
            return
        }
        val lang = L10n.locale(l10n).language
        huntGameName = NativeBridge.huntGameName(handle)
        val n = NativeBridge.huntTargetCount(handle)
        huntTargets = Array(n) { NativeBridge.huntTargetName(handle, it, lang) }
        huntKinds = IntArray(n) { NativeBridge.huntTargetKind(handle, it) }
        huntHelps = Array(n) {
            HuntHelp(
                NativeBridge.huntHelpTitle(handle, it, lang),
                NativeBridge.huntHelpBody(handle, it, lang),
                NativeBridge.huntHelpVideo(handle, it),
            )
        }
        Log.i(TAG, "shiny hunter: $huntGameName, $n alvos ($lang)")
    }

    /** Copia as stats da caça (Hunter) pros campos voláteis lidos pela UI. */
    private fun publishHuntStats() {
        statAttempts = NativeBridge.huntAttempts(handle)
        statSpecies = NativeBridge.huntLastSpecies(handle)
        statBestSV = NativeBridge.huntBestShinyValue(handle)
        // Selvagem "qualquer": o sprite acompanha a espécie do último encontro.
        if (statSpecies != spriteSpecies) refreshHuntSprites()
    }

    /**
     * Decodifica os sprites (normal + shiny) do painel na thread GL (dona do
     * ponteiro). Alvo de espécie fixa → a própria espécie; selvagem "qualquer" →
     * a do último encontro (vazio até o 1º encontro), como no desktop.
     */
    private fun refreshHuntSprites() {
        spriteSpecies = NativeBridge.huntLastSpecies(handle)
        spriteNormal = NativeBridge.huntTargetSprite(handle, huntTarget, false)
            .takeIf { it.isNotEmpty() }
        spriteShiny = NativeBridge.huntTargetSprite(handle, huntTarget, true)
            .takeIf { it.isNotEmpty() }
    }

    /** Chamado na thread GL quando a caça acha o shiny: para e avisa a UI. */
    private fun onHuntFinished() {
        hunting = false
        val n = statAttempts
        val name = currentTargetName
        runOnUiThread {
            stopHuntUI()
            huntViews.showFound(name, count(n))
        }
    }

    /**
     * Chamado na thread GL quando um presente esbarra no time cheio: o presente
     * iria pro PC (invisível pra caça). Para e explica o que fazer.
     */
    private fun onHuntPartyFull() {
        hunting = false
        val name = currentTargetName
        runOnUiThread {
            stopHuntUI()
            pauseForMenu()
            hunterOpen = true
            PartyFullScreen(this, this, name).show()
        }
    }

    /** Prêmio do cassino sem moedas: a caça parou; explica quanto falta. */
    private fun onHuntNoCoins() {
        hunting = false
        val name = currentTargetName
        val price = NativeBridge.huntPrice(handle, currentTargetIndex)
        val coins = NativeBridge.huntCoins(handle).coerceAtLeast(0)
        runOnUiThread {
            stopHuntUI()
            AlertDialog.Builder(this)
                .setTitle(tr(R.string.hunt_no_coins_title))
                .setMessage(tr(R.string.hunt_no_coins_msg, name, price, coins))
                .setPositiveButton(tr(R.string.ok), null)
                .show()
        }
    }

    /** Grava o `.sav` em disco se houve alteração desde a última gravação. */
    private fun flushBackup() {
        val code = currentGameCode ?: return
        if (handle == 0L || !NativeBridge.backupDirty(handle)) return
        val bytes = NativeBridge.saveBackup(handle)
        try {
            savFile(code).writeBytes(bytes)
            NativeBridge.clearBackupDirty(handle)
            Log.i(TAG, "save .sav gravado ($code)")
        } catch (e: Exception) {
            Log.e(TAG, "falha ao gravar .sav: $e")
            return // sem o save de trabalho, não adianta espelhar
        }
        // Cópia visível pra backup/portabilidade (best-effort).
        mirrorSav(code, bytes)
    }

    private fun doSaveState(slot: Int) {
        val code = currentGameCode ?: return
        val bytes = NativeBridge.saveState(handle)
        try {
            stateFile(code, slot).writeBytes(bytes)
            toast(tr(R.string.state_saved, slot + 1))
        } catch (e: Exception) {
            Log.e(TAG, "falha ao salvar estado: $e")
            toast(tr(R.string.state_save_failed))
            return // sem o estado de trabalho, não adianta espelhar
        }
        saveThumb(code, slot)
        // Cópia visível pra backup/portabilidade (best-effort), como o .sav.
        mirrorState(code, slot, bytes)
    }

    /**
     * Grava a miniatura do estado: copia o quadro exibido (thread GL, sem avançar
     * a emulação), e o encode PNG roda numa thread à parte. Com upscale ligado o
     * quadro vem ampliado e é reduzido de volta a 240×160. Best-effort.
     */
    private fun saveThumb(code: String, slot: Int) {
        val f = upscaleFactor
        val buf = ByteBuffer.allocateDirect(W * f * H * f * 4).order(ByteOrder.nativeOrder())
        NativeBridge.copyFramebuffer(handle, buf)
        buf.rewind()
        val px = ByteArray(buf.capacity())
        buf.get(px)
        val out = thumbFile(code, slot)
        Thread {
            try {
                val bmp = Bitmap.createBitmap(W * f, H * f, Bitmap.Config.ARGB_8888)
                bmp.copyPixelsFromBuffer(ByteBuffer.wrap(px))
                val small = if (f == 1) bmp else Bitmap.createScaledBitmap(bmp, W, H, false)
                out.outputStream().use { small.compress(Bitmap.CompressFormat.PNG, 100, it) }
                if (small !== bmp) small.recycle()
                bmp.recycle()
            } catch (e: Exception) {
                Log.w(TAG, "miniatura do estado falhou: $e")
            }
        }.start()
    }

    private fun doLoadState(bytes: ByteArray) {
        if (NativeBridge.loadState(handle, bytes)) toast(tr(R.string.state_loaded))
        else toast(tr(R.string.state_load_failed))
    }

    /** Cria o AudioTrack 32768 Hz estéreo PCM16 em streaming, ou null se falhar. */
    private fun createAudioTrack(): AudioTrack? = try {
        val rate = 32768
        val channel = AudioFormat.CHANNEL_OUT_STEREO
        val encoding = AudioFormat.ENCODING_PCM_16BIT
        val minBuf = AudioTrack.getMinBufferSize(rate, channel, encoding)
        val bufSize = maxOf(minBuf, 8192)
        AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_GAME)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                    .build(),
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setSampleRate(rate)
                    .setChannelMask(channel)
                    .setEncoding(encoding)
                    .build(),
            )
            .setBufferSizeInBytes(bufSize)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .build()
            .also { it.play() }
    } catch (e: Exception) {
        Log.e(TAG, "AudioTrack indisponível: $e")
        null
    }

    /** Um passe compilado de um efeito multipass + como sua saída é amostrada. */
    private class MpPass(
        val program: Int,
        val posLoc: Int,
        val texLoc: Int,
        val origLoc: Int,
        val prevLoc: Int,
        val inputLoc: Int,
        val outputLoc: Int,
        val origSizeLoc: Int,
        val prevSizeLoc: Int,
        val frameLoc: Int,
        val scale: Int,
        val filter: Int,
        val floatFb: Boolean,
        val prev: Int,
    )

    /** Espec de um passe lida do manifesto (`frag`, `scale`, `filter`, `float`, `prev`). */
    private class MpSpec(
        val frag: String,
        val scale: Int,
        val filter: Int,
        val floatFb: Boolean,
        val prev: Int,
    )

    /** Textura intermediária (destino de um passe não-final): FBO + textura + tamanho. */
    private class MpTarget(val fbo: Int, val tex: Int, val w: Int, val h: Int)

    /** Efeito multipass: N passes + (N-1) alvos intermediários (o último vai à tela). */
    private class Multipass(val passes: List<MpPass>) {
        var targets: List<MpTarget> = emptyList()
        var srcW = -1
        var srcH = -1
    }

    /**
     * Renderer GL: roda o loop de emulação em `onDrawFrame` (thread GL). Envia o
     * framebuffer do core como textura 240×160 (`GL_NEAREST`, pixel art nítido) e
     * desenha um quad em tela cheia; a `glViewport` faz o letterbox preservando o
     * aspecto 240:160. Os recursos GL são recriados a cada `onSurfaceCreated` (o
     * contexto pode ser perdido no pause); o ponteiro do [Gba][NativeBridge] não.
     */
    private inner class GbaRenderer : GLSurfaceView.Renderer {
        private lateinit var buf: ByteBuffer
        private lateinit var quad: FloatBuffer
        private lateinit var quadFbo: FloatBuffer
        private var program = 0
        private var texId = 0
        private var posLoc = 0
        private var texLoc = 0
        // Locations dos uniforms do contrato (ver assets/shaders/README.md).
        // -1 quando o efeito não usa o uniform (glUniform vira no-op, sem erro).
        private var inputLoc = -1
        private var outputLoc = -1
        private var frameLoc = -1
        // Retângulo do letterbox (integer scaling), calculado em onSurfaceChanged.
        private var vw = 0
        private var vh = 0
        // Tamanho da surface (pra restaurar o viewport de letterbox após os passes).
        private var surfaceW = 0
        private var surfaceH = 0
        // Efeito multipass ativo (não-nulo quando currentShader é chave de MULTIPASS).
        private var multipass: Multipass? = null
        // Fator de upscale com que a textura está alocada. Quando difere de
        // `upscaleFactor`, a textura é realocada com as dimensões novas.
        private var texFactor = 1
        private var frames = 0
        private val audioBuf = ShortArray(8192)
        private val frameNanos = (1_000_000_000.0 / 59.7275).toLong()
        // Custo médio (EMA) de um frame do fast-forward sem tela e do frame
        // visível (emulação + upscale + cópia), pro orçamento de tempo do draw.
        private var skipCostNanos = 0L
        private var visibleCostNanos = 0L

        /** Média móvel exponencial (peso 1/8 pra amostra nova). */
        private fun ema(avg: Long, sample: Long): Long =
            if (avg == 0L) sample else avg + (sample - avg) / 8

        override fun onSurfaceCreated(gl: GL10?, config: EGLConfig?) {
            if (handle == 0L) handle = NativeBridge.create()
            if (audio == null) {
                audio = createAudioTrack()
                applyAudioGain()
                if (menuPaused) audio?.pause()
            }
            // Dimensionado pro fator máximo (HQ4x): o nativo escreve o frame já
            // ampliado aqui; a textura acompanha o fator atual.
            buf = ByteBuffer.allocateDirect(W * H * 4 * MAX_UPSCALE * MAX_UPSCALE)
                .order(ByteOrder.nativeOrder())
            NativeBridge.setUpscale(handle, currentUpscale)

            // Triangle strip: pos.xy + tex.uv por vértice. tex v=0 no topo casa
            // com a linha 0 do framebuffer (topo da imagem GBA).
            val verts = floatArrayOf(
                -1f, 1f, 0f, 0f, // sup-esq
                -1f, -1f, 0f, 1f, // inf-esq
                1f, 1f, 1f, 0f, // sup-dir
                1f, -1f, 1f, 1f, // inf-dir
            )
            quad = ByteBuffer.allocateDirect(verts.size * 4)
                .order(ByteOrder.nativeOrder())
                .asFloatBuffer()
                .apply { put(verts); position(0) }
            // Quad dos passes intermediários do multipass (render em FBO): v=0 em
            // y=-1. A linha 0 de um FBO fica em y=-1, então a linha 0 da imagem cai
            // na linha 0 da textura — mesma orientação da fonte. Com o quad da tela
            // cada FBO invertia a imagem (BLUR saía de cabeça pra baixo; no ScaleFX
            // as texturas de passes diferentes ficavam desalinhadas).
            val vertsFbo = floatArrayOf(
                -1f, 1f, 0f, 1f,
                -1f, -1f, 0f, 0f,
                1f, 1f, 1f, 1f,
                1f, -1f, 1f, 0f,
            )
            quadFbo = ByteBuffer.allocateDirect(vertsFbo.size * 4)
                .order(ByteOrder.nativeOrder())
                .asFloatBuffer()
                .apply { put(vertsFbo); position(0) }

            // Sonda o suporte a framebuffer meio-float (gate do ScaleFX & cia): precisa
            // criar a textura RGBA16F (OES_texture_half_float) E renderizar nela
            // (EXT_color_buffer_half_float). GLES2 não garante nenhum dos dois.
            val ext = GLES20.glGetString(GLES20.GL_EXTENSIONS).orEmpty()
            halfFloatSupported = ext.contains("GL_OES_texture_half_float") &&
                ext.contains("GL_EXT_color_buffer_half_float")
            Log.i(TAG, "meio-float (RGBA16F FBO): ${if (halfFloatSupported) "ok" else "ausente"}")

            // Contexto GL novo (a rotação re-parenta a GLSurfaceView, o que recria
            // o contexto): os nomes GL guardados são do contexto morto. Esquece-os
            // sem deletar — deletar aqui poderia apagar objetos novos que recebam
            // o mesmo nome.
            program = 0
            multipass = null
            rebuildProgram()
            texId = createTexture()
            // createTexture aloca 240×160 (fator 1). Sem isto o texFactor antigo
            // (ex.: 4 com HQ4x) sobrevivia e o upload ampliado caía num
            // glTexSubImage2D maior que a textura → tela preta até trocar o filtro.
            texFactor = 1
            GLES20.glClearColor(0f, 0f, 0f, 1f)
        }

        override fun onSurfaceChanged(gl: GL10?, width: Int, height: Int) {
            // Integer scaling: maior múltiplo inteiro de 240×160 que cabe,
            // centralizado (letterbox). Mantém os pixels nítidos e o aspecto 3:2.
            val s = maxOf(1, minOf(width / W, height / H))
            vw = W * s
            vh = H * s
            surfaceW = width
            surfaceH = height
            GLES20.glViewport((width - vw) / 2, (height - vh) / 2, vw, vh)

            // Sinal preciso de cadência: avisa o compositor que esta surface
            // produz ~59,73 fps fixos. FIXED_SOURCE = "não dá pra reamostrar, casa
            // o display com isso". Surface válida aqui (estamos no callback do GL).
            if (Build.VERSION.SDK_INT >= 30) {
                glView.holder.surface?.setFrameRate(
                    GBA_FPS, Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE
                )
                logRefresh("onSurfaceChanged")
            }
        }

        override fun onDrawFrame(gl: GL10?) {
            val start = System.nanoTime()

            pendingRom.getAndSet(null)?.let {
                // Grava o save do jogo anterior antes de trocar de cartucho.
                flushBackup()
                NativeBridge.loadRom(handle, it)
                romLoaded = true
                onRomLoaded()
            }

            // Comandos da caça (menu).
            val startTarget = pendingHuntStart.getAndSet(-1)
            if (startTarget >= 0 && romLoaded && NativeBridge.huntStart(handle, startTarget)) {
                huntTarget = startTarget
                refreshHuntSprites()
                hunting = true
                runOnUiThread { startHuntUI() }
            }
            if (pendingHuntStop.getAndSet(false)) {
                NativeBridge.huntStop(handle)
                hunting = false
                runOnUiThread { stopHuntUI() }
            }

            // Comandos do link (menu). Dá pra conectar mesmo sem ROM; a troca em
            // si só anda com o jogo rodando (o protocolo corre no renderFrame).
            val linkHostPort = pendingLinkHost.getAndSet(-1)
            if (linkHostPort >= 0) NativeBridge.linkStartHost(handle, linkHostPort)
            pendingLinkJoin.getAndSet(null)?.let { NativeBridge.linkStartJoin(handle, it) }
            if (pendingLinkCancel.getAndSet(false)) NativeBridge.linkCancel(handle)
            if (pendingLinkDisconnect.getAndSet(false)) NativeBridge.linkDisconnect(handle)
            // Publica o estado (linkStatus também recolhe a sessão pronta) e avisa
            // nas transições — o menu é estático, então o toast dá o retorno.
            val ls = NativeBridge.linkStatus(handle)
            linkStatePub.set(ls)
            linkRolePub.set(NativeBridge.linkRole(handle))
            if (ls != lastLinkState) {
                val prev = lastLinkState
                lastLinkState = ls
                runOnUiThread {
                    when {
                        ls == 2 -> toast(tr(R.string.link_connected_toast))
                        ls == 0 && prev == 2 -> toast(tr(R.string.link_ended))
                    }
                    // Segura o Wi-Fi em baixa latência enquanto há link (conectando
                    // ou conectado); solta ao voltar pro solo.
                    if (ls == 0) releaseWifiLock() else acquireWifiLock()
                }
            }
            // Mostra a causa de uma falha de conexão (ex.: permissão, porta ocupada).
            val linkErr = NativeBridge.linkTakeError(handle)
            if (linkErr.isNotEmpty()) runOnUiThread { toast(tr(R.string.link_failed, linkErr)) }

            // Save states só fora da caça (o menu durante o hunt nem os oferece).
            if (!hunting) {
                val saveSlot = pendingSaveState.getAndSet(-1)
                if (romLoaded && saveSlot >= 0) doSaveState(saveSlot)
                pendingLoadState.getAndSet(null)?.let { if (romLoaded) doLoadState(it) }
            }

            // Menu/Configurações abertos: não emula (a textura guarda o último
            // frame); a caça segue rodando, ela tem a tela dela.
            val frozen = menuPaused && !hunting
            if (romLoaded && !frozen) {
                if (hunting) {
                    // O Hunter dirige os inputs e roda os frames por dentro; só
                    // copiamos o framebuffer resultante (sem avançar de novo).
                    val step = NativeBridge.huntStep(handle, HUNT_BATCH)
                    buf.clear()
                    NativeBridge.copyFramebuffer(handle, buf)
                    buf.rewind()
                    publishHuntStats()
                    when (step) {
                        NativeBridge.HUNT_SHINY -> onHuntFinished()
                        NativeBridge.HUNT_PARTY_FULL -> onHuntPartyFull()
                        NativeBridge.HUNT_NOT_ENOUGH_COINS -> onHuntNoCoins()
                    }
                } else {
                    val speed = ffSpeed
                    NativeBridge.setButtons(handle, buttonMask.get())
                    // Fast-forward: roda os frames extras ANTES do frame visível,
                    // descartando o áudio deles. O pacing lá embaixo segue ancorado
                    // no write bloqueante do áudio do último frame (1 frame de
                    // áudio por draw), com o som virando "fita acelerada".
                    //
                    // Os extras usam `runFrame` (sem upscale nem cópia: ninguém os
                    // vê) e respeitam um orçamento de tempo: só começa outro se ele
                    // + o frame visível ainda cabem em FF_BUDGET_NANOS. Com o
                    // aparelho quente (throttling) a velocidade cai um pouco em vez
                    // de o draw estourar o vsync e o áudio secar (travadas).
                    var ran = 0
                    while (ran < speed - 1 &&
                        System.nanoTime() - start + skipCostNanos + visibleCostNanos <= FF_BUDGET_NANOS
                    ) {
                        val t0 = System.nanoTime()
                        NativeBridge.runFrame(handle)
                        while (NativeBridge.drainAudio(handle, audioBuf) > 0) { /* descarta */ }
                        skipCostNanos = ema(skipCostNanos, System.nanoTime() - t0)
                        ran++
                    }
                    val t0 = System.nanoTime()
                    buf.clear()
                    NativeBridge.renderFrame(handle, buf)
                    buf.rewind()
                    visibleCostNanos = ema(visibleCostNanos, System.nanoTime() - t0)
                    ran++
                    frames += ran
                    // "Cruzou um múltiplo": com ran > 1 o == 0 podia nunca bater.
                    if (frames % FLUSH_EVERY_FRAMES < ran) flushBackup()
                }
                // Dimensões do frame conforme o fator de upscale (nativo já
                // entregou o buffer ampliado). Realoca a textura quando muda.
                val f = upscaleFactor
                val ow = W * f
                val oh = H * f
                GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, texId)
                if (f != texFactor) {
                    GLES20.glTexImage2D(
                        GLES20.GL_TEXTURE_2D, 0, GLES20.GL_RGBA, ow, oh, 0,
                        GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, buf,
                    )
                    texFactor = f
                } else {
                    GLES20.glTexSubImage2D(
                        GLES20.GL_TEXTURE_2D, 0, 0, 0, ow, oh,
                        GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, buf,
                    )
                }
                // Captura: copia o frame atual (já ampliado) e delega o encode.
                if (pendingScreenshot.getAndSet(false)) {
                    val px = ByteArray(ow * oh * 4)
                    buf.rewind()
                    buf.get(px)
                    saveScreenshot(px, ow, oh)
                }
            }

            GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
            if (romLoaded) {
                val mp = multipass
                if (mp != null) {
                    // Multipass: fonte = textura já carregada (240·f × 160·f).
                    drawMultipass(mp, texId, W * texFactor, H * texFactor)
                } else {
                    GLES20.glUseProgram(program)
                    // Uniforms do contrato (no-op se o efeito não os declara). O input
                    // é o tamanho da textura, que cresce com o fator de upscale.
                    GLES20.glUniform2f(inputLoc, (W * texFactor).toFloat(), (H * texFactor).toFloat())
                    GLES20.glUniform2f(outputLoc, vw.toFloat(), vh.toFloat())
                    GLES20.glUniform1i(frameLoc, frames)
                    quad.position(0)
                    GLES20.glVertexAttribPointer(posLoc, 2, GLES20.GL_FLOAT, false, 16, quad)
                    GLES20.glEnableVertexAttribArray(posLoc)
                    quad.position(2)
                    GLES20.glVertexAttribPointer(texLoc, 2, GLES20.GL_FLOAT, false, 16, quad)
                    GLES20.glEnableVertexAttribArray(texLoc)
                    GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
                    GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, texId)
                    GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)
                }
            }

            // Pacing: durante a caça, sem áudio e sem sleep — o vsync do
            // eglSwapBuffers paceia os draws e cada draw roda HUNT_BATCH frames
            // (~8×). No jogo normal, o `write` bloqueante do áudio ancora ao tempo
            // real; sem áudio/ROM, cai pro relógio.
            val a = audio
            if (hunting) {
                // sem pacing extra
            } else if (romLoaded && a != null && !frozen) {
                val n = NativeBridge.drainAudio(handle, audioBuf)
                if (n > 0) a.write(audioBuf, 0, n) else Thread.sleep(2)
            } else {
                val sleep = frameNanos - (System.nanoTime() - start)
                if (sleep > 0) Thread.sleep(sleep / 1_000_000, (sleep % 1_000_000).toInt())
            }
        }

        private fun createTexture(): Int {
            val ids = IntArray(1)
            GLES20.glGenTextures(1, ids, 0)
            GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, ids[0])
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_NEAREST)
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_NEAREST)
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)
            GLES20.glTexImage2D(
                GLES20.GL_TEXTURE_2D, 0, GLES20.GL_RGBA, W, H, 0,
                GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, null,
            )
            return ids[0]
        }

        /** (Re)compila o efeito atual (single-pass ou multipass) e recaptura locations. */
        fun rebuildProgram() {
            releaseMultipass()
            // Efeito multipass: compila a cadeia e para por aqui (o single-pass
            // `program` fica ocioso). Se falhar, cai no passthrough single-pass.
            if (MULTIPASS.any { it.first == currentShader }) {
                multipass = buildMultipass(currentShader)
                if (multipass != null) return
            }
            if (program != 0) GLES20.glDeleteProgram(program)
            program = buildProgram(currentShader)
            posLoc = GLES20.glGetAttribLocation(program, "aPos")
            texLoc = GLES20.glGetAttribLocation(program, "aTex")
            inputLoc = GLES20.glGetUniformLocation(program, "uInputSize")
            outputLoc = GLES20.glGetUniformLocation(program, "uOutputSize")
            frameLoc = GLES20.glGetUniformLocation(program, "uFrameCount")
        }

        /**
         * Corpo do efeito de `<key>.frag` nos assets (a pasta canônica
         * `assets/shaders/` da raiz do repo é registrada como assets srcDir no
         * build.gradle.kts, então os `.frag` ficam na raiz dos assets do APK).
         * Cai no passthrough se falhar.
         */
        private fun loadShaderBody(key: String): String {
            // Shader importado: usa a fonte guardada (cai no passthrough se ausente).
            if (key == "custom") {
                return customShaderSrc ?: "vec4 effect(vec2 uv) { return SAMPLE(uTex, uv); }"
            }
            return try {
                assets.open("$key.frag").bufferedReader().use { it.readText() }
            } catch (e: Exception) {
                Log.e(TAG, "shader '$key' não encontrado: $e")
                "vec4 effect(vec2 uv) { return SAMPLE(uTex, uv); }"
            }
        }

        private fun buildProgram(shaderKey: String): Int =
            buildProgramFromBody(loadShaderBody(shaderKey))

        /** Compila VS + (preâmbulo GLES + corpo `effect` + main) e devolve o programa. */
        private fun buildProgramFromBody(body: String): Int {
            val vs = compileShader(
                GLES20.GL_VERTEX_SHADER,
                """
                attribute vec2 aPos;
                attribute vec2 aTex;
                varying vec2 vTex;
                void main() {
                    vTex = aTex;
                    gl_Position = vec4(aPos, 0.0, 1.0);
                }
                """.trimIndent(),
            )
            // Preâmbulo GLES (contrato em assets/shaders/README.md) + corpo + main.
            // highp onde disponível: ScaleFX empacota inteiros em canais e decodifica
            // com floor(x*K+.5); mediump (10 bits de mantissa) erra em K grande.
            // uOrigTex/uPrevTex são a extensão multipass da ABI (ver README): a
            // textura original e a saída de um passe anterior declarado (`prev = N`).
            val fsHeader = """
                #ifdef GL_FRAGMENT_PRECISION_HIGH
                precision highp float;
                #else
                precision mediump float;
                #endif
                uniform sampler2D uTex;
                uniform sampler2D uOrigTex;
                uniform sampler2D uPrevTex;
                uniform vec2 uInputSize;
                uniform vec2 uOutputSize;
                uniform vec2 uOrigSize;
                uniform vec2 uPrevSize;
                uniform int uFrameCount;
                varying vec2 vTex;
                #define SAMPLE texture2D
            """.trimIndent()
            val fsFooter = "\nvoid main() { gl_FragColor = effect(vTex); }\n"
            val fs = compileShader(
                GLES20.GL_FRAGMENT_SHADER,
                "$fsHeader\n$body$fsFooter",
            )
            val prog = GLES20.glCreateProgram()
            GLES20.glAttachShader(prog, vs)
            GLES20.glAttachShader(prog, fs)
            GLES20.glLinkProgram(prog)
            val status = IntArray(1)
            GLES20.glGetProgramiv(prog, GLES20.GL_LINK_STATUS, status, 0)
            if (status[0] == 0) {
                Log.e(TAG, "link do programa GL falhou: ${GLES20.glGetProgramInfoLog(prog)}")
            }
            GLES20.glDeleteShader(vs)
            GLES20.glDeleteShader(fs)
            return prog
        }

        /** Lê o corpo `effect` de um `.frag` dos assets pelo nome exato ("blur-h.frag"). */
        private fun loadFragFile(name: String): String =
            try {
                assets.open(name).bufferedReader().use { it.readText() }
            } catch (e: Exception) {
                Log.e(TAG, "frag '$name' (multipass) não encontrado: $e")
                "vec4 effect(vec2 uv) { return SAMPLE(uTex, uv); }"
            }

        /** Parse do manifesto `.mpass` em passes ordenados (ver assets/shaders/README.md). */
        private fun parseManifest(src: String): List<MpSpec> {
            val out = ArrayList<Pair<Int, MpSpec>>()
            for (raw in src.lines()) {
                val line = raw.trim()
                if (line.isEmpty() || line.startsWith("#")) continue
                val eq = line.indexOf('=')
                if (eq < 0) continue
                // `passes = N` e outras chaves informativas não têm índice → ignoradas.
                val idx = line.substring(0, eq).trim().removePrefix("pass").toIntOrNull() ?: continue
                val parts = line.substring(eq + 1).split(';').map { it.trim() }
                val frag = parts.getOrNull(0).orEmpty()
                if (frag.isEmpty()) continue
                var scale = 1
                var filter = GLES20.GL_NEAREST
                var floatFb = false
                var prev = -1
                for (p in parts.drop(1)) {
                    val kv = p.split('=')
                    if (kv.size < 2) continue
                    when (kv[0].trim()) {
                        "scale" -> scale = kv[1].trim().toIntOrNull()?.coerceAtLeast(1) ?: 1
                        "filter" ->
                            filter = if (kv[1].trim() == "linear") GLES20.GL_LINEAR else GLES20.GL_NEAREST
                        "float" -> floatFb = kv[1].trim() == "true"
                        "prev" -> prev = kv[1].trim().toIntOrNull() ?: -1
                    }
                }
                out.add(idx to MpSpec(frag, scale, filter, floatFb, prev))
            }
            return out.sortedBy { it.first }.map { it.second }
        }

        /** Compila um efeito multipass a partir do manifesto `<key>.mpass`. */
        private fun buildMultipass(key: String): Multipass? {
            val manifest = try {
                assets.open("$key.mpass").bufferedReader().use { it.readText() }
            } catch (e: Exception) {
                Log.e(TAG, "mpass '$key' não encontrado: $e")
                return null
            }
            val specs = parseManifest(manifest)
            if (specs.isEmpty()) {
                Log.e(TAG, "mpass '$key' sem passes")
                return null
            }
            // ScaleFX & cia guardam dados em RGBA16F; sem a extensão o efeito não roda.
            if (specs.any { it.floatFb } && !halfFloatSupported) {
                Log.e(TAG, "mpass '$key' precisa de meio-float, indisponível neste device")
                return null
            }
            val passes = specs.map { s ->
                val prog = buildProgramFromBody(loadFragFile(s.frag))
                MpPass(
                    prog,
                    GLES20.glGetAttribLocation(prog, "aPos"),
                    GLES20.glGetAttribLocation(prog, "aTex"),
                    GLES20.glGetUniformLocation(prog, "uOrigTex"),
                    GLES20.glGetUniformLocation(prog, "uPrevTex"),
                    GLES20.glGetUniformLocation(prog, "uInputSize"),
                    GLES20.glGetUniformLocation(prog, "uOutputSize"),
                    GLES20.glGetUniformLocation(prog, "uOrigSize"),
                    GLES20.glGetUniformLocation(prog, "uPrevSize"),
                    GLES20.glGetUniformLocation(prog, "uFrameCount"),
                    s.scale,
                    s.filter,
                    s.floatFb,
                    s.prev,
                )
            }
            return Multipass(passes)
        }

        /**
         * Cria uma textura `w`×`h` (RGBA8 ou, se `floatFb`, RGBA meio-float via
         * `GL_HALF_FLOAT_OES`) + FBO. `null` se o FBO não ficar completo. Meio-float
         * só é pedido quando `halfFloatSupported` (checado antes de compilar o efeito).
         */
        private fun createTarget(w: Int, h: Int, filter: Int, floatFb: Boolean): MpTarget? {
            val tex = IntArray(1)
            GLES20.glGenTextures(1, tex, 0)
            GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, tex[0])
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MIN_FILTER, filter)
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_MAG_FILTER, filter)
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
            GLES20.glTexParameteri(GLES20.GL_TEXTURE_2D, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)
            // GLES2: internalformat == format (GL_RGBA, sem tamanho); o `type` decide a
            // precisão. GL_HALF_FLOAT_OES não está na classe GLES20 → constante crua.
            val type = if (floatFb) GL_HALF_FLOAT_OES else GLES20.GL_UNSIGNED_BYTE
            GLES20.glTexImage2D(
                GLES20.GL_TEXTURE_2D, 0, GLES20.GL_RGBA, w, h, 0,
                GLES20.GL_RGBA, type, null,
            )
            val fbo = IntArray(1)
            GLES20.glGenFramebuffers(1, fbo, 0)
            GLES20.glBindFramebuffer(GLES20.GL_FRAMEBUFFER, fbo[0])
            GLES20.glFramebufferTexture2D(
                GLES20.GL_FRAMEBUFFER, GLES20.GL_COLOR_ATTACHMENT0, GLES20.GL_TEXTURE_2D, tex[0], 0,
            )
            val complete =
                GLES20.glCheckFramebufferStatus(GLES20.GL_FRAMEBUFFER) == GLES20.GL_FRAMEBUFFER_COMPLETE
            GLES20.glBindFramebuffer(GLES20.GL_FRAMEBUFFER, 0)
            if (!complete) {
                Log.e(TAG, "FBO multipass incompleto ${w}x$h")
                GLES20.glDeleteFramebuffers(1, fbo, 0)
                GLES20.glDeleteTextures(1, tex, 0)
                return null
            }
            return MpTarget(fbo[0], tex[0], w, h)
        }

        /** (Re)aloca os N-1 alvos intermediários quando o tamanho da fonte muda. */
        private fun ensureTargets(mp: Multipass, srcW: Int, srcH: Int) {
            if (mp.srcW == srcW && mp.srcH == srcH && mp.targets.size == mp.passes.size - 1) return
            for (t in mp.targets) {
                GLES20.glDeleteFramebuffers(1, intArrayOf(t.fbo), 0)
                GLES20.glDeleteTextures(1, intArrayOf(t.tex), 0)
            }
            val list = ArrayList<MpTarget>()
            for (i in 0 until mp.passes.size - 1) {
                val s = mp.passes[i].scale
                val t = createTarget(srcW * s, srcH * s, mp.passes[i].filter, mp.passes[i].floatFb)
                    ?: break
                list.add(t)
            }
            mp.targets = list
            mp.srcW = srcW
            mp.srcH = srcH
        }

        /** Desenha um efeito multipass: passes 0..n-2 em FBOs, o último na tela. */
        private fun drawMultipass(mp: Multipass, srcTex: Int, srcW: Int, srcH: Int) {
            ensureTargets(mp, srcW, srcH)
            if (mp.targets.size != mp.passes.size - 1) return // falha ao alocar FBOs
            val n = mp.passes.size
            var inTex = srcTex
            var inW = srcW
            var inH = srcH
            // A textura original (fonte da cadeia) fica fixa na unit 1 o tempo todo.
            GLES20.glActiveTexture(GLES20.GL_TEXTURE1)
            GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, srcTex)
            for (i in 0 until n) {
                val isLast = i == n - 1
                val pass = mp.passes[i]
                val outW: Int
                val outH: Int
                if (isLast) {
                    GLES20.glBindFramebuffer(GLES20.GL_FRAMEBUFFER, 0)
                    GLES20.glViewport((surfaceW - vw) / 2, (surfaceH - vh) / 2, vw, vh)
                    outW = vw
                    outH = vh
                } else {
                    val t = mp.targets[i]
                    GLES20.glBindFramebuffer(GLES20.GL_FRAMEBUFFER, t.fbo)
                    GLES20.glViewport(0, 0, t.w, t.h)
                    outW = t.w
                    outH = t.h
                }
                GLES20.glUseProgram(pass.program)
                GLES20.glUniform1i(pass.origLoc, 1)
                GLES20.glUniform1i(pass.prevLoc, 2)
                GLES20.glUniform2f(pass.inputLoc, inW.toFloat(), inH.toFloat())
                GLES20.glUniform2f(pass.outputLoc, outW.toFloat(), outH.toFloat())
                GLES20.glUniform2f(pass.origSizeLoc, srcW.toFloat(), srcH.toFloat())
                // uPrevTex (unit 2): saída de um passe anterior declarado (`prev = N`).
                // Sem `prev` válido aponta pra fonte (inofensivo: só quem declara amostra).
                val prevT = mp.targets.getOrNull(pass.prev)
                val prevTex = prevT?.tex ?: srcTex
                GLES20.glUniform2f(
                    pass.prevSizeLoc,
                    (prevT?.w ?: srcW).toFloat(),
                    (prevT?.h ?: srcH).toFloat(),
                )
                GLES20.glActiveTexture(GLES20.GL_TEXTURE2)
                GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, prevTex)
                GLES20.glUniform1i(pass.frameLoc, frames)
                val q = if (isLast) quad else quadFbo
                q.position(0)
                GLES20.glVertexAttribPointer(pass.posLoc, 2, GLES20.GL_FLOAT, false, 16, q)
                GLES20.glEnableVertexAttribArray(pass.posLoc)
                q.position(2)
                GLES20.glVertexAttribPointer(pass.texLoc, 2, GLES20.GL_FLOAT, false, 16, q)
                GLES20.glEnableVertexAttribArray(pass.texLoc)
                GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
                GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, inTex)
                GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)
                if (!isLast) {
                    val t = mp.targets[i]
                    inTex = t.tex
                    inW = t.w
                    inH = t.h
                }
            }
            // Restaura a unit ativa padrão (o resto do renderer assume TEXTURE0).
            GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
        }

        /** Libera programas e FBOs do multipass atual (troca de shader / recriação). */
        private fun releaseMultipass() {
            val mp = multipass ?: return
            for (p in mp.passes) GLES20.glDeleteProgram(p.program)
            for (t in mp.targets) {
                GLES20.glDeleteFramebuffers(1, intArrayOf(t.fbo), 0)
                GLES20.glDeleteTextures(1, intArrayOf(t.tex), 0)
            }
            multipass = null
        }

        private fun compileShader(type: Int, src: String): Int {
            val shader = GLES20.glCreateShader(type)
            GLES20.glShaderSource(shader, src)
            GLES20.glCompileShader(shader)
            val status = IntArray(1)
            GLES20.glGetShaderiv(shader, GLES20.GL_COMPILE_STATUS, status, 0)
            if (status[0] == 0) {
                Log.e(TAG, "compilação de shader falhou: ${GLES20.glGetShaderInfoLog(shader)}")
            }
            return shader
        }
    }
}
