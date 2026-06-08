package com.auroragba

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.Bundle
import android.util.Log
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.Toast
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/**
 * Tela única do emulador: uma [SurfaceView] mostra o framebuffer e um
 * [ControlsView] por cima trata o toque. Uma thread dedicada roda o loop de
 * emulação+render; **todo** acesso ao ponteiro nativo do [Gba][NativeBridge]
 * acontece nessa thread. A UI passa input (máscara de botões) e a ROM escolhida
 * por estruturas atômicas.
 */
class MainActivity : Activity(), SurfaceHolder.Callback {

    private companion object {
        const val W = 240
        const val H = 160
        const val OPEN_ROM = 1
        const val TAG = "AuroraGBA"
        // Grava o .sav periodicamente quando há alteração (~5 s a 60 fps).
        const val FLUSH_EVERY_FRAMES = 300
        val MATCH = FrameLayout.LayoutParams.MATCH_PARENT
    }

    private lateinit var surface: SurfaceView
    private val buttonMask = AtomicInteger(0)
    private val pendingRom = AtomicReference<ByteArray?>(null)

    // Comandos do menu, executados na thread de emulação (acesso ao ponteiro):
    // salvar estado (flag) e carregar estado (bytes lidos do arquivo na UI).
    private val pendingSaveState = AtomicBoolean(false)
    private val pendingLoadState = AtomicReference<ByteArray?>(null)

    // Game code do jogo atual (chave dos arquivos de save). Escrito na thread de
    // emulação ao carregar a ROM, lido na UI pra montar os caminhos.
    @Volatile private var currentGameCode: String? = null

    // Ponteiro nativo: criado pela thread de render, liberado em onDestroy (vive
    // entre destruições da surface — pause/resume não reseta o jogo).
    @Volatile private var handle = 0L
    @Volatile private var running = false
    @Volatile private var romLoaded = false
    private var renderThread: Thread? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        val root = FrameLayout(this)
        surface = SurfaceView(this)
        surface.holder.addCallback(this)
        root.addView(surface, FrameLayout.LayoutParams(MATCH, MATCH))

        val controls = ControlsView(this)
        controls.onMask = { buttonMask.set(it) }
        controls.onMenu = { showMenu() }
        root.addView(controls, FrameLayout.LayoutParams(MATCH, MATCH))

        setContentView(root)

        if (savedInstanceState == null) openRomPicker()
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
     * Menu do emulador (botão ☰). Hoje: ROM e save state de slot único; é o ponto
     * de extensão pras próximas opções (mais slots, configurações, etc.).
     */
    private fun showMenu() {
        val items = arrayOf("Carregar ROM", "Salvar estado", "Carregar estado")
        AlertDialog.Builder(this)
            .setTitle("Menu")
            .setItems(items) { _, which ->
                when (which) {
                    0 -> openRomPicker()
                    1 -> saveStateFromMenu()
                    2 -> loadStateFromMenu()
                }
            }
            .show()
    }

    /** Arquivo `.sav` (backup do cartucho) do jogo, em `filesDir`. */
    private fun savFile(code: String) = File(filesDir, "$code.sav")

    /** Arquivo do save state (slot único `.ss1`) do jogo, em `filesDir`. */
    private fun stateFile(code: String) = File(filesDir, "$code.ss1")

    /** Pede pra thread de emulação salvar o estado no slot único. */
    private fun saveStateFromMenu() {
        if (!romLoaded || currentGameCode == null) {
            toast("Carregue uma ROM antes de salvar estado")
            return
        }
        pendingSaveState.set(true)
    }

    /** Lê o save state do disco (na UI) e entrega pra thread de emulação aplicar. */
    private fun loadStateFromMenu() {
        val code = currentGameCode
        if (!romLoaded || code == null) {
            toast("Carregue uma ROM antes de carregar estado")
            return
        }
        val f = stateFile(code)
        if (!f.exists()) {
            toast("Nenhum estado salvo")
            return
        }
        pendingLoadState.set(f.readBytes())
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

    override fun onDestroy() {
        super.onDestroy()
        stopRenderThread()
        if (handle != 0L) {
            NativeBridge.destroy(handle)
            handle = 0L
        }
    }

    // ── SurfaceHolder.Callback ───────────────────────────────────────────────
    override fun surfaceCreated(holder: SurfaceHolder) = startRenderThread()
    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {}
    override fun surfaceDestroyed(holder: SurfaceHolder) = stopRenderThread()

    private fun startRenderThread() {
        if (renderThread != null) return
        running = true
        renderThread = Thread(::renderLoop, "emu").also { it.start() }
    }

    private fun stopRenderThread() {
        running = false
        renderThread?.join()
        renderThread = null
    }

    private fun renderLoop() {
        if (handle == 0L) handle = NativeBridge.create()
        val buf = ByteBuffer.allocateDirect(W * H * 4).order(ByteOrder.nativeOrder())
        val bitmap = Bitmap.createBitmap(W, H, Bitmap.Config.ARGB_8888)
        val paint = Paint().apply { isFilterBitmap = false }
        val src = Rect(0, 0, W, H)
        val frameNanos = (1_000_000_000.0 / 59.7275).toLong()

        // Áudio: 32768 Hz estéreo (taxa nativa do APU, sem reamostragem). O
        // `write` bloqueante do AudioTrack ancora a emulação ao tempo real —
        // mesmo modelo do desktop (pacing pelo consumo de áudio). Se o áudio não
        // estiver disponível, caímos pro pacing por relógio (frameNanos).
        val audio = createAudioTrack()
        val audioBuf = ShortArray(8192)
        var frames = 0

        try {
            while (running) {
                val start = System.nanoTime()

                pendingRom.getAndSet(null)?.let {
                    // Grava o save do jogo anterior antes de trocar de cartucho.
                    flushBackup()
                    NativeBridge.loadRom(handle, it)
                    romLoaded = true
                    onRomLoaded()
                }
                // Comandos do menu (save states), na thread do ponteiro.
                if (romLoaded && pendingSaveState.getAndSet(false)) doSaveState()
                pendingLoadState.getAndSet(null)?.let { if (romLoaded) doLoadState(it) }

                if (romLoaded) {
                    NativeBridge.setButtons(handle, buttonMask.get())
                    buf.clear()
                    NativeBridge.renderFrame(handle, buf)
                    buf.rewind()
                    bitmap.copyPixelsFromBuffer(buf)
                    if (++frames % FLUSH_EVERY_FRAMES == 0) flushBackup()
                }

                val canvas = surface.holder.lockCanvas()
                if (canvas == null) {
                    Thread.sleep(8)
                    continue
                }
                try {
                    canvas.drawColor(Color.BLACK)
                    if (romLoaded) {
                        canvas.drawBitmap(bitmap, src, fitRect(canvas.width, canvas.height), paint)
                    }
                } finally {
                    surface.holder.unlockCanvasAndPost(canvas)
                }

                // Pacing: pelo áudio (write bloqueia até abrir espaço) ou, sem
                // áudio/ROM, pelo relógio.
                if (romLoaded && audio != null) {
                    val n = NativeBridge.drainAudio(handle, audioBuf)
                    if (n > 0) audio.write(audioBuf, 0, n) else Thread.sleep(2)
                } else {
                    val sleep = frameNanos - (System.nanoTime() - start)
                    if (sleep > 0) Thread.sleep(sleep / 1_000_000, (sleep % 1_000_000).toInt())
                }
            }
        } finally {
            // Garante a gravação do .sav ao sair (background/fechar app).
            flushBackup()
            audio?.run {
                pause()
                flush()
                release()
            }
        }
    }

    // ── Persistência de saves (chamadas na thread de emulação) ───────────────

    /** Após carregar a ROM: guarda o game code e carrega o `.sav` existente. */
    private fun onRomLoaded() {
        val code = NativeBridge.gameCode(handle)
        currentGameCode = code
        if (!NativeBridge.hasSave(handle)) return
        val f = savFile(code)
        if (f.exists()) {
            val ok = NativeBridge.loadBackup(handle, f.readBytes())
            Log.i(TAG, "save .sav carregado=$ok (${f.name})")
        } else {
            Log.i(TAG, "sem .sav prévio para $code")
        }
    }

    /** Grava o `.sav` em disco se houve alteração desde a última gravação. */
    private fun flushBackup() {
        val code = currentGameCode ?: return
        if (!NativeBridge.backupDirty(handle)) return
        try {
            savFile(code).writeBytes(NativeBridge.saveBackup(handle))
            NativeBridge.clearBackupDirty(handle)
            Log.i(TAG, "save .sav gravado ($code)")
        } catch (e: Exception) {
            Log.e(TAG, "falha ao gravar .sav: $e")
        }
    }

    private fun doSaveState() {
        val code = currentGameCode ?: return
        try {
            stateFile(code).writeBytes(NativeBridge.saveState(handle))
            toast("Estado salvo")
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
        Log.e("AuroraGBA", "AudioTrack indisponível: $e")
        null
    }

    /** Retângulo de destino preservando o aspecto 240:160, centralizado. */
    private fun fitRect(cw: Int, ch: Int): RectF {
        val scale = minOf(cw.toFloat() / W, ch.toFloat() / H)
        val w = W * scale
        val h = H * scale
        val left = (cw - w) / 2f
        val top = (ch - h) / 2f
        return RectF(left, top, left + w, top + h)
    }
}
