package com.auroragba

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Color
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.opengl.GLES20
import android.opengl.GLSurfaceView
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
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
class MainActivity : Activity() {

    private companion object {
        const val W = 240
        const val H = 160
        const val OPEN_ROM = 1
        const val TAG = "AuroraGBA"
        // Grava o .sav periodicamente quando há alteração (~5 s a 60 fps).
        const val FLUSH_EVERY_FRAMES = 300
        // Frames de emulação por draw durante a caça (acelera o hunt; ~8× a 60Hz).
        const val HUNT_BATCH = 8
        // Slots de save state em disco (`<code>.ss1`..`.ss8`), igual ao desktop.
        const val SAVE_SLOTS = 8
        val MATCH = FrameLayout.LayoutParams.MATCH_PARENT
    }

    private lateinit var glView: GLSurfaceView
    private val buttonMask = AtomicInteger(0)
    private val pendingRom = AtomicReference<ByteArray?>(null)

    // Comandos do menu, executados na thread GL (acesso ao ponteiro): salvar
    // estado no slot pedido (-1 = nenhum) e carregar estado (bytes lidos do
    // arquivo na UI; o slot já foi resolvido lá).
    private val pendingSaveState = AtomicInteger(-1)
    private val pendingLoadState = AtomicReference<ByteArray?>(null)

    // Game code do jogo atual (chave dos arquivos de save). Escrito na thread GL
    // ao carregar a ROM, lido na UI pra montar os caminhos.
    @Volatile private var currentGameCode: String? = null

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

    // Stats da caça publicadas pela thread GL a cada step; lidas pelo updater da UI.
    @Volatile private var statAttempts = 0L
    @Volatile private var statSpecies = 0
    @Volatile private var statBestSV = 0xFFFF
    @Volatile private var currentTargetName = ""

    private lateinit var huntStatus: TextView
    private val uiHandler = Handler(Looper.getMainLooper())

    // Sprites do alvo (ARGB8888 64×64), decodificados na thread GL ao iniciar a
    // caça e lidos pela UI ao montar o painel do retrato.
    @Volatile private var spriteNormal: IntArray? = null
    @Volatile private var spriteShiny: IntArray? = null

    // Views persistentes (reaproveitadas entre paisagem/retrato em [applyLayout]).
    private lateinit var controls: ControlsView
    private lateinit var hunterPanel: LinearLayout
    private lateinit var panelSpriteNormal: ImageView
    private lateinit var panelSpriteShiny: ImageView
    private lateinit var panelStats: TextView
    private var portrait = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        glView = GLSurfaceView(this).apply {
            setEGLContextClientVersion(2)
            preserveEGLContextOnPause = true
            setRenderer(GbaRenderer())
            renderMode = GLSurfaceView.RENDERMODE_CONTINUOUSLY
        }

        controls = ControlsView(this).apply {
            onMask = { buttonMask.set(it) }
            onMenu = { showMenu() }
        }

        // Overlay compacto de status da caça (paisagem): topo, abaixo do ☰.
        huntStatus = TextView(this).apply {
            setBackgroundColor(Color.argb(170, 0, 0, 0))
            setTextColor(Color.WHITE)
            textSize = 14f
            setPadding(28, 18, 28, 18)
            visibility = View.GONE
        }

        buildHunterPanel()

        // Monta o layout conforme a orientação atual (rebuild em rotação).
        applyLayout()

        if (savedInstanceState == null) openRomPicker()
    }

    /** Cria o painel completo do hunter (usado no retrato): sprites + stats + parar. */
    private fun buildHunterPanel() {
        val dp = resources.displayMetrics.density
        fun spriteView() = ImageView(this).apply {
            scaleType = ImageView.ScaleType.FIT_CENTER
            layoutParams = LinearLayout.LayoutParams((96 * dp).toInt(), (96 * dp).toInt())
        }
        panelSpriteNormal = spriteView()
        panelSpriteShiny = spriteView()

        val sprites = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER
            fun labeled(label: String, img: ImageView) = LinearLayout(this@MainActivity).apply {
                orientation = LinearLayout.VERTICAL
                gravity = Gravity.CENTER
                setPadding((16 * dp).toInt(), 0, (16 * dp).toInt(), 0)
                addView(img)
                addView(
                    TextView(this@MainActivity).apply {
                        text = label
                        setTextColor(Color.LTGRAY)
                        gravity = Gravity.CENTER
                    },
                )
            }
            addView(labeled("Normal", panelSpriteNormal))
            addView(labeled("✨ Shiny", panelSpriteShiny))
        }

        panelStats = TextView(this).apply {
            setTextColor(Color.WHITE)
            textSize = 15f
            gravity = Gravity.CENTER
            setPadding(0, (12 * dp).toInt(), 0, (12 * dp).toInt())
        }

        val stop = Button(this).apply {
            text = "⏹ Parar caça"
            setOnClickListener { pendingHuntStop.set(true) }
        }

        hunterPanel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setBackgroundColor(Color.rgb(18, 18, 22))
            setPadding(0, (16 * dp).toInt(), 0, (16 * dp).toInt())
            addView(sprites)
            addView(panelStats)
            addView(stop)
            visibility = View.GONE
        }
    }

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
        for (v in listOf<View>(glView, controls, huntStatus, hunterPanel)) {
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
            bottom.addView(hunterPanel, FrameLayout.LayoutParams(MATCH, MATCH))
            col.addView(bottom)
            root = col
        } else {
            val frame = FrameLayout(this)
            frame.addView(glView, FrameLayout.LayoutParams(MATCH, MATCH))
            frame.addView(controls, FrameLayout.LayoutParams(MATCH, MATCH))
            frame.addView(
                huntStatus,
                FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                ).apply {
                    gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
                    topMargin = 220
                },
            )
            root = frame
        }
        setContentView(root)
        refreshHuntViews()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        applyLayout()
    }

    /** Mostra/esconde controles, overlay e painel conforme caça × orientação. */
    private fun refreshHuntViews() {
        if (portrait) {
            huntStatus.visibility = View.GONE
            controls.visibility = if (hunting) View.GONE else View.VISIBLE
            hunterPanel.visibility = if (hunting) View.VISIBLE else View.GONE
            if (hunting) updateHunterPanel()
        } else {
            controls.visibility = View.VISIBLE
            hunterPanel.visibility = View.GONE
            huntStatus.visibility = if (hunting) View.VISIBLE else View.GONE
            if (hunting) huntStatus.text = huntStatusText()
        }
    }

    override fun onResume() {
        super.onResume()
        glView.onResume()
        audio?.play()
    }

    override fun onPause() {
        super.onPause()
        // Garante a gravação do .sav ao sair (a fila do GL é drenada na thread do
        // ponteiro); o flush periódico é a rede de segurança.
        glView.queueEvent { flushBackup() }
        audio?.pause()
        glView.onPause()
    }

    override fun onDestroy() {
        super.onDestroy()
        if (handle != 0L) {
            NativeBridge.destroy(handle)
            handle = 0L
        }
        audio?.release()
        audio = null
    }

    private fun openRomPicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(intent, OPEN_ROM)
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
                .setTitle("Shiny Hunter")
                .setItems(arrayOf("Parar caça")) { _, _ -> pendingHuntStop.set(true) }
                .show()
            return
        }
        val items = mutableListOf("Carregar ROM", "Salvar estado", "Carregar estado")
        if (huntSupported) items.add("✨ Shiny Hunter")
        AlertDialog.Builder(this)
            .setTitle("Menu")
            .setItems(items.toTypedArray()) { _, which ->
                when (items[which]) {
                    "Carregar ROM" -> openRomPicker()
                    "Salvar estado" -> saveStateFromMenu()
                    "Carregar estado" -> loadStateFromMenu()
                    "✨ Shiny Hunter" -> startShinyHunter()
                }
            }
            .show()
    }

    /**
     * Abre a escolha de alvo e dispara a caça. O jogo precisa estar **parado na
     * frente do alvo com o save carregado** (igual ao desktop). A caça em si roda
     * na thread GL via [pendingHuntStart].
     */
    private fun startShinyHunter() {
        if (!romLoaded || !huntSupported) {
            toast("Jogo não suportado pelo Shiny Hunter")
            return
        }
        val targets = huntTargets
        if (targets.isEmpty()) {
            toast("Sem alvos para este jogo")
            return
        }
        AlertDialog.Builder(this)
            .setTitle("Alvo — $huntGameName")
            .setItems(targets) { _, i ->
                currentTargetName = targets[i]
                statAttempts = 0
                statSpecies = 0
                statBestSV = 0xFFFF
                pendingHuntStart.set(i)
            }
            .show()
    }

    /** Texto do overlay de status da caça (lido das stats publicadas). */
    private fun huntStatusText(): String {
        val best = if (statBestSV == 0xFFFF) "—" else statBestSV.toString()
        return "✨ Caçando $currentTargetName\n" +
            "Tentativas: $statAttempts    melhor SV: $best\n" +
            "Espécie lida: $statSpecies    (☰ para parar)"
    }

    /** Texto detalhado do painel do retrato (sprites à parte). */
    private fun huntPanelText(): String {
        val best = if (statBestSV == 0xFFFF) "—" else statBestSV.toString()
        val prob = 1.0 - Math.pow(1.0 - 1.0 / 8192.0, statAttempts.toDouble())
        return "✨ Caçando $currentTargetName\n\n" +
            "Tentativas: $statAttempts\n" +
            "Melhor SV: $best   (shiny < 8)\n" +
            "Espécie lida: $statSpecies\n" +
            "Chance acumulada: ${"%.1f".format(prob * 100)}%"
    }

    /** Atualiza o painel do retrato (sprites + stats). */
    private fun updateHunterPanel() {
        spriteNormal?.let { panelSpriteNormal.setImageBitmap(spriteBitmap(it)) }
        spriteShiny?.let { panelSpriteShiny.setImageBitmap(spriteBitmap(it)) }
        panelStats.text = huntPanelText()
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
            if (portrait) updateHunterPanel() else huntStatus.text = huntStatusText()
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

    /** Arquivo do save state do `slot` (0-indexado: `.ss1`..`.ss8`), em `filesDir`. */
    private fun stateFile(code: String, slot: Int) = File(filesDir, "$code.ss${slot + 1}")

    /** Rótulo de um slot pro seletor: "Slot N — vazio" ou "Slot N — dd/MM HH:mm". */
    private fun slotLabel(code: String, slot: Int): String {
        val f = stateFile(code, slot)
        val when_ = if (f.exists()) {
            SimpleDateFormat("dd/MM HH:mm", Locale.getDefault()).format(Date(f.lastModified()))
        } else {
            "vazio"
        }
        return "Slot ${slot + 1} — $when_"
    }

    /** Seletor de slot pra salvar: todos os 8 (sobrescreve), com data dos ocupados. */
    private fun saveStateFromMenu() {
        val code = currentGameCode
        if (!romLoaded || code == null) {
            toast("Carregue uma ROM antes de salvar estado")
            return
        }
        val labels = Array(SAVE_SLOTS) { slotLabel(code, it) }
        AlertDialog.Builder(this)
            .setTitle("Salvar no slot")
            .setItems(labels) { _, slot -> pendingSaveState.set(slot) }
            .show()
    }

    /** Seletor de slot pra carregar: vazios ficam desabilitados. */
    private fun loadStateFromMenu() {
        val code = currentGameCode
        if (!romLoaded || code == null) {
            toast("Carregue uma ROM antes de carregar estado")
            return
        }
        val exists = BooleanArray(SAVE_SLOTS) { stateFile(code, it).exists() }
        if (exists.none { it }) {
            toast("Nenhum estado salvo")
            return
        }
        val labels = Array(SAVE_SLOTS) { slotLabel(code, it) }
        AlertDialog.Builder(this)
            .setTitle("Carregar do slot")
            .setItems(labels) { _, slot ->
                if (exists[slot]) {
                    pendingLoadState.set(stateFile(code, slot).readBytes())
                } else {
                    toast("Slot ${slot + 1} está vazio")
                }
            }
            .show()
    }

    private fun toast(msg: String) =
        runOnUiThread { Toast.makeText(this, msg, Toast.LENGTH_SHORT).show() }

    @Deprecated("API clássica; suficiente para este app de tela única")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == OPEN_ROM && resultCode == RESULT_OK) {
            val uri = data?.data ?: return
            val bytes = contentResolver.openInputStream(uri)?.use { it.readBytes() } ?: return
            pendingRom.set(bytes)
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
        if (huntSupported) {
            huntGameName = NativeBridge.huntGameName(handle)
            val n = NativeBridge.huntTargetCount(handle)
            huntTargets = Array(n) { NativeBridge.huntTargetName(handle, it) }
            Log.i(TAG, "shiny hunter: $huntGameName, ${n} alvos")
        } else {
            huntGameName = ""
            huntTargets = emptyArray()
        }
        runOnUiThread { stopHuntUI() }

        if (!NativeBridge.hasSave(handle)) return
        val f = savFile(code)
        if (f.exists()) {
            val ok = NativeBridge.loadBackup(handle, f.readBytes())
            Log.i(TAG, "save .sav carregado=$ok (${f.name})")
        } else {
            Log.i(TAG, "sem .sav prévio para $code")
        }
    }

    /** Copia as stats da caça (Hunter) pros campos voláteis lidos pela UI. */
    private fun publishHuntStats() {
        statAttempts = NativeBridge.huntAttempts(handle)
        statSpecies = NativeBridge.huntLastSpecies(handle)
        statBestSV = NativeBridge.huntBestShinyValue(handle)
    }

    /** Chamado na thread GL quando a caça acha o shiny: para e avisa a UI. */
    private fun onHuntFinished() {
        hunting = false
        val n = statAttempts
        runOnUiThread {
            stopHuntUI()
            Toast.makeText(
                this,
                "✨ Shiny encontrado em $n tentativas! Controle devolvido.",
                Toast.LENGTH_LONG,
            ).show()
        }
    }

    /** Grava o `.sav` em disco se houve alteração desde a última gravação. */
    private fun flushBackup() {
        val code = currentGameCode ?: return
        if (handle == 0L || !NativeBridge.backupDirty(handle)) return
        try {
            savFile(code).writeBytes(NativeBridge.saveBackup(handle))
            NativeBridge.clearBackupDirty(handle)
            Log.i(TAG, "save .sav gravado ($code)")
        } catch (e: Exception) {
            Log.e(TAG, "falha ao gravar .sav: $e")
        }
    }

    private fun doSaveState(slot: Int) {
        val code = currentGameCode ?: return
        try {
            stateFile(code, slot).writeBytes(NativeBridge.saveState(handle))
            toast("Estado salvo no slot ${slot + 1}")
        } catch (e: Exception) {
            Log.e(TAG, "falha ao salvar estado: $e")
            toast("Falha ao salvar estado")
        }
    }

    private fun doLoadState(bytes: ByteArray) {
        if (NativeBridge.loadState(handle, bytes)) toast("Estado carregado")
        else toast("Falha ao carregar estado")
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
        private var program = 0
        private var texId = 0
        private var posLoc = 0
        private var texLoc = 0
        private var frames = 0
        private val audioBuf = ShortArray(8192)
        private val frameNanos = (1_000_000_000.0 / 59.7275).toLong()

        override fun onSurfaceCreated(gl: GL10?, config: EGLConfig?) {
            if (handle == 0L) handle = NativeBridge.create()
            if (audio == null) audio = createAudioTrack()
            buf = ByteBuffer.allocateDirect(W * H * 4).order(ByteOrder.nativeOrder())

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

            program = buildProgram()
            posLoc = GLES20.glGetAttribLocation(program, "aPos")
            texLoc = GLES20.glGetAttribLocation(program, "aTex")
            texId = createTexture()
            GLES20.glClearColor(0f, 0f, 0f, 1f)
        }

        override fun onSurfaceChanged(gl: GL10?, width: Int, height: Int) {
            // Letterbox: maior retângulo 240:160 centralizado.
            val scale = minOf(width.toFloat() / W, height.toFloat() / H)
            val vw = (W * scale).toInt()
            val vh = (H * scale).toInt()
            GLES20.glViewport((width - vw) / 2, (height - vh) / 2, vw, vh)
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
                // Decodifica os sprites do alvo (thread do ponteiro) pro painel.
                spriteNormal = NativeBridge.huntTargetSprite(handle, startTarget, false)
                    .takeIf { it.isNotEmpty() }
                spriteShiny = NativeBridge.huntTargetSprite(handle, startTarget, true)
                    .takeIf { it.isNotEmpty() }
                hunting = true
                runOnUiThread { startHuntUI() }
            }
            if (pendingHuntStop.getAndSet(false)) {
                NativeBridge.huntStop(handle)
                hunting = false
                runOnUiThread { stopHuntUI() }
            }

            // Save states só fora da caça (o menu durante o hunt nem os oferece).
            if (!hunting) {
                val saveSlot = pendingSaveState.getAndSet(-1)
                if (romLoaded && saveSlot >= 0) doSaveState(saveSlot)
                pendingLoadState.getAndSet(null)?.let { if (romLoaded) doLoadState(it) }
            }

            if (romLoaded) {
                if (hunting) {
                    // O Hunter dirige os inputs e roda os frames por dentro; só
                    // copiamos o framebuffer resultante (sem avançar de novo).
                    val found = NativeBridge.huntStep(handle, HUNT_BATCH)
                    buf.clear()
                    NativeBridge.copyFramebuffer(handle, buf)
                    buf.rewind()
                    publishHuntStats()
                    if (found) onHuntFinished()
                } else {
                    NativeBridge.setButtons(handle, buttonMask.get())
                    buf.clear()
                    NativeBridge.renderFrame(handle, buf)
                    buf.rewind()
                    if (++frames % FLUSH_EVERY_FRAMES == 0) flushBackup()
                }
                GLES20.glBindTexture(GLES20.GL_TEXTURE_2D, texId)
                GLES20.glTexSubImage2D(
                    GLES20.GL_TEXTURE_2D, 0, 0, 0, W, H,
                    GLES20.GL_RGBA, GLES20.GL_UNSIGNED_BYTE, buf,
                )
            }

            GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
            if (romLoaded) {
                GLES20.glUseProgram(program)
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

            // Pacing: durante a caça, sem áudio e sem sleep — o vsync do
            // eglSwapBuffers paceia os draws e cada draw roda HUNT_BATCH frames
            // (~8×). No jogo normal, o `write` bloqueante do áudio ancora ao tempo
            // real; sem áudio/ROM, cai pro relógio.
            val a = audio
            if (hunting) {
                // sem pacing extra
            } else if (romLoaded && a != null) {
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

        private fun buildProgram(): Int {
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
            val fs = compileShader(
                GLES20.GL_FRAGMENT_SHADER,
                """
                precision mediump float;
                uniform sampler2D uTex;
                varying vec2 vTex;
                void main() {
                    gl_FragColor = texture2D(uTex, vTex);
                }
                """.trimIndent(),
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
